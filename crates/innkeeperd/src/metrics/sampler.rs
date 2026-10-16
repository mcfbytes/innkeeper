//! Turns samples taken every few seconds into one minute's figures for each metric.

use super::Totals;

/// Min, max, sum and count of the samples of one gauge, as Azure Monitor takes them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stat {
    pub(crate) min: f64,
    pub(crate) max: f64,
    pub(crate) sum: f64,
    pub(crate) count: u64,
}

impl Stat {
    fn one(value: f64) -> Self {
        Stat {
            min: value,
            max: value,
            sum: value,
            count: 1,
        }
    }

    fn add(&mut self, value: f64) {
        self.min = self.min.min(value);
        self.max = self.max.max(value);
        self.sum += value;
        self.count += 1;
    }
}

/// One sample of the gauges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Gauges {
    pub(crate) connections: u64,
    pub(crate) players: u64,
    pub(crate) rss_bytes: Option<u64>,
}

/// One push's worth: the gauges' figures and the counters' growth since the last good push.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Batch {
    pub(crate) gauges: Vec<(&'static str, Stat)>,
    pub(crate) counts: Vec<(&'static str, u64)>,
}

/// Collects gauge samples and counter deltas between pushes.
#[derive(Debug, Default)]
pub(crate) struct Sampler {
    heartbeat: Option<Stat>,
    connections: Option<Stat>,
    players: Option<Stat>,
    rss: Option<Stat>,
    /// Totals when the last push was acknowledged.
    pushed: Totals,
}

fn fold(slot: &mut Option<Stat>, value: f64) {
    match slot {
        Some(stat) => stat.add(value),
        None => *slot = Some(Stat::one(value)),
    }
}

impl Sampler {
    /// A sampler whose counter deltas start from `totals`, not from zero.
    pub(crate) fn starting_at(totals: Totals) -> Self {
        Sampler {
            pushed: totals,
            ..Sampler::default()
        }
    }

    pub(crate) fn sample(&mut self, gauges: Gauges) {
        fold(&mut self.heartbeat, 1.0);
        fold(&mut self.connections, gauges.connections as f64);
        fold(&mut self.players, gauges.players as f64);
        if let Some(rss) = gauges.rss_bytes {
            fold(&mut self.rss, rss as f64);
        }
    }

    /// The figures to push now. Nothing changes until [`Sampler::acknowledge`], so a failed push
    /// leaves the counter growth to go out with the next one.
    pub(crate) fn batch(&self, now: Totals) -> Batch {
        let mut gauges = Vec::new();
        for (name, stat) in [
            ("heartbeat", self.heartbeat),
            ("connections", self.connections),
            ("players", self.players),
            ("rss_bytes", self.rss),
        ] {
            if let Some(stat) = stat {
                gauges.push((name, stat));
            }
        }
        let growth = |now: u64, before: u64| now.saturating_sub(before);
        let counts = [
            ("logins", growth(now.logins, self.pushed.logins)),
            ("signins", growth(now.signins, self.pushed.signins)),
            (
                "accounts_created",
                growth(now.accounts_created, self.pushed.accounts_created),
            ),
            ("ws_refused", growth(now.ws_refused, self.pushed.ws_refused)),
            (
                "store_errors",
                growth(now.store_errors, self.pushed.store_errors),
            ),
            (
                "messages_in",
                growth(now.messages_in, self.pushed.messages_in),
            ),
        ];
        // Idle counters send nothing: a month of zeros is samples to pay for and no news.
        let counts = counts.into_iter().filter(|(_, delta)| *delta > 0).collect();
        Batch { gauges, counts }
    }

    /// The batch for `now` reached Azure: start the next one from here.
    pub(crate) fn acknowledge(&mut self, now: Totals) {
        self.drop_gauges();
        self.pushed = now;
    }

    /// The batch could not be sent and is too old for Azure to take (it accepts the last 20
    /// minutes); the gauges restart and the counters keep their growth.
    pub(crate) fn drop_gauges(&mut self) {
        self.heartbeat = None;
        self.connections = None;
        self.players = None;
        self.rss = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gauges(connections: u64, players: u64) -> Gauges {
        Gauges {
            connections,
            players,
            rss_bytes: Some(1000),
        }
    }

    #[test]
    fn gauges_fold_into_min_max_sum_count() {
        let mut sampler = Sampler::default();
        for connections in [2, 5, 3] {
            sampler.sample(gauges(connections, 1));
        }
        let batch = sampler.batch(Totals::default());
        let connections = batch.gauges.iter().find(|(n, _)| *n == "connections");
        assert_eq!(
            connections.unwrap().1,
            Stat {
                min: 2.0,
                max: 5.0,
                sum: 10.0,
                count: 3
            }
        );
        let heartbeat = batch.gauges.iter().find(|(n, _)| *n == "heartbeat");
        assert_eq!(heartbeat.unwrap().1.sum, 3.0);
    }

    #[test]
    fn counters_send_growth_once_and_skip_idle_ones() {
        let mut sampler = Sampler::starting_at(Totals {
            logins: 10,
            ..Totals::default()
        });
        sampler.sample(gauges(0, 0));
        let now = Totals {
            logins: 13,
            messages_in: 40,
            ..Totals::default()
        };
        let batch = sampler.batch(now);
        assert_eq!(batch.counts, vec![("logins", 3), ("messages_in", 40)]);
        // A failed push keeps the growth for the next one.
        sampler.drop_gauges();
        let later = Totals {
            logins: 14,
            messages_in: 41,
            ..Totals::default()
        };
        assert_eq!(
            sampler.batch(later).counts,
            vec![("logins", 4), ("messages_in", 41)]
        );
        sampler.acknowledge(later);
        assert!(sampler.batch(later).counts.is_empty());
    }

    #[test]
    fn no_rss_sample_means_no_rss_figure() {
        let mut sampler = Sampler::default();
        sampler.sample(Gauges {
            connections: 0,
            players: 0,
            rss_bytes: None,
        });
        let batch = sampler.batch(Totals::default());
        assert!(batch.gauges.iter().all(|(name, _)| *name != "rss_bytes"));
    }
}
