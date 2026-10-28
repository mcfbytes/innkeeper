//! The Application Insights ingestion body: one MetricData envelope per metric, no properties, and
//! one Request or RemoteDependency envelope per diagnostics trace item.

use std::time::Duration;

use serde_json::{json, Map, Value};

use super::sampler::{Batch, Stat};
use super::trace::{Detail, Item};

/// The role name every envelope carries, so the portal can tell this process's rows apart.
pub(crate) const ROLE: &str = "innkeeperd";

/// `YYYY-MM-DDTHH:MM:SSZ` of an instant in Unix milliseconds.
pub(crate) fn iso_time(at: i64) -> String {
    let seconds = at.div_euclid(crate::dates::SECOND).rem_euclid(86_400);
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        crate::dates::format_date(at),
        seconds / 3600,
        seconds % 3600 / 60,
        seconds % 60
    )
}

/// `YYYY-MM-DDTHH:MM:SS.fffZ`: a trace item's start, to the millisecond.
fn iso_time_ms(at: i64) -> String {
    let seconds = iso_time(at);
    let whole = seconds.trim_end_matches('Z');
    format!("{whole}.{:03}Z", at.rem_euclid(crate::dates::SECOND))
}

/// The schema's duration text, `hh:mm:ss.fff`, with a `d.` prefix from one day on.
fn span_duration(duration: Duration) -> String {
    let ms = duration.as_millis();
    let (days, rest) = (ms / 86_400_000, ms % 86_400_000);
    let clock = format!(
        "{:02}:{:02}:{:02}.{:03}",
        rest / 3_600_000,
        rest / 60_000 % 60,
        rest / 1000 % 60,
        rest % 1000
    );
    match days {
        0 => clock,
        days => format!("{days}.{clock}"),
    }
}

/// A counter's growth goes out as one sample of that size.
pub(crate) fn count_stat(delta: u64) -> Stat {
    let value = delta as f64;
    Stat {
        min: value,
        max: value,
        sum: value,
        count: 1,
    }
}

/// One metric as an envelope. `value` is the sum of the samples, as the schema's aggregation says.
fn envelope(ikey: &str, time: i64, name: &str, stat: Stat) -> Value {
    json!({
        "name": "Microsoft.ApplicationInsights.Metric",
        "time": iso_time(time),
        "iKey": ikey,
        "tags": {"ai.cloud.role": ROLE},
        "data": {
            "baseType": "MetricData",
            "baseData": {
                "ver": 2,
                "metrics": [{
                    "name": name,
                    "kind": "Aggregation",
                    "value": stat.sum,
                    "count": stat.count,
                    "min": stat.min,
                    "max": stat.max,
                }],
            },
        },
    })
}

/// The JSON array POSTed to `/v2.1/track` for one batch.
pub(crate) fn envelopes(ikey: &str, time: i64, batch: &Batch) -> Value {
    let gauges = batch.gauges.iter().map(|(name, stat)| (*name, *stat));
    let counts = batch
        .counts
        .iter()
        .map(|(name, delta)| (*name, count_stat(*delta)));
    Value::Array(
        gauges
            .chain(counts)
            .map(|(name, stat)| envelope(ikey, time, name, stat))
            .collect(),
    )
}

fn numbers(pairs: &[(&'static str, f64)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), json!(v)))
        .collect()
}

fn texts(pairs: &[(&'static str, String)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), json!(v)))
        .collect()
}

/// One trace item as an envelope. The client IP tag is zeros, so Azure records no address.
fn trace_envelope(ikey: &str, item: &Item) -> Value {
    let (name, base_type, base_data) = match &item.detail {
        Detail::Request {
            response_code,
            measurements,
            properties,
        } => (
            "Microsoft.ApplicationInsights.Request",
            "RequestData",
            json!({
                "ver": 2,
                "id": item.id.0,
                "name": item.name,
                "duration": span_duration(item.duration),
                "responseCode": response_code,
                "success": item.success,
                "properties": texts(properties),
                "measurements": numbers(measurements),
            }),
        ),
        Detail::Dependency {
            kind,
            target,
            result_code,
        } => (
            "Microsoft.ApplicationInsights.RemoteDependency",
            "RemoteDependencyData",
            json!({
                "ver": 2,
                "id": item.id.0,
                "name": item.name,
                "type": kind,
                "target": target,
                "duration": span_duration(item.duration),
                "resultCode": result_code,
                "success": item.success,
                "properties": {},
            }),
        ),
    };
    let mut envelope = json!({
        "name": name,
        "time": iso_time_ms(item.at),
        "iKey": ikey,
        "tags": {
            "ai.cloud.role": ROLE,
            "ai.operation.id": item.operation,
            "ai.operation.parentId": item.parent,
            "ai.location.ip": "0.0.0.0",
        },
        "data": {"baseType": base_type, "baseData": base_data},
    });
    if let (Some(rate), Value::Object(map)) = (item.sample_rate, &mut envelope) {
        map.insert("sampleRate".to_owned(), json!(rate));
    }
    envelope
}

/// The JSON array POSTed to `/v2.1/track` for a batch of trace items.
pub(crate) fn trace_envelopes(ikey: &str, items: &[Item]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|item| trace_envelope(ikey, item))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_is_iso_8601_utc() {
        assert_eq!(iso_time(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_time(1_700_000_000_123), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn a_batch_is_one_metric_envelope_per_figure() {
        let batch = Batch {
            gauges: vec![(
                "connections",
                Stat {
                    min: 1.0,
                    max: 4.0,
                    sum: 10.0,
                    count: 6,
                },
            )],
            counts: vec![("logins", 3)],
        };
        let value = envelopes("key-1", 0, &batch);
        let list = value.as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["iKey"], "key-1");
        assert_eq!(list[0]["name"], "Microsoft.ApplicationInsights.Metric");
        assert_eq!(list[0]["data"]["baseType"], "MetricData");
        let metric = &list[0]["data"]["baseData"]["metrics"][0];
        assert_eq!(metric["name"], "connections");
        assert_eq!(metric["value"], 10.0);
        assert_eq!(metric["count"], 6);
        assert_eq!(list[1]["data"]["baseData"]["metrics"][0]["value"], 3.0);
        assert!(list[1]["data"]["baseData"].get("properties").is_none());
    }

    #[test]
    fn a_duration_is_the_schema_s_clock_text() {
        assert_eq!(span_duration(Duration::from_millis(1234)), "00:00:01.234");
        assert_eq!(span_duration(Duration::from_secs(3723)), "01:02:03.000");
        assert_eq!(
            span_duration(Duration::from_millis(90_061_005)),
            "1.01:01:01.005"
        );
        assert_eq!(iso_time_ms(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
    }

    fn item(detail: Detail, sample_rate: Option<f64>) -> Item {
        Item {
            operation: "4bf92f3577b34da6a3ce929d0e0e4736".to_owned(),
            parent: "00f067aa0ba902b7".to_owned(),
            id: super::super::trace::SpanId("b7ad6b7169203331".to_owned()),
            name: "Login".to_owned(),
            at: 0,
            duration: Duration::from_millis(5),
            success: true,
            sample_rate,
            detail,
        }
    }

    #[test]
    fn a_dependency_item_is_a_remote_dependency_envelope() {
        let detail = Detail::Dependency {
            kind: "Land",
            target: "Hub".to_owned(),
            result_code: "ok",
        };
        let list = trace_envelopes("key-1", &[item(detail, Some(10.0))]);
        let envelope = &list[0];
        assert_eq!(
            envelope["name"],
            "Microsoft.ApplicationInsights.RemoteDependency"
        );
        assert_eq!(envelope["sampleRate"], 10.0);
        assert_eq!(envelope["time"], "1970-01-01T00:00:00.000Z");
        let tags = &envelope["tags"];
        assert_eq!(tags["ai.cloud.role"], "innkeeperd");
        assert_eq!(tags["ai.operation.id"], "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(tags["ai.operation.parentId"], "00f067aa0ba902b7");
        assert_eq!(tags["ai.location.ip"], "0.0.0.0");
        assert_eq!(envelope["data"]["baseType"], "RemoteDependencyData");
        let data = &envelope["data"]["baseData"];
        assert_eq!(
            data,
            &json!({"ver": 2, "id": "b7ad6b7169203331", "name": "Login", "type": "Land",
                    "target": "Hub", "duration": "00:00:00.005", "resultCode": "ok",
                    "success": true, "properties": {}})
        );
    }

    #[test]
    fn a_request_item_is_a_request_envelope_without_a_sample_rate() {
        let detail = Detail::Request {
            response_code: "hung-up".to_owned(),
            measurements: vec![("calls", 3.0), ("dropped", 0.0)],
            properties: vec![("clientVersion", "1.1.7".to_owned())],
        };
        let list = trace_envelopes("key-1", &[item(detail, None)]);
        let envelope = &list[0];
        assert_eq!(envelope["name"], "Microsoft.ApplicationInsights.Request");
        assert!(envelope.get("sampleRate").is_none());
        assert_eq!(envelope["data"]["baseType"], "RequestData");
        let data = &envelope["data"]["baseData"];
        assert_eq!(data["responseCode"], "hung-up");
        assert_eq!(data["measurements"], json!({"calls": 3.0, "dropped": 0.0}));
        assert_eq!(data["properties"], json!({"clientVersion": "1.1.7"}));
        assert_eq!(data["duration"], "00:00:00.005");
    }
}
