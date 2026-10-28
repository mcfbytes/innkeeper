//! The traceparent parser, the result codes, land naming, the volume rules and the line's items.

use innkeeper_world::LandType;

use super::*;

const OPERATION: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
const PARENT: &str = "00f067aa0ba902b7";
const VALID: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

#[test]
fn a_valid_traceparent_gives_the_operation_and_the_parent() {
    let parsed = TraceParent::parse(VALID).unwrap();
    assert_eq!(parsed.operation, OPERATION);
    assert_eq!(parsed.parent, PARENT);
    assert!(TraceParent::parse(&VALID.replace("-01", "-00")).is_some());
}

#[test]
fn an_invalid_traceparent_is_none() {
    let zeros = "0".repeat(32);
    for text in [
        "",
        "00",
        "01-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        "ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        "00-4BF92F3577B34DA6A3CE929D0E0E4736-00f067aa0ba902b7-01",
        "00-4bf92f3577b34da6a3ce929d0e0e473-00f067aa0ba902b7-01",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b-01",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-1",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-0g",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-extra",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-AA",
        &format!("00-{zeros}-00f067aa0ba902b7-01"),
    ] {
        assert!(TraceParent::parse(text).is_none(), "{text}");
    }
}

#[test]
fn the_query_and_the_header_carry_the_traceparent() {
    let query = format!("session=abc&traceparent={VALID}");
    assert_eq!(
        TraceParent::from_query(Some(&query)),
        TraceParent::parse(VALID)
    );
    assert!(TraceParent::from_query(Some("session=abc")).is_none());
    assert!(TraceParent::from_query(Some("traceparent=00-zz")).is_none());
    assert!(TraceParent::from_query(None).is_none());
    let mut headers = HeaderMap::new();
    assert!(TraceParent::from_headers(&headers).is_none());
    headers.insert("traceparent", VALID.parse().unwrap());
    assert!(TraceParent::from_headers(&headers).is_some());
    headers.append("traceparent", VALID.parse().unwrap());
    assert!(TraceParent::from_headers(&headers).is_none(), "repeated");
}

#[test]
fn span_ids_are_sixteen_random_lowercase_hex() {
    let (a, b) = (SpanId::random(), SpanId::random());
    assert!(lower_hex_id(&a.0, 16).is_some(), "{a:?}");
    assert_ne!(a, b);
}

#[test]
fn result_codes_come_from_the_fixed_vocabulary() {
    let calls: Vec<(&str, bool)> = [
        CallResult::Ok,
        CallResult::Nak,
        CallResult::Undecodable,
        CallResult::Refused,
        CallResult::NotSent,
    ]
    .map(|result| (result.outcome().code, result.outcome().success))
    .to_vec();
    assert_eq!(
        calls,
        [
            ("ok", true),
            ("nak", true),
            ("undecodable", false),
            ("refused", true),
            ("not-sent", false)
        ]
    );
    let ends: Vec<(&str, bool)> = [
        LineEnd::HungUp,
        LineEnd::Idle,
        LineEnd::Kicked,
        LineEnd::GaveUp,
        LineEnd::InboxFull,
        LineEnd::Error,
    ]
    .map(|end| (end.outcome().code, end.outcome().success))
    .to_vec();
    assert_eq!(
        ends,
        [
            ("hung-up", true),
            ("idle", true),
            ("kicked", true),
            ("gave-up", false),
            ("inbox-full", false),
            ("error", false)
        ]
    );
}

#[test]
fn a_call_targets_its_game_then_its_land_then_the_hub() {
    let lands = LandCatalog::stock();
    let place = |land_type| {
        Some(Place {
            land_type: LandType(land_type),
            land_number: 1,
        })
    };
    assert_eq!(land_target(None, None, &lands), "Hub");
    assert_eq!(land_target(None, place(1), &lands), "Clubhouse");
    assert_eq!(land_target(None, place(5), &lands), "Arena");
    assert_eq!(land_target(None, place(4), &lands), "Yserbius");
    assert_eq!(land_target(None, place(6), &lands), "Cawdor");
    assert_eq!(land_target(None, place(8), &lands), "Twinion");
    assert_eq!(land_target(None, place(77), &lands), "land 77");
    assert_eq!(
        land_target(Some("Red Baron"), place(2), &lands),
        "Red Baron"
    );
}

fn admits(volume: &mut Volume, now: Instant, calls: usize) -> Vec<Admission> {
    (0..calls)
        .map(|_| volume.admit(now, "Send", "Clubhouse", false))
        .collect()
}

#[test]
fn thirty_calls_a_minute_go_out_one_for_one_then_one_in_ten() {
    let mut volume = Volume::default();
    let start = Instant::now();
    let seen = admits(&mut volume, start, 60);
    assert!(seen[..30].iter().all(|a| *a == Admission::Send(None)));
    assert_eq!(seen[30], Admission::Send(Some(10.0)));
    assert!(seen[31..40].iter().all(|a| *a == Admission::Represented));
    assert_eq!(seen[40], Admission::Send(Some(10.0)));
    assert_eq!(seen[50], Admission::Send(Some(10.0)));
    // Another (name, target) pair has its own thirty.
    assert_eq!(
        volume.admit(start, "Send", "Arena", false),
        Admission::Send(None)
    );
    // Failures always go out, one for one.
    assert_eq!(
        volume.admit(start, "Send", "Clubhouse", true),
        Admission::Send(None)
    );
    // A new minute starts the count again.
    let later = start + WINDOW;
    assert_eq!(
        volume.admit(later, "Send", "Clubhouse", false),
        Admission::Send(None)
    );
}

#[test]
fn the_line_ceiling_stops_items_even_failures() {
    let mut volume = Volume::default();
    let now = Instant::now();
    for n in 0..ITEMS_PER_WINDOW {
        let target = format!("land {n}");
        assert_eq!(
            volume.admit(now, "Send", &target, false),
            Admission::Send(None)
        );
    }
    assert_eq!(
        volume.admit(now, "Send", "Hub", false),
        Admission::OverCeiling
    );
    assert_eq!(
        volume.admit(now, "Send", "Hub", true),
        Admission::OverCeiling
    );
}

fn drain(items: &mut mpsc::Receiver<Item>) -> Vec<Item> {
    std::iter::from_fn(|| items.try_recv().ok()).collect()
}

#[test]
fn no_traceparent_means_no_trace_and_no_items() {
    let (tracer, mut items) = channel();
    assert!(tracer.begin_line(Some("session=abc")).is_none());
    assert!(tracer.begin_line(None).is_none());
    let headers = HeaderMap::new();
    assert!(tracer
        .begin_request(&headers, || unreachable!("not named"))
        .is_none());
    assert!(drain(&mut items).is_empty());
}

#[test]
fn a_traced_line_sends_its_calls_under_its_own_request() {
    let (tracer, mut items) = channel();
    let query = format!("traceparent={VALID}");
    let line = tracer.begin_line(Some(&query)).unwrap();
    line.note_client_version(innkeeper_world::ClientVersion::new(1, 1, 7));
    let clock = Stopwatch::start();
    line.record(Call::land("Login", "Hub".into(), clock, CallResult::Nak));
    let saved = Outcome {
        code: "ok",
        success: true,
    };
    line.record(Call::save("manifest", saved, clock));
    line.finish(LineEnd::Kicked);
    let sent = drain(&mut items);
    assert_eq!(sent.len(), 3);
    let request = &sent[2];
    assert_eq!(request.name, "WS line");
    assert_eq!(
        (request.operation.as_str(), request.parent.as_str()),
        (OPERATION, PARENT)
    );
    assert!(request.success);
    assert_eq!(
        request.detail,
        Detail::Request {
            response_code: "kicked".into(),
            measurements: vec![("calls", 2.0), ("dropped", 0.0)],
            properties: vec![("clientVersion", "1.1.7".into())],
        }
    );
    for call in &sent[..2] {
        assert_eq!(call.operation, OPERATION);
        assert_eq!(call.parent, request.id.0, "a call's parent is the line");
        assert_ne!(call.id, request.id);
    }
    assert_eq!(sent[0].name, "Login");
    assert_eq!(
        sent[0].detail,
        Detail::Dependency {
            kind: "Land",
            target: "Hub".into(),
            result_code: "nak"
        }
    );
    assert_eq!(
        sent[1].detail,
        Detail::Dependency {
            kind: "Saves",
            target: "saves".into(),
            result_code: "ok"
        }
    );
}

fn traced_line(tracer: &Tracer) -> Arc<LineTrace> {
    tracer
        .begin_line(Some(&format!("traceparent={VALID}")))
        .unwrap()
}

fn send_calls(line: &LineTrace, calls: usize) {
    let clock = Stopwatch::start();
    for n in 0..calls {
        line.record(Call::land(
            "Send",
            format!("land {n}"),
            clock,
            CallResult::Ok,
        ));
    }
}

#[test]
fn calls_over_the_line_ceiling_are_counted_as_dropped() {
    let (tracer, mut items) = channel();
    let line = traced_line(&tracer);
    let total = ITEMS_PER_WINDOW as usize + 5;
    send_calls(&line, total);
    let state = line.state();
    assert_eq!((state.calls, state.dropped), (total as u64, 5));
    drop(state);
    assert_eq!(drain(&mut items).len(), ITEMS_PER_WINDOW as usize);
}

#[test]
fn a_full_queue_drops_and_counts_and_never_waits() {
    let (tracer, mut items) = channel();
    let lines: Vec<_> = (0..4).map(|_| traced_line(&tracer)).collect();
    for line in &lines {
        send_calls(line, ITEMS_PER_WINDOW as usize);
    }
    let dropped: u64 = lines.iter().map(|line| line.state().dropped).sum();
    let offered = 4 * ITEMS_PER_WINDOW as usize;
    assert_eq!(dropped as usize, offered - QUEUE_ITEMS);
    assert_eq!(drain(&mut items).len(), QUEUE_ITEMS);
}

#[test]
fn an_http_request_is_named_before_it_runs_and_a_4xx_is_no_failure() {
    let (tracer, mut items) = channel();
    let mut headers = HeaderMap::new();
    headers.insert("traceparent", VALID.parse().unwrap());
    for status in [401, 503] {
        let trace = tracer.begin_request(&headers, || "GET /auth/session".into());
        trace.unwrap().finish(status);
    }
    let sent = drain(&mut items);
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].name, "GET /auth/session");
    assert_eq!(sent[0].parent, PARENT);
    assert!(sent[0].success && !sent[1].success);
    let Detail::Request { response_code, .. } = &sent[1].detail else {
        panic!("{:?}", sent[1].detail);
    };
    assert_eq!(response_code, "503");
}
