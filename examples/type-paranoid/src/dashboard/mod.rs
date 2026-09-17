mod render;
mod state;

#[cfg(test)]
mod tests;

use std::{
    io::{self, IsTerminal},
    time::Instant,
};

use anyhow::{Context, Result, bail};
use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;

pub use state::State;

pub struct Dashboard {
    terminal: ratatui::DefaultTerminal,
    events: EventStream,
    started: Instant,
    pub state: State,
}

pub fn ensure_terminal() -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!(
            "--pretty requires an interactive terminal on stdin and stdout; omit it for plain output or use --json"
        );
    }
    Ok(())
}

impl Dashboard {
    pub fn new(interface: &str, window_secs: u64, assess: bool) -> Result<Self> {
        ensure_terminal()?;
        let terminal = ratatui::try_init()
            .inspect_err(|_| ratatui::restore())
            .context("could not initialize terminal dashboard")?;
        Ok(Self {
            terminal,
            events: EventStream::new(),
            started: Instant::now(),
            state: State::new(interface, window_secs, assess),
        })
    }

    pub fn refresh(&mut self) -> Result<()> {
        self.state.tick(self.started.elapsed().as_secs_f64());
        self.terminal
            .draw(|frame| render::draw(frame, &self.state))?;
        Ok(())
    }

    pub async fn input(&mut self) -> Result<bool> {
        match self
            .events
            .next()
            .await
            .transpose()
            .context("terminal input failed")?
        {
            Some(event) => Ok(is_quit(&event)),
            None => bail!("terminal input closed"),
        }
    }
}

impl Drop for Dashboard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

fn is_quit(event: &Event) -> bool {
    matches!(event, Event::Key(key) if key.kind != KeyEventKind::Release &&
        (matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) ||
        (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))))
}
