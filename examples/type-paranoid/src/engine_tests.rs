use super::*;

fn event(ip: &str, bytes: u64) -> Observation {
    Observation {
        peer: Peer {
            remote: ip.parse().unwrap(),
            protocol: Protocol::Tcp,
        },
        outbound: true,
        payload_bytes: bytes,
    }
}

#[test]
fn distinguishes_expected_backup_new_upload_and_historical_spike() {
    let mut engine = Engine::new(Config {
        expected: HashSet::from(["192.0.2.1:443".parse().unwrap()]),
        ..Default::default()
    });
    for _ in 0..4 {
        engine.observe(event("192.0.2.2:443", 16_384));
        engine.observe(event("192.0.2.1:443", 30_000_000));
        assert!(engine.finish(10.0).candidates.is_empty());
    }
    engine.observe(event("192.0.2.2:443", 2_000_000));
    engine.observe(event("192.0.2.3:443", 30_000_000));
    engine.observe(event("192.0.2.1:443", 30_000_000));
    let window = engine.finish(10.0);
    assert_eq!(window.candidates.len(), 2);
    assert!(window.candidates[0].signals.contains(&"upload_to_new_peer"));
    assert_eq!(
        window.candidates[1].signals,
        ["outbound_rate_above_baseline"]
    );
    assert!(!window.candidates[1].new_in_retained_history);
    assert_eq!(window.candidates[1].baseline_windows, 4);
    assert!(window.candidates.iter().all(|c| c.process.is_none()));
}

#[test]
fn downloads_are_not_uploads_and_short_windows_use_rates() {
    let mut engine = Engine::new(Config::default());
    let mut download = event("192.0.2.1:443", 50_000_000);
    download.outbound = false;
    engine.observe(download);
    assert!(engine.finish(10.0).candidates.is_empty());
    for _ in 0..3 {
        engine.observe(event("192.0.2.2:443", 1_000_000));
        engine.finish(10.0);
    }
    engine.observe(event("192.0.2.2:443", 300_000));
    assert!(engine.finish(3.0).candidates.is_empty());
}

#[test]
fn bounds_peer_history_and_candidate_memory_and_reports_overflow() {
    let mut engine = Engine::new(Config {
        max_peers: 2,
        max_candidates: 1,
        ..Default::default()
    });
    for ip in ["192.0.2.1:443", "192.0.2.2:443", "192.0.2.3:443"] {
        engine.observe(event(ip, 30_000_000));
    }
    let window = engine.finish(10.0);
    assert_eq!(window.sent_bytes, 90_000_000);
    assert_eq!(window.untracked_packets, 1);
    assert_eq!(window.candidates.len(), 1);
    assert_eq!(window.omitted_candidates, 1);
    assert_eq!(window.history_peers, 2);
    for _ in 0..7 {
        engine.finish(10.0);
    }
    assert!(engine.history.is_empty());
}

#[test]
fn returning_peer_after_history_expiry_is_explicitly_new_to_retained_history() {
    let mut engine = Engine::new(Config::default());
    engine.observe(event("192.0.2.1:443", 1024));
    engine.finish(10.0);
    for _ in 0..6 {
        engine.finish(10.0);
    }
    engine.observe(event("192.0.2.1:443", 30_000_000));
    let window = engine.finish(10.0);
    assert!(window.candidates[0].new_in_retained_history);
    assert_eq!(window.candidates[0].baseline_windows, 0);
}
