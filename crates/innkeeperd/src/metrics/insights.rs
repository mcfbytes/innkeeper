//! `--app-insights-connection-string`: pushes the process counters and the diagnostics trace items
//! to Application Insights' ingestion endpoint, authenticated by the VM's managed identity
//! (docs/server/innkeeperd.md section 10). The connection string's key is no secret.
//!
//! The task owns everything it touches and awaits only with a timeout, so a slow or unreachable
//! Azure never delays a game connection. Failures back off and log at most once per ten minutes.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{info, warn};
use url::Url;
use zeroize::Zeroizing;

use super::payload::{envelopes, trace_envelopes};
use super::sampler::{Batch, Gauges, Sampler};
use super::trace::{self, Item, Tracer, QUEUE_ITEMS};
use super::{counters, rss_bytes};
use crate::config::Config;
use crate::connection::ConnectionSettings;
use crate::keyvault::{join, Credential};

const IMDS: &str = "http://169.254.169.254";
const AUTHORITY: &str = "https://login.microsoftonline.com";
/// The audience of the ingestion endpoint's Entra auth. The trailing slash is part of it.
const RESOURCE: &str = "https://monitor.azure.com/";
const INGESTION_SUFFIX: &str = ".applicationinsights.azure.com";
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
const PUSH_EVERY: Duration = Duration::from_secs(60);
/// Samples per push: one every ten seconds at the default interval.
const SAMPLES_PER_PUSH: u32 = 6;
/// Trace drains per metrics push: one every five seconds at the default interval.
const DRAINS_PER_PUSH: u32 = 12;
const FIRST_BACKOFF: Duration = Duration::from_secs(60);
const MAX_BACKOFF: Duration = Duration::from_secs(600);
const WARN_EVERY: Duration = Duration::from_secs(600);
/// A token is renewed this long before it expires.
const TOKEN_MARGIN: Duration = Duration::from_secs(300);

/// What the pusher needs, collected before the sandbox so a certificate file can still be read.
#[derive(Debug)]
pub(crate) struct Settings {
    credential: Credential,
    imds: Url,
    authority: Url,
    ikey: String,
    /// The full `.../v2.1/track` URL.
    track: Url,
    interval: Duration,
    /// The trace queue, once [`Settings::tracer`] has made it.
    items: Option<mpsc::Receiver<Item>>,
}

/// `InstrumentationKey` and `IngestionEndpoint` out of a connection string; other keys are ignored.
fn parse_connection_string(text: &str) -> Result<(String, Url), String> {
    let (mut ikey, mut endpoint) = (None, None);
    for part in text.split(';') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "instrumentationkey" => ikey = Some(value.trim().to_owned()),
            "ingestionendpoint" => endpoint = Some(value.trim().to_owned()),
            _ => {}
        }
    }
    let ikey = ikey.ok_or("the connection string has no InstrumentationKey")?;
    if ikey.is_empty() || !ikey.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        return Err("the InstrumentationKey is not a GUID".to_owned());
    }
    let endpoint = endpoint.ok_or("the connection string has no IngestionEndpoint")?;
    let url = Url::parse(&endpoint).map_err(|_| "the IngestionEndpoint is not a URL")?;
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    if url.scheme() != "https" || !host.ends_with(INGESTION_SUFFIX) {
        return Err(
            "the IngestionEndpoint must be https://NAME.applicationinsights.azure.com".to_owned(),
        );
    }
    Ok((ikey, url))
}

impl Settings {
    /// `None` unless `--app-insights-connection-string` was given.
    pub(crate) fn from_config(config: &Config) -> Result<Option<Self>, String> {
        let Some(text) = &config.app_insights_connection_string else {
            return Ok(None);
        };
        let parse = |text: &str| Url::parse(text).map_err(|error| format!("{text}: {error}"));
        let (ikey, endpoint) = parse_connection_string(text)?;
        let endpoint = match &config.test_ingestion_endpoint {
            Some(test) => parse(test)?,
            None => endpoint,
        };
        let track = join(&endpoint, "v2.1/track")?;
        let credential = Credential::from_env().map_err(|error| error.to_string())?;
        let imds = parse(config.test_imds_endpoint.as_deref().unwrap_or(IMDS))?;
        let authority = parse(
            config
                .test_authority_endpoint
                .as_deref()
                .unwrap_or(AUTHORITY),
        )?;
        let interval = config
            .test_metrics_interval
            .map_or(PUSH_EVERY, |interval| *interval);
        Ok(Some(Settings {
            credential,
            imds,
            authority,
            ikey,
            track,
            interval,
            items: None,
        }))
    }

    /// The lines' end of the trace queue this pusher drains.
    pub(crate) fn tracer(&mut self) -> Tracer {
        let (tracer, items) = trace::channel();
        self.items = Some(items);
        tracer
    }
}

/// Lets a warning through at most once per `period`.
#[derive(Debug)]
struct WarnLimiter {
    period: Duration,
    last: Option<Instant>,
}

impl WarnLimiter {
    fn new(period: Duration) -> Self {
        WarnLimiter { period, last: None }
    }

    fn allow(&mut self, now: Instant) -> bool {
        let due = self
            .last
            .is_none_or(|last| now.saturating_duration_since(last) >= self.period);
        if due {
            self.last = Some(now);
        }
        due
    }
}

/// The wait after `failures` failed pushes in a row: a minute, doubling, at most ten.
fn backoff(failures: u32) -> Duration {
    let shift = failures.saturating_sub(1).min(8);
    FIRST_BACKOFF.saturating_mul(1 << shift).min(MAX_BACKOFF)
}

struct Cached {
    token: Zeroizing<String>,
    renew_at: Instant,
}

struct Client {
    http: reqwest::Client,
    settings: Settings,
    token: Option<Cached>,
}

impl Client {
    fn new(settings: Settings) -> Result<Self, String> {
        // IMDS must never go through a proxy, and nothing here follows redirects.
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Client {
            http,
            settings,
            token: None,
        })
    }

    async fn token(&mut self) -> Result<Zeroizing<String>, String> {
        if let Some(cached) = &self.token {
            if Instant::now() < cached.renew_at {
                return Ok(cached.token.clone());
            }
        }
        let settings = &self.settings;
        let request = settings.credential.token_request_for(
            &self.http,
            &settings.imds,
            &settings.authority,
            RESOURCE,
        )?;
        let response = request
            .send()
            .await
            .map_err(|error| format!("token: {}", error.without_url()))?;
        let status = response.status();
        let answer: Value = response.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            let code = answer
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("no error code");
            return Err(format!("token: status {status}, {code}"));
        }
        let token = answer
            .get("access_token")
            .and_then(Value::as_str)
            .ok_or("token: the answer has no access_token")?;
        let token = Zeroizing::new(token.to_owned());
        let lifetime = match answer.get("expires_in") {
            Some(Value::String(text)) => text.parse().ok(),
            Some(other) => other.as_u64(),
            None => None,
        }
        .unwrap_or(3000);
        let renew_in = Duration::from_secs(lifetime)
            .saturating_sub(TOKEN_MARGIN)
            .max(Duration::from_secs(30));
        self.token = Some(Cached {
            token: token.clone(),
            renew_at: Instant::now() + renew_in,
        });
        Ok(token)
    }

    /// Posts the batch as one array of envelopes.
    async fn push(&mut self, batch: &Batch, time: i64) -> Result<(), String> {
        if batch.gauges.is_empty() && batch.counts.is_empty() {
            return Ok(());
        }
        let body = envelopes(&self.settings.ikey, time, batch);
        self.post(&body).await
    }

    async fn push_items(&mut self, items: &[Item]) -> Result<(), String> {
        let body = trace_envelopes(&self.settings.ikey, items);
        self.post(&body).await
    }

    async fn post(&mut self, body: &Value) -> Result<(), String> {
        let token = self.token().await?;
        let response = self
            .http
            .post(self.settings.track.clone())
            .bearer_auth(token.as_str())
            .json(body)
            .send()
            .await
            .map_err(|error| format!("track: {}", error.without_url()))?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            self.token = None;
        }
        // 206 means some items were refused (too old, say); resending them would not help.
        if status.is_success() {
            Ok(())
        } else {
            Err(format!("track: status {status}"))
        }
    }
}

fn gauges(settings: &ConnectionSettings) -> Gauges {
    Gauges {
        connections: settings.switchboard.count() as u64,
        players: counters().totals().players,
        rss_bytes: rss_bytes(),
    }
}

/// The shared back-off of the metrics and trace posts: one endpoint, one token, one health.
#[derive(Debug)]
struct Health {
    failures: u32,
    retry_at: Instant,
    warnings: WarnLimiter,
}

impl Health {
    fn new() -> Self {
        Health {
            failures: 0,
            retry_at: Instant::now(),
            warnings: WarnLimiter::new(WARN_EVERY),
        }
    }

    fn backing_off(&self) -> bool {
        Instant::now() < self.retry_at
    }

    fn note(&mut self, pushed: Result<(), String>) -> bool {
        match pushed {
            Ok(()) => {
                if self.failures > 0 {
                    info!("app insights metrics: pushing again");
                }
                self.failures = 0;
                true
            }
            Err(error) => {
                self.failures = self.failures.saturating_add(1);
                self.retry_at = Instant::now() + backoff(self.failures);
                if self.warnings.allow(Instant::now()) {
                    let failures = self.failures;
                    warn!(%error, failures, "app insights metrics: push failed; will retry");
                }
                false
            }
        }
    }
}

/// Everything queued now, up to one queue's worth.
fn take_items(items: &mut mpsc::Receiver<Item>) -> Vec<Item> {
    let mut batch = Vec::new();
    while batch.len() < QUEUE_ITEMS {
        let Ok(item) = items.try_recv() else {
            break;
        };
        batch.push(item);
    }
    batch
}

fn ticker(period: Duration) -> tokio::time::Interval {
    let mut ticker = tokio::time::interval(period.max(Duration::from_millis(1)));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker
}

/// Samples every `interval / 6` and pushes every `interval`; drains the trace queue every
/// `interval / 12`. Runs until the process ends.
pub(crate) async fn run(mut settings: Settings, connections: Arc<ConnectionSettings>) {
    let interval = settings.interval;
    let mut items = settings.items.take();
    let mut client = match Client::new(settings) {
        Ok(client) => client,
        Err(error) => {
            warn!(%error, "app insights metrics are off: cannot start the HTTP client");
            return;
        }
    };
    let mut sampler = Sampler::starting_at(counters().totals());
    let mut health = Health::new();
    let mut samples = ticker(interval / SAMPLES_PER_PUSH);
    let mut drains = ticker(interval / DRAINS_PER_PUSH);
    let mut ticks = 0u32;
    loop {
        tokio::select! {
            _ = samples.tick() => {}
            _ = drains.tick(), if items.is_some() => {
                let batch = items.as_mut().map(take_items).unwrap_or_default();
                // Items cannot wait out a back-off; dropping them keeps the queue moving.
                if !batch.is_empty() && !health.backing_off() {
                    health.note(client.push_items(&batch).await);
                }
                continue;
            }
        }
        sampler.sample(gauges(&connections));
        ticks += 1;
        if ticks < SAMPLES_PER_PUSH {
            continue;
        }
        ticks = 0;
        if health.backing_off() {
            // Gauges older than Azure's window are no use; counters keep their growth.
            sampler.drop_gauges();
            continue;
        }
        let now = counters().totals();
        let batch = sampler.batch(now);
        let time = crate::dates::now_ms();
        match health.note(client.push(&batch, time).await) {
            true => sampler.acknowledge(now),
            false => sampler.drop_gauges(),
        }
    }
}

#[cfg(test)]
mod tests;
