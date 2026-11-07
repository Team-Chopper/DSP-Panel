//! Network search for DSPs: every address of a few /24 networks is tried on
//! TCP 9761 with the handshake and the device-info request, then the
//! connection is closed (no disconnect command: if the DSP serves another
//! controller, 0x11 might end that session). By default the networks are the
//! server's own and those of the configured DSPs. A DSP takes one connection
//! at a time, so addresses the panel is connected to are not probed.

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

use dsp_core::protocol::{self as p, Message, Protocol};
use dsp_core::{DeviceModel, FrameDecoder};
use futures_util::StreamExt;
use futures_util::stream;
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{Instant, timeout};

/// TCP port of the DSPs.
pub const DSP_PORT: u16 = 9761;
/// Probes running at the same time.
const PARALLEL: usize = 64;
/// At most this many /24 networks per search.
pub const MAX_NETWORKS: usize = 4;

/// A device that answered.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    pub host: String,
    pub port: u16,
    /// Answer to the device-info request ("DSP408 V0104P"), empty if none.
    pub info: String,
    /// Model id if the answer names a supported model.
    pub model: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct ScanTiming {
    pub connect: Duration,
    pub answer: Duration,
}

impl Default for ScanTiming {
    fn default() -> Self {
        ScanTiming { connect: Duration::from_millis(700), answer: Duration::from_millis(1200) }
    }
}

/// The server's own /24 networks: the one of the address it reaches the
/// internet with (no packet is sent), and on Linux every directly connected
/// /24 from the routing table – a venue LAN often has no default route.
pub fn own_networks() -> Vec<[u8; 3]> {
    let mut list: Vec<[u8; 3]> = default_route_network().into_iter().collect();
    if let Ok(table) = std::fs::read_to_string("/proc/net/route") {
        for net in connected_networks(&table) {
            if !list.contains(&net) {
                list.push(net);
            }
        }
    }
    list
}

fn default_route_network() -> Option<[u8; 3]> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => network_of(ip),
        _ => None,
    }
}

/// Directly connected /24 networks in a Linux /proc/net/route table
/// (destination and mask as little-endian hex, gateway 0).
fn connected_networks(table: &str) -> Vec<[u8; 3]> {
    let hex = |s: &str| u32::from_str_radix(s, 16).ok().map(|v| v.to_le_bytes());
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            let (dest, gateway, mask) = (hex(f.get(1)?)?, hex(f.get(2)?)?, hex(f.get(7)?)?);
            let ip = Ipv4Addr::from(dest);
            (gateway == [0; 4] && mask == [255, 255, 255, 0] && !ip.is_loopback() && !ip.is_unspecified())
                .then(|| [dest[0], dest[1], dest[2]])
        })
        .collect()
}

fn network_of(ip: Ipv4Addr) -> Option<[u8; 3]> {
    let [a, b, c, _] = ip.octets();
    Some([a, b, c])
}

/// Network of a host text ("192.168.1.50"), if it is an IPv4 address.
pub fn network_of_host(host: &str) -> Option<[u8; 3]> {
    host.trim().parse::<Ipv4Addr>().ok().and_then(network_of)
}

/// Parse "192.168.1" / "192.168.1.0" / "192.168.1.0/24" into a network.
pub fn parse_network(text: &str) -> Option<[u8; 3]> {
    let text = text.trim().trim_end_matches("/24");
    let parts: Vec<&str> = text.split('.').collect();
    if !(3..=4).contains(&parts.len()) {
        return None;
    }
    let mut octets = [0u8; 3];
    for (o, part) in octets.iter_mut().zip(&parts) {
        *o = part.parse().ok()?;
    }
    Some(octets)
}

pub fn network_text(net: [u8; 3]) -> String {
    format!("{}.{}.{}.0/24", net[0], net[1], net[2])
}

/// Try every host of [networks] on [port]; hosts in [skip] are left out.
/// Sorted by address.
pub async fn scan(networks: &[[u8; 3]], skip: &HashSet<String>, port: u16, timing: ScanTiming) -> Vec<Found> {
    let hosts: Vec<Ipv4Addr> = networks
        .iter()
        .take(MAX_NETWORKS)
        .flat_map(|n| (1..=254).map(move |h| Ipv4Addr::new(n[0], n[1], n[2], h)))
        .filter(|ip| !skip.contains(&ip.to_string()))
        .collect();
    let mut found: Vec<(Ipv4Addr, Found)> = stream::iter(hosts)
        .map(|ip| async move { probe(ip, port, timing).await.map(|f| (ip, f)) })
        .buffer_unordered(PARALLEL)
        .filter_map(|r| async move { r })
        .collect()
        .await;
    found.sort_by_key(|(ip, _)| *ip);
    found.into_iter().map(|(_, f)| f).collect()
}

/// Connect to one address and ask it who it is. `None` if nothing listens.
pub async fn probe(ip: Ipv4Addr, port: u16, timing: ScanTiming) -> Option<Found> {
    let address = SocketAddr::new(IpAddr::V4(ip), port);
    let mut stream = timeout(timing.connect, TcpStream::connect(address)).await.ok()?.ok()?;
    let mut info = String::new();
    let request = [p::handshake_command(), p::request_device_info()].concat();
    if stream.write_all(&request).await.is_ok() {
        let deadline = Instant::now() + timing.answer;
        let mut decoder = FrameDecoder::new();
        let protocol = Protocol::new(DeviceModel::DSP408);
        let mut buffer = [0u8; 512];
        'read: while let Ok(Ok(n)) = tokio::time::timeout_at(deadline, stream.read(&mut buffer)).await {
            if n == 0 {
                break;
            }
            for frame in decoder.add(&buffer[..n]) {
                if let Some(Message::DeviceInfo { name }) = protocol.decode(&frame) {
                    info = name.trim().to_string();
                    break 'read;
                }
            }
        }
    }
    let _ = stream.shutdown().await;
    let model = DeviceModel::detect(&info).map(|m| m.id.to_string());
    Some(Found { host: ip.to_string(), port, info, model })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn networks() {
        assert_eq!(parse_network("192.168.1.0/24"), Some([192, 168, 1]));
        assert_eq!(parse_network(" 10.21.0 "), Some([10, 21, 0]));
        assert_eq!(parse_network("10.21.0.123"), Some([10, 21, 0]));
        assert_eq!(parse_network("300.1.1"), None);
        assert_eq!(parse_network("dsp.local"), None);
        assert_eq!(network_of_host("10.21.0.123"), Some([10, 21, 0]));
        assert_eq!(network_of_host("dsp.local"), None);
        assert_eq!(network_text([10, 21, 0]), "10.21.0.0/24");
    }

    #[test]
    fn connected_networks_from_the_routing_table() {
        let table = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
            eth0\t00000000\t0100150A\t0003\t0\t0\t0\t00000000\t0\t0\t0\n\
            eth0\t0000150A\t00000000\t0001\t0\t0\t0\t00FFFFFF\t0\t0\t0\n\
            eth1\t0001A8C0\t00000000\t0001\t0\t0\t0\t00FFFFFF\t0\t0\t0\n\
            eth2\t0000A8C0\t00000000\t0001\t0\t0\t0\t0000FFFF\t0\t0\t0\n";
        assert_eq!(connected_networks(table), [[10, 21, 0], [192, 168, 1]]);
    }
}
