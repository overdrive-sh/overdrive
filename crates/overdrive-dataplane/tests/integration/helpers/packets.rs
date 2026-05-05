//! TCP/IPv4 packet synthesis + checksum helpers for Tier 3 veth tests.
//!
//! Companion to [`super::veth`]. The packet shape mirrors the Tier 2
//! `xdp_service_map_lookup` triptych in
//! `crates/overdrive-bpf/tests/integration/xdp_service_map_lookup.rs` —
//! a minimal Ethernet+IPv4+TCP-SYN frame addressed to `dst_ip:dst_port`,
//! checksums populated via RFC 1071 one's-complement sums so the
//! kernel's `bpf_l3_csum_replace` / `bpf_l4_csum_replace` DELTA-style
//! updates produce a valid post-rewrite checksum.
//!
//! Lives in a shared helper module rather than being re-implemented per
//! Tier 3 test — Slice 02 (this step) and Slice 05 (REVERSE_NAT_MAP)
//! both need the same primitives, and a single source of truth means a
//! checksum bug fix lands once.

#![cfg(target_os = "linux")]
#![allow(clippy::missing_panics_doc)]

/// Ethernet header length (no VLAN).
pub const ETH_HDR_LEN: usize = 14;
/// IPv4 header length (no options, IHL = 5).
pub const IPV4_HDR_LEN: usize = 20;
/// TCP header length (no options, data offset = 5).
pub const TCP_HDR_LEN: usize = 20;
/// Full minimal Ethernet+IPv4+TCP-SYN frame size.
pub const PKT_LEN: usize = ETH_HDR_LEN + IPV4_HDR_LEN + TCP_HDR_LEN;

/// Synthesise a minimal Ethernet+IPv4+TCP-SYN frame addressed to
/// `dst_octets:dst_port` with valid IPv4 and TCP checksums.
///
/// MAC addresses are fixed sentinels (locally-administered unicast on
/// both ends) — they do not participate in the SERVICE_MAP key shape
/// and the post-rewrite assertions ignore them. Source IP is fixed
/// (`10.0.0.100`); source port is `12345`.
pub fn synthesise_tcp_syn(dst_octets: [u8; 4], dst_port: u16) -> Vec<u8> {
    let mut pkt = vec![0u8; PKT_LEN];

    // Ethernet (14B): dst MAC, src MAC, ethertype 0x0800 (IPv4).
    pkt[0..6].copy_from_slice(&[0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
    pkt[6..12].copy_from_slice(&[0x52, 0x54, 0x00, 0xab, 0xcd, 0xef]);
    pkt[12..14].copy_from_slice(&[0x08, 0x00]);

    // IPv4 (20B):
    let ip = ETH_HDR_LEN;
    pkt[ip] = 0x45; // ver=4, IHL=5
    pkt[ip + 1] = 0x00; // TOS
    let total_len: u16 = (IPV4_HDR_LEN + TCP_HDR_LEN) as u16;
    pkt[ip + 2..ip + 4].copy_from_slice(&total_len.to_be_bytes());
    pkt[ip + 4..ip + 6].copy_from_slice(&0u16.to_be_bytes()); // id
    pkt[ip + 6..ip + 8].copy_from_slice(&0u16.to_be_bytes()); // flags+frag
    pkt[ip + 8] = 0x40; // TTL=64
    pkt[ip + 9] = 0x06; // proto=TCP
    pkt[ip + 10..ip + 12].copy_from_slice(&0u16.to_be_bytes()); // checksum
    pkt[ip + 12..ip + 16].copy_from_slice(&[10, 0, 0, 100]); // src IP
    pkt[ip + 16..ip + 20].copy_from_slice(&dst_octets); // dst IP

    // Compute IPv4 header checksum (RFC 1071, header-only).
    let csum = ipv4_header_checksum(&pkt[ip..ip + IPV4_HDR_LEN]);
    pkt[ip + 10..ip + 12].copy_from_slice(&csum.to_be_bytes());

    // TCP (20B):
    let tcp = ip + IPV4_HDR_LEN;
    let src_port: u16 = 12345;
    pkt[tcp..tcp + 2].copy_from_slice(&src_port.to_be_bytes());
    pkt[tcp + 2..tcp + 4].copy_from_slice(&dst_port.to_be_bytes());
    pkt[tcp + 4..tcp + 8].copy_from_slice(&0u32.to_be_bytes()); // seq
    pkt[tcp + 8..tcp + 12].copy_from_slice(&0u32.to_be_bytes()); // ack
    pkt[tcp + 12] = 0x50; // data offset = 5 (no options)
    pkt[tcp + 13] = 0x02; // flags = SYN
    pkt[tcp + 14..tcp + 16].copy_from_slice(&8192u16.to_be_bytes()); // window
    pkt[tcp + 16..tcp + 18].copy_from_slice(&0u16.to_be_bytes()); // checksum
    pkt[tcp + 18..tcp + 20].copy_from_slice(&0u16.to_be_bytes()); // urg ptr

    // Compute TCP checksum over pseudo-header + TCP header.
    let tcp_csum =
        tcp_checksum(&pkt[ip + 12..ip + 16], &pkt[ip + 16..ip + 20], &pkt[tcp..tcp + TCP_HDR_LEN]);
    pkt[tcp + 16..tcp + 18].copy_from_slice(&tcp_csum.to_be_bytes());

    pkt
}

/// RFC 1071 one's-complement checksum over a 20-byte IPv4 header.
/// A header with valid checksum field re-summed via this function
/// returns 0.
#[must_use]
pub fn ipv4_header_checksum(hdr: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i < hdr.len() {
        sum += u32::from(u16::from_be_bytes([hdr[i], hdr[i + 1]]));
        i += 2;
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// RFC 793 + RFC 1071 TCP checksum over IPv4 pseudo-header + TCP
/// header. Caller passes the **rewritten** src/dst IP and TCP segment
/// (header already including any rewritten ports) — recomputing this
/// against a frame with valid checksum returns 0.
#[must_use]
pub fn tcp_checksum(src_ip: &[u8], dst_ip: &[u8], tcp: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    // Pseudo-header: src(4) + dst(4) + zero(1) + proto(1) + tcp_len(2)
    for chunk in [src_ip, dst_ip].iter() {
        for w in chunk.chunks(2) {
            sum += u32::from(u16::from_be_bytes([w[0], w[1]]));
        }
    }
    sum += u32::from(0x0006_u16); // proto = TCP (zero byte + proto byte)
    sum += u32::from(tcp.len() as u16);
    // TCP header / segment.
    let mut i = 0;
    while i + 1 < tcp.len() {
        sum += u32::from(u16::from_be_bytes([tcp[i], tcp[i + 1]]));
        i += 2;
    }
    if i < tcp.len() {
        sum += u32::from(u16::from_be_bytes([tcp[i], 0]));
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    //! Unit-level pinning of the checksum helpers — exercises the
    //! core invariant that recomputing the checksum over a
    //! valid-checksum frame yields zero. Kept tight: the synthesis
    //! function generates a frame; recomputed checksums over that
    //! frame must be zero. A regression that breaks `ipv4_header_
    //! checksum` or `tcp_checksum` (or `synthesise_tcp_syn`) trips
    //! at the assertion, regardless of the kernel-side path.

    use super::*;

    #[test]
    fn synthesised_frame_has_valid_ipv4_checksum() {
        let pkt = synthesise_tcp_syn([10, 0, 0, 1], 8080);
        let csum = ipv4_header_checksum(&pkt[ETH_HDR_LEN..ETH_HDR_LEN + IPV4_HDR_LEN]);
        assert_eq!(csum, 0, "IPv4 header checksum must validate against synthesised frame");
    }

    #[test]
    fn synthesised_frame_has_valid_tcp_checksum() {
        let pkt = synthesise_tcp_syn([10, 0, 0, 1], 8080);
        let tcp = ETH_HDR_LEN + IPV4_HDR_LEN;
        let csum = tcp_checksum(
            &pkt[ETH_HDR_LEN + 12..ETH_HDR_LEN + 16],
            &pkt[ETH_HDR_LEN + 16..ETH_HDR_LEN + 20],
            &pkt[tcp..tcp + TCP_HDR_LEN],
        );
        assert_eq!(csum, 0, "TCP checksum must validate against synthesised frame");
    }
}
