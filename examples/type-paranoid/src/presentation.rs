use std::sync::atomic::Ordering;

use anyhow::Result;
use serde_json::json;

use crate::{
    assess::Batch,
    capture::Capture,
    cli::Analysis,
    dashboard::Dashboard,
    engine::{Observation, Window},
    output,
};

/// Own all terminal output here, including results from background assessment jobs.
pub struct Presentation {
    dashboard: Option<Box<Dashboard>>,
    json: bool,
}

impl Presentation {
    pub fn new(interface: &str, window_secs: u64, options: &Analysis) -> Result<Self> {
        let dashboard = options
            .pretty
            .then(|| Dashboard::new(interface, window_secs, options.assess))
            .transpose()?
            .map(Box::new);
        Ok(Self {
            dashboard,
            json: options.json,
        })
    }

    pub fn observe(&mut self, packet: Observation) {
        if let Some(ui) = &mut self.dashboard {
            ui.state.observe(packet);
        }
    }

    pub fn window(&mut self, window: &Window) {
        if let Some(ui) = &mut self.dashboard {
            ui.state.window(window);
        } else {
            output::window(window, self.json);
        }
    }

    pub fn health(&mut self, capture: &Capture) {
        if let Some(ui) = &mut self.dashboard {
            ui.state.dropped = capture.dropped.load(Ordering::Relaxed);
            ui.state.unparsed = capture.unparsed.load(Ordering::Relaxed);
        } else {
            output::capture_health(capture, self.json);
        }
    }

    pub fn assessment(&mut self, batch: &Batch) {
        if let Some(ui) = &mut self.dashboard {
            ui.state.assessment(batch);
        } else {
            output::assessment(batch, self.json);
        }
    }

    pub fn skipped(&mut self, window: u64) {
        if let Some(ui) = &mut self.dashboard {
            ui.state.skipped(window);
        } else if self.json {
            println!(
                "{}",
                json!({"event": "assessment_skipped", "window": window, "reason": "concurrency_limit"})
            );
        } else {
            eprintln!(
                "Assessment skipped: both request slots are busy; local findings remain available."
            );
        }
    }

    pub fn refresh(&mut self, capture: &Capture, inflight: usize) -> Result<()> {
        if let Some(ui) = &mut self.dashboard {
            ui.state.dropped = capture.dropped.load(Ordering::Relaxed);
            ui.state.unparsed = capture.unparsed.load(Ordering::Relaxed);
            ui.state.inflight = inflight;
            ui.refresh()?;
        }
        Ok(())
    }

    pub async fn input(&mut self) -> Result<bool> {
        match &mut self.dashboard {
            Some(ui) => ui.input().await,
            None => std::future::pending().await,
        }
    }
}
