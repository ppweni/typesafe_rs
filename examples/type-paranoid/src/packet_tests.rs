use super::*;
use etherparse::PacketBuilder;

fn local() -> HashSet<IpAddr> {
    HashSet::from(["192.0.2.1".parse().unwrap(), "2001:db8::1".parse().unwrap()])
}

fn tcp(source: [u8; 4], destination: [u8; 4], source_port: u16, destination_port: u16) -> Vec<u8> {
    let builder = PacketBuilder::ethernet2([1; 6], [2; 6])
        .ipv4(source, destination, 64)
        .tcp(source_port, destination_port, 0, 1024);
    let mut packet = Vec::new();
    builder.write(&mut packet, &[42; 128]).unwrap();
    packet
}

#[test]
fn maps_both_directions_to_same_peer_and_counts_only_transport_payload() {
    let outgoing = tcp([192, 0, 2, 1], [198, 51, 100, 2], 50000, 443);
    let incoming = tcp([198, 51, 100, 2], [192, 0, 2, 1], 443, 50000);
    let out = decode(&outgoing, &local(), &HashSet::new())
        .unwrap()
        .unwrap();
    let incoming = decode(&incoming, &local(), &HashSet::new())
        .unwrap()
        .unwrap();
    assert_eq!(out.peer, incoming.peer);
    assert_eq!(out.peer.remote, "198.51.100.2:443".parse().unwrap());
    assert!(out.outbound);
    assert!(!incoming.outbound);
    assert_eq!(out.payload_bytes, 128);
    assert_eq!(out.peer.protocol, Protocol::Tcp);
}

#[test]
fn supports_ipv6_udp_without_copying_packet_contents_into_events() {
    let source = "2001:db8::1"
        .parse::<std::net::Ipv6Addr>()
        .unwrap()
        .octets();
    let destination = "2001:db8::2"
        .parse::<std::net::Ipv6Addr>()
        .unwrap()
        .octets();
    let mut packet = Vec::new();
    PacketBuilder::ethernet2([1; 6], [2; 6])
        .ipv6(source, destination, 64)
        .udp(12345, 443)
        .write(&mut packet, &[1; 64])
        .unwrap();
    let event = decode(&packet, &local(), &HashSet::new()).unwrap().unwrap();
    assert_eq!(event.peer.remote, "[2001:db8::2]:443".parse().unwrap());
    assert_eq!(event.payload_bytes, 64);
    assert_eq!(event.peer.protocol, Protocol::Udp);
}

#[test]
fn ignores_transit_self_and_excluded_endpoint_traffic() {
    let excluded = HashSet::from(["198.51.100.2:443".parse().unwrap()]);
    for packet in [
        tcp([192, 0, 2, 1], [198, 51, 100, 2], 12345, 443),
        tcp([198, 51, 100, 2], [192, 0, 2, 1], 443, 12345),
        tcp([192, 0, 2, 1], [192, 0, 2, 1], 12345, 443),
        tcp([198, 51, 100, 2], [198, 51, 100, 3], 12345, 443),
    ] {
        assert!(decode(&packet, &local(), &excluded).unwrap().is_none());
    }
}

#[test]
fn malformed_and_fragmented_packets_do_not_become_flows() {
    assert!(decode(&[0; 3], &local(), &HashSet::new()).is_err());
    let mut packet = tcp([192, 0, 2, 1], [198, 51, 100, 2], 12345, 443);
    packet[20] = 0x20; // IPv4 more-fragments flag (Ethernet header is 14 bytes).
    assert!(decode(&packet, &local(), &HashSet::new()).is_err());
}
