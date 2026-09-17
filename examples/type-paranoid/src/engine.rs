use std::{
    collections::{HashMap, HashSet, VecDeque},
    net::SocketAddr,
};

use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    Tcp,
    Udp,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
pub struct Peer {
    pub remote: SocketAddr,
    pub protocol: Protocol,
}

#[derive(Clone, Copy, Debug)]
pub struct Observation {
    pub peer: Peer,
    pub outbound: bool,
    pub payload_bytes: u64,
}

pub struct Config {
    pub upload_bytes: u64,
    pub min_upload_bytes: u64,
    pub expected: HashSet<SocketAddr>,
    pub max_peers: usize,
    pub history_windows: usize,
    pub max_candidates: usize,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            upload_bytes: 10_485_760,
            min_upload_bytes: 262_144,
            expected: HashSet::new(),
            max_peers: 4096,
            history_windows: 6,
            max_candidates: 8,
        }
    }
}

#[derive(Default)]
struct Traffic {
    sent: u64,
    received: u64,
    packets: u64,
}

struct History {
    // Bytes per second, so a partial final window doesn't look like a full window.
    rates: VecDeque<f64>,
    last_active: u64,
}

#[derive(Debug, Serialize)]
pub struct Candidate {
    pub peer: Peer,
    pub sent_bytes: u64,
    pub received_bytes: u64,
    pub packets: u64,
    pub new_in_retained_history: bool,
    pub baseline_windows: usize,
    pub baseline_sent_bytes_per_second: Option<f64>,
    pub current_sent_bytes_per_second: f64,
    pub signals: Vec<&'static str>,
    /// Packet capture alone cannot identify the owning process.
    pub process: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Window {
    pub number: u64,
    pub seconds: f64,
    pub peers: usize,
    pub sent_bytes: u64,
    pub received_bytes: u64,
    pub untracked_packets: u64,
    pub candidates: Vec<Candidate>,
    pub omitted_candidates: usize,
    pub history_peers: usize,
}

pub struct Engine {
    config: Config,
    traffic: HashMap<Peer, Traffic>,
    history: HashMap<Peer, History>,
    window: u64,
    untracked: u64,
    sent: u64,
    received: u64,
}
impl Engine {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            traffic: HashMap::new(),
            history: HashMap::new(),
            window: 0,
            untracked: 0,
            sent: 0,
            received: 0,
        }
    }

    pub fn observe(&mut self, event: Observation) {
        if event.outbound {
            self.sent = self.sent.saturating_add(event.payload_bytes);
        } else {
            self.received = self.received.saturating_add(event.payload_bytes);
        }
        if !self.traffic.contains_key(&event.peer) && self.traffic.len() >= self.config.max_peers {
            self.untracked = self.untracked.saturating_add(1);
            return;
        }
        let traffic = self.traffic.entry(event.peer).or_default();
        traffic.packets = traffic.packets.saturating_add(1);
        if event.outbound {
            traffic.sent = traffic.sent.saturating_add(event.payload_bytes);
        } else {
            traffic.received = traffic.received.saturating_add(event.payload_bytes);
        }
    }

    pub fn finish(&mut self, seconds: f64) -> Window {
        self.window += 1;
        let seconds = seconds.max(0.001);
        self.history
            .retain(|_, h| self.window - h.last_active <= self.config.history_windows as u64);
        let mut candidates = Vec::new();
        for (&peer, traffic) in &self.traffic {
            let history = self.history.get(&peer);
            let baseline_windows = history.map_or(0, |h| h.rates.len());
            let baseline = history
                .filter(|h| !h.rates.is_empty())
                .map(|h| h.rates.iter().sum::<f64>() / h.rates.len() as f64);
            let rate = traffic.sent as f64 / seconds;
            let mut signals = Vec::new();
            if !self.config.expected.contains(&peer.remote) {
                if traffic.sent >= self.config.upload_bytes {
                    signals.push("large_outbound_transfer");
                }
                if traffic.sent >= self.config.min_upload_bytes {
                    if history.is_none() && traffic.sent >= traffic.received.saturating_mul(10) {
                        signals.push("upload_to_new_peer");
                    }
                    if baseline_windows >= 3
                        && baseline.is_some_and(|mean| rate >= mean.max(1.0) * 4.0)
                    {
                        signals.push("outbound_rate_above_baseline");
                    }
                }
            }
            if !signals.is_empty() {
                candidates.push(Candidate {
                    peer,
                    sent_bytes: traffic.sent,
                    received_bytes: traffic.received,
                    packets: traffic.packets,
                    new_in_retained_history: history.is_none(),
                    baseline_windows,
                    baseline_sent_bytes_per_second: baseline,
                    current_sent_bytes_per_second: rate,
                    signals,
                    process: None,
                });
            }
        }
        candidates.sort_by(|a, b| {
            b.sent_bytes
                .cmp(&a.sent_bytes)
                .then_with(|| a.peer.remote.cmp(&b.peer.remote))
        });
        let omitted_candidates = candidates.len().saturating_sub(self.config.max_candidates);
        candidates.truncate(self.config.max_candidates);

        // History has the same hard cap as the current window.
        for (peer, history) in &mut self.history {
            let rate = self
                .traffic
                .get(peer)
                .map_or(0.0, |t| t.sent as f64 / seconds);
            if self.traffic.contains_key(peer) {
                history.last_active = self.window;
            }
            history.rates.push_back(rate);
            if history.rates.len() > self.config.history_windows {
                history.rates.pop_front();
            }
        }
        for (&peer, traffic) in &self.traffic {
            if self.history.len() >= self.config.max_peers {
                break;
            }
            self.history.entry(peer).or_insert_with(|| History {
                rates: VecDeque::from([traffic.sent as f64 / seconds]),
                last_active: self.window,
            });
        }
        let result = Window {
            number: self.window,
            seconds,
            peers: self.traffic.len(),
            sent_bytes: std::mem::take(&mut self.sent),
            received_bytes: std::mem::take(&mut self.received),
            untracked_packets: std::mem::take(&mut self.untracked),
            candidates,
            omitted_candidates,
            history_peers: self.history.len(),
        };
        self.traffic.clear();
        result
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
