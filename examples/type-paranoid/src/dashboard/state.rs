use std::collections::VecDeque;

use crate::{
    assess::Batch,
    engine::{Observation, Peer, Window},
};

pub struct Alert {
    pub window: u64,
    pub peer: Peer,
    pub sent: u64,
    pub signals: String,
    pub assessment: String,
}

pub struct State {
    pub interface: String,
    pub window_secs: u64,
    pub assess: bool,
    pub elapsed: f64,
    pub sent: u64,
    pub received: u64,
    pub outbound_rate: f64,
    pub inbound_rate: f64,
    pub outbound: VecDeque<(f64, f64)>,
    pub inbound: VecDeque<(f64, f64)>,
    pub alerts: VecDeque<Alert>,
    pub candidates: u64,
    pub window: u64,
    pub peers: usize,
    pub dropped: u64,
    pub unparsed: u64,
    pub untracked: u64,
    pub inflight: usize,
    pub last_window_at: f64,
    pub notice: String,
    sampled_at: f64,
    sampled_sent: u64,
    sampled_received: u64,
}

impl State {
    pub fn new(interface: &str, window_secs: u64, assess: bool) -> Self {
        Self {
            interface: interface.into(),
            window_secs,
            assess,
            elapsed: 0.0,
            sent: 0,
            received: 0,
            outbound_rate: 0.0,
            inbound_rate: 0.0,
            outbound: VecDeque::new(),
            inbound: VecDeque::new(),
            alerts: VecDeque::new(),
            candidates: 0,
            window: 0,
            peers: 0,
            dropped: 0,
            unparsed: 0,
            untracked: 0,
            inflight: 0,
            last_window_at: 0.0,
            sampled_at: 0.0,
            sampled_sent: 0,
            sampled_received: 0,
            notice: if assess {
                "TypeSafe ready; candidates are assessed at window boundaries."
            } else {
                "Local rules only. Add --assess to enable TypeSafe."
            }
            .into(),
        }
    }

    pub fn observe(&mut self, observation: Observation) {
        if observation.outbound {
            self.sent = self.sent.saturating_add(observation.payload_bytes);
        } else {
            self.received = self.received.saturating_add(observation.payload_bytes);
        }
    }

    pub fn tick(&mut self, elapsed: f64) {
        self.elapsed = elapsed;
        let seconds = elapsed - self.sampled_at;
        if seconds < 1.0 {
            return;
        }
        self.outbound_rate = (self.sent - self.sampled_sent) as f64 / seconds;
        self.inbound_rate = (self.received - self.sampled_received) as f64 / seconds;
        self.sampled_at = elapsed;
        self.sampled_sent = self.sent;
        self.sampled_received = self.received;
        for (points, rate) in [
            (&mut self.outbound, self.outbound_rate),
            (&mut self.inbound, self.inbound_rate),
        ] {
            points.push_back((elapsed, rate));
            while points.len() > 120 || points.front().is_some_and(|p| elapsed - p.0 > 120.0) {
                points.pop_front();
            }
        }
    }

    pub fn window(&mut self, window: &Window) {
        self.window = window.number;
        self.last_window_at = self.elapsed;
        self.peers = window.peers;
        self.untracked = self.untracked.saturating_add(window.untracked_packets);
        self.candidates = self
            .candidates
            .saturating_add((window.candidates.len() + window.omitted_candidates) as u64);
        for candidate in window.candidates.iter().rev() {
            self.alerts.push_front(Alert {
                window: window.number,
                peer: candidate.peer,
                sent: candidate.sent_bytes,
                signals: candidate.signals.join(", ").replace('_', " "),
                assessment: if self.assess { "pending" } else { "local only" }.into(),
            });
        }
        self.alerts.truncate(32);
        if window.omitted_candidates > 0 {
            self.notice = format!(
                "Window {}: {} additional candidates omitted by the assessment cap.",
                window.number, window.omitted_candidates
            );
        }
    }

    pub fn assessment(&mut self, batch: &Batch) {
        match &batch.result {
            Ok((assessments, _)) => {
                for assessment in assessments {
                    for alert in &mut self.alerts {
                        if alert.window == batch.window && alert.peer == assessment.peer {
                            alert.assessment = format!(
                                "{} {:.1}/3",
                                assessment.classification, assessment.review_priority
                            );
                        }
                    }
                }
                self.notice = format!(
                    "TypeSafe completed window {}. Scores indicate review priority, not proof of exfiltration.",
                    batch.window
                );
            }
            Err(error) => {
                self.mark(batch.window, "API error");
                self.notice = format!("TypeSafe: {error}; local findings remain available.");
            }
        }
    }

    pub fn skipped(&mut self, window: u64) {
        self.mark(window, "skipped");
        self.notice =
            format!("Window {window}: TypeSafe slots busy; local findings remain available.");
    }

    fn mark(&mut self, window: u64, status: &str) {
        for alert in &mut self.alerts {
            if alert.window == window {
                alert.assessment = status.into();
            }
        }
    }
}

pub fn bytes(value: f64) -> String {
    let (value, unit) = if value >= 1_073_741_824.0 {
        (value / 1_073_741_824.0, "GiB")
    } else if value >= 1_048_576.0 {
        (value / 1_048_576.0, "MiB")
    } else if value >= 1024.0 {
        (value / 1024.0, "KiB")
    } else {
        (value, "B")
    };
    format!("{value:.1} {unit}")
}
