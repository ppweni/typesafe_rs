use super::*;
use crate::{
    assess::{Assessment, Batch},
    engine::{Config, Engine, Observation, Peer, Protocol},
};
use crossterm::event::{KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};

fn packet(bytes: u64, outbound: bool) -> Observation {
    Observation {
        peer: Peer {
            remote: "198.51.100.77:8443".parse().unwrap(),
            protocol: Protocol::Tcp,
        },
        outbound,
        payload_bytes: bytes,
    }
}

#[test]
fn rates_use_actual_elapsed_time_and_history_is_bounded() {
    let mut state = State::new("en1", 10, false);
    state.observe(packet(4096, true));
    state.observe(packet(1024, false));
    state.tick(0.25);
    assert!(state.outbound.is_empty());
    state.tick(2.0);
    assert_eq!(state.outbound_rate, 2048.0);
    assert_eq!(state.inbound_rate, 512.0);
    for second in 3..200 {
        state.tick(second as f64);
    }
    assert_eq!(state.outbound.len(), 120);
    assert_eq!(state.outbound_rate, 0.0);
    assert_eq!(state.sent, 4096);
}

#[test]
fn late_assessment_updates_only_its_own_window() {
    let mut engine = Engine::new(Config::default());
    let mut state = State::new("en1", 10, true);
    for _ in 0..2 {
        engine.observe(packet(30_000_000, true));
        state.window(&engine.finish(10.0));
    }
    state.assessment(&Batch {
        window: 1,
        result: Ok((
            vec![Assessment {
                peer: packet(0, true).peer,
                classification: "unusual".into(),
                review_priority: 2.5,
                classification_confidence: 0.8,
            }],
            None,
        )),
    });
    assert_eq!(state.alerts[0].assessment, "pending");
    assert_eq!(state.alerts[1].assessment, "unusual 2.5/3");
    state.assessment(&Batch {
        window: 2,
        result: Err(anyhow::anyhow!("offline")),
    });
    assert_eq!(state.alerts[0].assessment, "API error");
    assert!(state.notice.contains("offline"));
}

#[test]
fn renders_graphs_candidates_and_compact_sizes() {
    let mut state = State::new("en1", 10, true);
    let mut engine = Engine::new(Config::default());
    for second in 1..30 {
        state.observe(packet(second * 10_000, true));
        state.observe(packet(5000, false));
        state.tick(second as f64);
    }
    engine.observe(packet(30_000_000, true));
    state.window(&engine.finish(10.0));
    for (width, height) in [(120, 35), (80, 24), (60, 15), (1, 1)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render::draw(frame, &state)).unwrap();
        let buffer = terminal.backend().buffer();
        let rendered: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        if width == 120 {
            assert!(rendered.contains("OUTBOUND / last 120s"));
            assert!(rendered.contains("INBOUND / last 120s"));
            assert!(rendered.contains("198.51.100.77:8443"));
            assert!(rendered.contains("pending"));
        } else if width == 60 {
            assert!(rendered.contains("Enlarge terminal"));
        }
    }
}

#[test]
fn quit_keys_and_release_events() {
    for code in [KeyCode::Char('q'), KeyCode::Esc] {
        assert!(is_quit(&Event::Key(KeyEvent::new(
            code,
            KeyModifiers::NONE
        ))));
    }
    assert!(is_quit(&Event::Key(KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL
    ))));
    assert!(!is_quit(&Event::Key(KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::NONE
    ))));
    assert!(!is_quit(&Event::Key(KeyEvent::new_with_kind(
        KeyCode::Char('q'),
        KeyModifiers::NONE,
        KeyEventKind::Release
    ))));
}

/// A real terminal smoke test, kept out of ordinary CI and the application CLI.
#[tokio::test]
#[ignore = "requires an interactive terminal; run with --ignored --nocapture"]
async fn terminal_session_restores_after_error() {
    use crossterm::terminal::is_raw_mode_enabled;
    assert!(!is_raw_mode_enabled().unwrap());
    let result: anyhow::Result<()> = async {
        let mut ui = Dashboard::new("test-interface", 10, false)?;
        assert!(is_raw_mode_enabled()?);
        let mut engine = Engine::new(Config::default());
        engine.observe(packet(30_000_000, true));
        ui.state.window(&engine.finish(10.0));
        ui.state.observe(packet(30_000_000, true));
        ui.state.observe(packet(3_000_000, false));
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(250));
        let deadline = tokio::time::sleep(std::time::Duration::from_secs(3));
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _ = tick.tick() => ui.refresh()?,
                input = ui.input() => { if input? { break; } },
                _ = &mut deadline => break,
            }
        }
        anyhow::bail!("simulated runtime failure")
    }
    .await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("simulated runtime failure")
    );
    assert!(!is_raw_mode_enabled().unwrap());
}
