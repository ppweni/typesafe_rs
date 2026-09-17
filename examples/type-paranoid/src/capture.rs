use std::{
    collections::HashSet,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, bail};
use pnet_datalink::{Channel, Config};
use tokio::sync::mpsc::{Sender, error::TrySendError};

use crate::{engine::Observation, packet};

pub struct Capture {
    pub stop: Arc<AtomicBool>,
    pub dropped: Arc<AtomicU64>,
    pub unparsed: Arc<AtomicU64>,
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn list_interfaces() -> Result<()> {
    for interface in pnet_datalink::interfaces() {
        println!(
            "{}\tup={}\tloopback={}\taddresses={:?}",
            interface.name,
            interface.is_up(),
            interface.is_loopback(),
            interface.ips
        );
    }
    Ok(())
}

pub fn start(
    name: &str,
    excluded: HashSet<SocketAddr>,
    tx: Sender<Result<Observation>>,
) -> Result<Capture> {
    let interfaces = pnet_datalink::interfaces();
    let interface = interfaces
        .iter()
        .find(|interface| interface.name == name)
        .with_context(|| format!("interface {name:?} not found; run `type-paranoid interfaces`"))?;
    if !interface.is_up()
        || interface.is_loopback()
        || interface.is_point_to_point()
        || interface.mac.is_none()
    {
        bail!(
            "select an active Ethernet-compatible interface; loopback and VPN/tunnel links are unsupported"
        );
    }
    // Include addresses on all interfaces so host-to-itself traffic isn't classified
    // as a remote upload. Addresses are a startup snapshot.
    let local: HashSet<_> = interfaces
        .iter()
        .flat_map(|i| i.ips.iter().map(|ip| ip.ip()))
        .collect();
    if interface.ips.is_empty() {
        bail!("interface {name:?} has no IP addresses");
    }
    let config = Config {
        read_buffer_size: 1_048_576,
        read_timeout: Some(Duration::from_millis(250)),
        promiscuous: false,
        ..Config::default()
    };
    let Channel::Ethernet(_sender, mut receiver) =
        pnet_datalink::channel(interface, config).context(capture_permission_hint())?
    else {
        bail!("unsupported capture channel");
    };
    let capture = Capture {
        stop: Arc::new(AtomicBool::new(false)),
        dropped: Arc::new(AtomicU64::new(0)),
        unparsed: Arc::new(AtomicU64::new(0)),
    };
    let stop = capture.stop.clone();
    let dropped = capture.dropped.clone();
    let unparsed = capture.unparsed.clone();
    std::thread::Builder::new()
        .name("type-paranoid-capture".into())
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let frame = match receiver.next() {
                    Ok(frame) => frame,
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::TimedOut
                                | std::io::ErrorKind::WouldBlock
                                | std::io::ErrorKind::Interrupted
                        ) =>
                    {
                        continue;
                    }
                    Err(error) => {
                        let _ = tx.blocking_send(Err(error.into()));
                        break;
                    }
                };
                match packet::decode(frame, &local, &excluded) {
                    Ok(Some(event)) => match tx.try_send(Ok(event)) {
                        Ok(()) => {}
                        Err(TrySendError::Full(_)) => {
                            dropped.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(TrySendError::Closed(_)) => break,
                    },
                    Ok(None) => {}
                    Err(()) => {
                        unparsed.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        })
        .context("could not start capture thread")?;
    Ok(capture)
}

fn capture_permission_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "cannot open macOS BPF packet capture: the process needs read/write access to /dev/bpf*. \
         Build with `cargo build -p type-paranoid`, then run the built binary with sudo, \
         for example `sudo ./target/debug/type-paranoid monitor --interface en1`. \
         The capture backend tries many BPF devices; a final 'No such file or directory' \
         can hide earlier permission failures"
    } else if cfg!(target_os = "linux") {
        "cannot open Linux packet capture: the process needs CAP_NET_RAW in its network namespace"
    } else {
        "cannot open packet capture: check native capture-device permissions"
    }
}
