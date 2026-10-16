//! The Application Insights ingestion body: one MetricData envelope per metric, no properties.

use serde_json::{json, Value};

use super::sampler::{Batch, Stat};

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
}
