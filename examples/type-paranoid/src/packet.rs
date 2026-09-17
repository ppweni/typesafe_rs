use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
};

use etherparse::{NetSlice, SlicedPacket, TransportSlice};

use crate::engine::{Observation, Peer, Protocol};

/// Copy only the tuple and counters out of the borrowed frame. No payload leaves
/// the capture thread. Fragmented packets are skipped rather than misattributed.
pub fn decode(
    frame: &[u8],
    local: &HashSet<IpAddr>,
    excluded: &HashSet<SocketAddr>,
) -> Result<Option<Observation>, ()> {
    let packet = SlicedPacket::from_ethernet(frame).map_err(|_| ())?;
    if packet.is_ip_payload_fragmented() {
        return Err(());
    }
    let (source, destination) = match packet.net {
        Some(NetSlice::Ipv4(ip)) => (
            IpAddr::V4(ip.header().source_addr()),
            IpAddr::V4(ip.header().destination_addr()),
        ),
        Some(NetSlice::Ipv6(ip)) => (
            IpAddr::V6(ip.header().source_addr()),
            IpAddr::V6(ip.header().destination_addr()),
        ),
        _ => return Ok(None),
    };
    let outbound = match (local.contains(&source), local.contains(&destination)) {
        (true, false) => true,
        (false, true) => false,
        // Ignore transit traffic, loopback, and host-to-itself traffic.
        _ => return Ok(None),
    };
    let (source_port, destination_port, protocol, payload_bytes) = match packet.transport {
        Some(TransportSlice::Tcp(tcp)) => (
            tcp.source_port(),
            tcp.destination_port(),
            Protocol::Tcp,
            tcp.payload().len(),
        ),
        Some(TransportSlice::Udp(udp)) => (
            udp.source_port(),
            udp.destination_port(),
            Protocol::Udp,
            udp.payload().len(),
        ),
        _ => return Ok(None),
    };
    let remote = if outbound {
        SocketAddr::new(destination, destination_port)
    } else {
        SocketAddr::new(source, source_port)
    };
    if excluded.contains(&remote)
        || remote.ip().is_multicast()
        || remote.ip().is_unspecified()
        || remote.ip() == IpAddr::V4(std::net::Ipv4Addr::BROADCAST)
    {
        return Ok(None);
    }
    Ok(Some(Observation {
        peer: Peer { remote, protocol },
        outbound,
        payload_bytes: payload_bytes as u64,
    }))
}

#[cfg(test)]
#[path = "packet_tests.rs"]
mod tests;
