//! The pusher against a mock managed-identity endpoint and ingestion endpoint on loopback.

use std::convert::Infallible;
use std::sync::Mutex;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use innkeeper_session::SessionConfig;
use innkeeper_world::World;
use serde_json::json;
use tokio::net::TcpListener;

use super::*;

#[test]
fn a_connection_string_gives_the_key_and_the_ingestion_endpoint() {
    let (ikey, endpoint) = parse_connection_string(
        "InstrumentationKey=0123abcd-0123-4567-89ab-0123456789ab;\
         IngestionEndpoint=https://westeurope-5.in.applicationinsights.azure.com/;\
         LiveEndpoint=https://westeurope.livediagnostics.monitor.azure.com/",
    )
    .unwrap();
    assert_eq!(ikey, "0123abcd-0123-4567-89ab-0123456789ab");
    assert_eq!(
        endpoint.as_str(),
        "https://westeurope-5.in.applicationinsights.azure.com/"
    );
}

#[test]
fn a_bad_connection_string_is_refused() {
    let key = "InstrumentationKey=0123abcd-0123;";
    for text in [
        "",
        "IngestionEndpoint=https://x.in.applicationinsights.azure.com/",
        &format!("{key}IngestionEndpoint=http://x.in.applicationinsights.azure.com/"),
        &format!("{key}IngestionEndpoint=https://evil.example/"),
        &format!("{key}IngestionEndpoint=https://x.applicationinsights.azure.com.evil.example/"),
        "InstrumentationKey=not a guid;IngestionEndpoint=https://x.in.applicationinsights.azure.com/",
    ] {
        assert!(parse_connection_string(text).is_err(), "{text}");
    }
}

#[test]
fn failures_back_off_from_a_minute_to_ten() {
    let waits: Vec<u64> = (1..=6).map(|n| backoff(n).as_secs()).collect();
    assert_eq!(waits, [60, 120, 240, 480, 600, 600]);
    assert_eq!(backoff(u32::MAX).as_secs(), 600);
}

#[test]
fn a_warning_is_let_through_once_per_period() {
    let mut limiter = WarnLimiter::new(Duration::from_secs(600));
    let start = Instant::now();
    assert!(limiter.allow(start));
    assert!(!limiter.allow(start + Duration::from_secs(599)));
    assert!(limiter.allow(start + Duration::from_secs(600)));
}

#[derive(Default)]
struct Mock {
    tokens: Mutex<Vec<(String, bool)>>,
    tracks: Mutex<Vec<(Option<String>, Value)>>,
    track_status: Mutex<u16>,
}

async fn handle(mock: Arc<Mock>, request: Request<Incoming>) -> Response<Full<Bytes>> {
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or("").to_owned();
    let reply = |status: u16, body: Value| {
        let mut response = Response::new(Full::new(Bytes::from(body.to_string())));
        *response.status_mut() = StatusCode::from_u16(status).unwrap();
        response
    };
    if path == "/metadata/identity/oauth2/token" {
        let metadata = request
            .headers()
            .get("metadata")
            .is_some_and(|v| v == "true");
        let resource_ok = query.contains("resource=https%3A%2F%2Fmonitor.azure.com%2F");
        mock.tokens.lock().unwrap().push((query, metadata));
        return match resource_ok && metadata {
            true => reply(
                200,
                json!({"access_token": "mi-token", "expires_in": "3599"}),
            ),
            false => reply(400, json!({"error": "invalid_resource"})),
        };
    }
    if path == "/v2.1/track" {
        let auth = request
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let bytes = request.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        mock.tracks.lock().unwrap().push((auth, body));
        let status = *mock.track_status.lock().unwrap();
        return reply(
            status,
            json!({"itemsReceived": 1, "itemsAccepted": 1, "errors": []}),
        );
    }
    reply(404, json!({}))
}

async fn start_mock(status: u16) -> (Arc<Mock>, String) {
    let mock = Arc::new(Mock::default());
    *mock.track_status.lock().unwrap() = status;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let served = Arc::clone(&mock);
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let mock = Arc::clone(&served);
            tokio::spawn(async move {
                let service = service_fn(move |request| {
                    let mock = Arc::clone(&mock);
                    async move { Ok::<_, Infallible>(handle(mock, request).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    (mock, base)
}

fn settings(base: &str) -> Settings {
    let base = Url::parse(base).unwrap();
    Settings {
        credential: Credential::Managed { client_id: None },
        imds: base.clone(),
        authority: base.clone(),
        ikey: "0123abcd-0123".to_owned(),
        track: join(&base, "v2.1/track").unwrap(),
        interval: Duration::from_millis(60),
        items: None,
    }
}

fn connections() -> Arc<ConnectionSettings> {
    Arc::new(ConnectionSettings::new(
        SessionConfig::default(),
        World::stock(),
        None,
    ))
}

async fn wait_for(mock: &Mock, tracks: usize) {
    for _ in 0..200 {
        if mock.tracks.lock().unwrap().len() >= tracks {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("no push arrived");
}

#[tokio::test]
async fn it_pushes_heartbeat_and_gauges_with_the_identity_s_token() {
    let (mock, base) = start_mock(200).await;
    counters().message_in();
    let task = tokio::spawn(run(settings(&base), connections()));
    wait_for(&mock, 1).await;
    task.abort();
    let tracks = mock.tracks.lock().unwrap();
    let (auth, body) = &tracks[0];
    assert_eq!(auth.as_deref(), Some("Bearer mi-token"));
    let names: Vec<&str> = body
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            e["data"]["baseData"]["metrics"][0]["name"]
                .as_str()
                .unwrap()
        })
        .collect();
    for wanted in ["heartbeat", "connections", "players"] {
        assert!(names.contains(&wanted), "{names:?}");
    }
    let heartbeat = body
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["data"]["baseData"]["metrics"][0]["name"] == "heartbeat")
        .unwrap();
    assert_eq!(heartbeat["data"]["baseData"]["metrics"][0]["max"], 1.0);
    assert_eq!(heartbeat["iKey"], "0123abcd-0123");
    // No envelope names a person, an address or a property.
    let text = body.to_string();
    assert!(!text.contains("properties") && !text.contains("127.0.0.1"));
    let tokens = mock.tokens.lock().unwrap();
    assert_eq!(tokens.len(), 1, "the token is cached across pushes");
}

#[tokio::test]
async fn a_failing_endpoint_is_retried_only_after_the_back_off() {
    let (mock, base) = start_mock(500).await;
    let task = tokio::spawn(run(settings(&base), connections()));
    wait_for(&mock, 1).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    task.abort();
    assert_eq!(mock.tracks.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn an_unreachable_endpoint_does_not_stop_the_task_or_block() {
    let settings = settings("http://127.0.0.1:1");
    let task = tokio::spawn(run(settings, connections()));
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!task.is_finished());
    task.abort();
}

fn envelope_names(body: &Value) -> Vec<String> {
    let list = body.as_array().map(Vec::as_slice).unwrap_or_default();
    list.iter()
        .filter_map(|e| e["name"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
async fn a_traced_line_s_items_reach_the_endpoint_beside_the_metrics() {
    let (mock, base) = start_mock(200).await;
    let mut settings = settings(&base);
    let tracer = settings.tracer();
    let traceparent = "traceparent=00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
    let line = tracer.begin_line(Some(traceparent)).unwrap();
    let clock = crate::metrics::trace::Stopwatch::start();
    let call = crate::metrics::trace::Call::land(
        "Login",
        "Hub".to_owned(),
        clock,
        crate::metrics::trace::CallResult::Ok,
    );
    line.record(call);
    line.finish(crate::metrics::trace::LineEnd::HungUp);
    let task = tokio::spawn(run(settings, connections()));
    let mut found = None;
    for _ in 0..200 {
        found = mock.tracks.lock().unwrap().iter().find_map(|(auth, body)| {
            let names = envelope_names(body);
            names
                .iter()
                .any(|n| n.ends_with(".Request"))
                .then(|| (auth.clone(), body.clone()))
        });
        if found.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    task.abort();
    let (auth, body) = found.expect("the trace items were pushed");
    assert_eq!(auth.as_deref(), Some("Bearer mi-token"));
    let names = envelope_names(&body);
    assert_eq!(
        names,
        [
            "Microsoft.ApplicationInsights.RemoteDependency",
            "Microsoft.ApplicationInsights.Request"
        ]
    );
    let request = &body[1];
    let dependency = &body[0];
    assert_eq!(
        dependency["tags"]["ai.operation.parentId"],
        request["data"]["baseData"]["id"]
    );
    assert_eq!(request["tags"]["ai.operation.parentId"], "00f067aa0ba902b7");
    assert_eq!(request["data"]["baseData"]["responseCode"], "hung-up");
    assert!(!body.to_string().contains("127.0.0.1"));
}
