//! The diagnostics trace: request and dependency items for a browser's line or `/auth` call, made only
//! when the browser sent a W3C traceparent (docs/server/innkeeperd.md section 10.4, privacy.md 7a).

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::OsRng;
use hyper::header::HeaderMap;
use innkeeper_world::{LandCatalog, Place};
use tokio::sync::mpsc;

/// Items waiting for the pusher. A full queue drops the newest, so a line never waits.
pub(crate) const QUEUE_ITEMS: usize = 2048;
const WINDOW: Duration = Duration::from_secs(60);
/// Calls per (name, target) and minute that go out one for one; later ones go out one in ten.
const FULL_CALLS_PER_WINDOW: u32 = 30;
const SAMPLE_ONE_IN: u32 = 10;
const ITEMS_PER_WINDOW: u32 = 600;
const LINE_NAME: &str = "WS line";
/// The name of a call whose message could not be decoded, so it has no kind.
pub(crate) const UNDECODABLE: &str = "Undecodable";
const HUB: &str = "Hub";
const SAVES_TARGET: &str = "saves";

/// The operation id and the parent span id of a valid `traceparent`, as lowercase hex.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TraceParent {
    operation: String,
    parent: String,
}

fn is_lower_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn lower_hex_id(text: &str, len: usize) -> Option<String> {
    let nonzero = text.bytes().any(|b| b != b'0');
    (is_lower_hex(text, len) && nonzero).then(|| text.to_owned())
}

impl TraceParent {
    /// `00-<32 hex>-<16 hex>-<2 hex>`, neither id all zeros; anything else is no traceparent.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('-');
        let (version, operation, parent, flags) =
            (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() || version != "00" || !is_lower_hex(flags, 2) {
            return None;
        }
        Some(TraceParent {
            operation: lower_hex_id(operation, 32)?,
            parent: lower_hex_id(parent, 16)?,
        })
    }

    /// The `traceparent` pair of a WebSocket URL's query.
    pub(crate) fn from_query(query: Option<&str>) -> Option<Self> {
        let mut pairs = url::form_urlencoded::parse(query?.as_bytes());
        let (_, value) = pairs.find(|(key, _)| key == "traceparent")?;
        Self::parse(&value)
    }

    /// The `traceparent` request header; a repeated header counts as none.
    pub(crate) fn from_headers(headers: &HeaderMap) -> Option<Self> {
        let mut values = headers.get_all("traceparent").iter();
        let (value, None) = (values.next()?, values.next()) else {
            return None;
        };
        Self::parse(value.to_str().ok()?.trim())
    }
}

/// A random 16-hex span id, never all zeros.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SpanId(pub(crate) String);

impl SpanId {
    fn random() -> Self {
        let mut bytes = [0u8; 8];
        while bytes == [0; 8] {
            OsRng.fill_bytes(&mut bytes);
        }
        SpanId(crate::signin::store::hex(&bytes))
    }
}

/// When something started, on both clocks: the monotonic one times it, the wall one dates it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Stopwatch {
    started: Instant,
    at: i64,
}

impl Stopwatch {
    pub(crate) fn start() -> Self {
        Stopwatch {
            started: Instant::now(),
            at: crate::dates::now_ms(),
        }
    }
}

/// A result code from the fixed vocabulary, and whether the portal counts it as a success.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Outcome {
    pub(crate) code: &'static str,
    pub(crate) success: bool,
}

/// How one client message went. A Nak and a refusal are answers, not faults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CallResult {
    Ok,
    Nak,
    Undecodable,
    Refused,
    NotSent,
}

impl CallResult {
    pub(crate) fn outcome(self) -> Outcome {
        let (code, success) = match self {
            CallResult::Ok => ("ok", true),
            CallResult::Nak => ("nak", true),
            CallResult::Undecodable => ("undecodable", false),
            CallResult::Refused => ("refused", true),
            CallResult::NotSent => ("not-sent", false),
        };
        Outcome { code, success }
    }
}

/// Why a traced line ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineEnd {
    HungUp,
    Idle,
    Kicked,
    GaveUp,
    InboxFull,
    Error,
}

impl LineEnd {
    pub(crate) fn outcome(self) -> Outcome {
        let (code, success) = match self {
            LineEnd::HungUp => ("hung-up", true),
            LineEnd::Idle => ("idle", true),
            LineEnd::Kicked => ("kicked", true),
            LineEnd::GaveUp => ("gave-up", false),
            LineEnd::InboxFull => ("inbox-full", false),
            LineEnd::Error => ("error", false),
        };
        Outcome { code, success }
    }
}

/// The dependency type: what App Map draws one node per target of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CallKind {
    Land,
    Saves,
}

impl CallKind {
    fn name(self) -> &'static str {
        match self {
            CallKind::Land => "Land",
            CallKind::Saves => "Saves",
        }
    }
}

/// One call on a traced line: a client message or a save-channel request.
#[derive(Clone, Debug)]
pub(crate) struct Call {
    pub(crate) name: &'static str,
    pub(crate) kind: CallKind,
    pub(crate) target: String,
    pub(crate) clock: Stopwatch,
    pub(crate) outcome: Outcome,
}

impl Call {
    pub(crate) fn land(
        name: &'static str,
        target: String,
        clock: Stopwatch,
        result: CallResult,
    ) -> Self {
        let outcome = result.outcome();
        Call {
            name,
            kind: CallKind::Land,
            target,
            clock,
            outcome,
        }
    }

    pub(crate) fn save(op: &'static str, outcome: Outcome, clock: Stopwatch) -> Self {
        Call {
            name: op,
            kind: CallKind::Saves,
            target: SAVES_TARGET.to_owned(),
            clock,
            outcome,
        }
    }
}

/// A call's target: the game being played when its name is known, else the land, else `Hub`.
pub(crate) fn land_target(game: Option<&str>, place: Option<Place>, lands: &LandCatalog) -> String {
    if let Some(game) = game {
        return game.to_owned();
    }
    let Some(place) = place else {
        return HUB.to_owned();
    };
    match lands.name_of(place.land_type) {
        Some(name) => name.to_owned(),
        None => format!("land {}", place.land_type.0),
    }
}

/// What the item adds to the common fields.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Detail {
    Request {
        response_code: String,
        measurements: Vec<(&'static str, f64)>,
        properties: Vec<(&'static str, String)>,
    },
    Dependency {
        kind: &'static str,
        target: String,
        result_code: &'static str,
    },
}

/// One request or dependency, ready for the envelope.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Item {
    pub(crate) operation: String,
    pub(crate) parent: String,
    pub(crate) id: SpanId,
    pub(crate) name: String,
    pub(crate) at: i64,
    pub(crate) duration: Duration,
    pub(crate) success: bool,
    /// Percent, when the item stands for more calls than itself.
    pub(crate) sample_rate: Option<f64>,
    pub(crate) detail: Detail,
}

/// The lines' and the HTTP routes' end of the pusher's bounded queue.
#[derive(Clone, Debug)]
pub(crate) struct Tracer {
    queue: mpsc::Sender<Item>,
}

/// A tracer and the queue the pusher drains.
pub(crate) fn channel() -> (Tracer, mpsc::Receiver<Item>) {
    let (queue, items) = mpsc::channel(QUEUE_ITEMS);
    (Tracer { queue }, items)
}

impl Tracer {
    fn send(&self, item: Item) -> bool {
        self.queue.try_send(item).is_ok()
    }

    /// A trace for a WebSocket line, when its URL carried a valid traceparent.
    pub(crate) fn begin_line(&self, query: Option<&str>) -> Option<Arc<LineTrace>> {
        let parent = TraceParent::from_query(query)?;
        Some(Arc::new(LineTrace {
            tracer: self.clone(),
            parent,
            id: SpanId::random(),
            clock: Stopwatch::start(),
            state: Mutex::new(LineState::default()),
        }))
    }

    /// A trace for one HTTP request, when it carried a valid traceparent header.
    pub(crate) fn begin_request(
        &self,
        headers: &HeaderMap,
        name: impl FnOnce() -> String,
    ) -> Option<RequestTrace> {
        let parent = TraceParent::from_headers(headers)?;
        Some(RequestTrace {
            tracer: self.clone(),
            parent,
            name: name(),
            clock: Stopwatch::start(),
        })
    }
}

/// How the volume rules treat one call.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Admission {
    /// Sent, with the sample rate it stands for.
    Send(Option<f64>),
    /// Not sent, but counted by a sampled item of the same name and target.
    Represented,
    /// Not sent and not represented: the line's ceiling for this minute is reached.
    OverCeiling,
}

/// Per line and minute: every failure, the first calls of each (name, target), then one in ten.
#[derive(Debug, Default)]
struct Volume {
    window_start: Option<Instant>,
    items: u32,
    calls: HashMap<(&'static str, String), u32>,
}

impl Volume {
    fn admit(&mut self, now: Instant, name: &'static str, target: &str, failed: bool) -> Admission {
        if self
            .window_start
            .is_none_or(|start| now.saturating_duration_since(start) >= WINDOW)
        {
            *self = Volume {
                window_start: Some(now),
                ..Volume::default()
            };
        }
        let rate = match failed {
            true => None,
            false => {
                let seen = self.count(name, target);
                if seen <= FULL_CALLS_PER_WINDOW {
                    None
                } else if (seen - FULL_CALLS_PER_WINDOW - 1) % SAMPLE_ONE_IN == 0 {
                    Some(100.0 / f64::from(SAMPLE_ONE_IN))
                } else {
                    return Admission::Represented;
                }
            }
        };
        if self.items >= ITEMS_PER_WINDOW {
            return Admission::OverCeiling;
        }
        self.items += 1;
        Admission::Send(rate)
    }

    fn count(&mut self, name: &'static str, target: &str) -> u32 {
        if let Some(seen) = self.calls.get_mut(&(name, target.to_owned())) {
            *seen = seen.saturating_add(1);
            return *seen;
        }
        self.calls.insert((name, target.to_owned()), 1);
        1
    }
}

#[derive(Debug, Default)]
struct LineState {
    calls: u64,
    dropped: u64,
    client_version: Option<String>,
    volume: Volume,
}

/// One traced WebSocket line: its "WS line" request and the calls under it.
#[derive(Debug)]
pub(crate) struct LineTrace {
    tracer: Tracer,
    parent: TraceParent,
    id: SpanId,
    clock: Stopwatch,
    state: Mutex<LineState>,
}

impl LineTrace {
    fn state(&self) -> MutexGuard<'_, LineState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The version a Login presented; the latest one goes with the line's request.
    pub(crate) fn note_client_version(&self, version: impl Display) {
        self.state().client_version = Some(version.to_string());
    }

    pub(crate) fn record(&self, call: Call) {
        let mut state = self.state();
        state.calls += 1;
        let admission = state.volume.admit(
            Instant::now(),
            call.name,
            &call.target,
            !call.outcome.success,
        );
        let sample_rate = match admission {
            Admission::Send(rate) => rate,
            Admission::Represented => return,
            Admission::OverCeiling => {
                state.dropped += 1;
                return;
            }
        };
        let item = Item {
            operation: self.parent.operation.clone(),
            parent: self.id.0.clone(),
            id: SpanId::random(),
            name: call.name.to_owned(),
            at: call.clock.at,
            duration: call.clock.started.elapsed(),
            success: call.outcome.success,
            sample_rate,
            detail: Detail::Dependency {
                kind: call.kind.name(),
                target: call.target,
                result_code: call.outcome.code,
            },
        };
        if !self.tracer.send(item) {
            state.dropped += 1;
        }
    }

    /// Sends the line's own request item; call it once, when the line is over.
    pub(crate) fn finish(&self, end: LineEnd) {
        let state = self.state();
        let outcome = end.outcome();
        let properties = state
            .client_version
            .iter()
            .map(|version| ("clientVersion", version.clone()))
            .collect();
        let item = Item {
            operation: self.parent.operation.clone(),
            parent: self.parent.parent.clone(),
            id: self.id.clone(),
            name: LINE_NAME.to_owned(),
            at: self.clock.at,
            duration: self.clock.started.elapsed(),
            success: outcome.success,
            sample_rate: None,
            detail: Detail::Request {
                response_code: outcome.code.to_owned(),
                measurements: vec![
                    ("calls", state.calls as f64),
                    ("dropped", state.dropped as f64),
                ],
                properties,
            },
        };
        self.tracer.send(item);
    }
}

/// One traced HTTP request, named by method and route template before it runs.
#[derive(Debug)]
pub(crate) struct RequestTrace {
    tracer: Tracer,
    parent: TraceParent,
    name: String,
    clock: Stopwatch,
}

impl RequestTrace {
    /// A 4xx is the page's answer (no session, say); only a 5xx is a failure.
    pub(crate) fn finish(self, status: u16) {
        let item = Item {
            operation: self.parent.operation,
            parent: self.parent.parent,
            id: SpanId::random(),
            name: self.name,
            at: self.clock.at,
            duration: self.clock.started.elapsed(),
            success: status < 500,
            sample_rate: None,
            detail: Detail::Request {
                response_code: status.to_string(),
                measurements: Vec::new(),
                properties: Vec::new(),
            },
        };
        self.tracer.send(item);
    }
}

#[cfg(test)]
mod tests;
