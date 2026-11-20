// tracker subfolder and IpPort implementation
#![allow(unknown_lints)]
#![allow(clippy::all)]
#![allow(dead_code)]

pub mod bencode;
pub mod http;
pub mod udp;
pub mod zero;

use std::{
    io::Error,
    net::{Ipv4Addr, SocketAddr, ToSocketAddrs},
};

use decentnet_protocol::address::PeerAddr;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use self::{http::http_announce, udp::udp_announce};

#[derive(Serialize, Deserialize, Copy, Clone)]
pub struct IpPort {
    pub ip: u32,
    pub port: u16,
}

impl IpPort {
    // Takes in byte string of ip:port pairs and parses them.
    pub fn from_bytes(bytes: &[u8]) -> Vec<Self> {
        let mut peers: Vec<IpPort> = vec![];
        if bytes.len() % 6 != 0 {
            return peers;
        }
        for chunk in bytes.chunks(6) {
            peers.push(IpPort {
                // BitTorrent compact peer records use network byte order.
                ip: u32::from_be_bytes(chunk[..4].try_into().unwrap()),
                port: u16::from_be_bytes(chunk[4..6].try_into().unwrap()),
            });
        }
        peers
    }
}

impl std::fmt::Debug for IpPort {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "[ip: {}, port: {}]", Ipv4Addr::from(self.ip), self.port)
    }
}

impl std::fmt::Display for IpPort {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}:{}", Ipv4Addr::from(self.ip), self.port)
    }
}

// Computes info_hash from a site address for BitTorrent-compatible trackers.
pub fn get_info_hash(addr: String) -> [u8; 20] {
    let mut hasher = Sha1::new();
    hasher.update(addr.as_bytes());
    hasher.finalize().into()
}

#[derive(Debug, Clone, Copy)]
pub enum Addr {
    Udp(SocketAddr),
    Http(SocketAddr),
    Zero(SocketAddr),
}

fn resolve_authority(authority: &str) -> Result<SocketAddr, String> {
    if let Ok(mut addresses) = authority.to_socket_addrs() {
        if let Some(address) = addresses.next() {
            return Ok(address);
        }
    }

    // Historical ZeroNet tracker lists sometimes spell IPv6 endpoints as
    // zero://2001:db8::1:15441 (without brackets). Normalize that form only
    // after ordinary host:port parsing has failed.
    if !authority.starts_with('[') {
        if let Some((host, port)) = authority.rsplit_once(':') {
            let port = port
                .parse::<u16>()
                .map_err(|_| format!("invalid tracker port in {authority}"))?;
            if host.contains(':') {
                let normalized = format!("[{host}]:{port}");
                return normalized
                    .to_socket_addrs()
                    .map_err(|error| format!("could not resolve tracker {authority}: {error}"))?
                    .next()
                    .ok_or_else(|| format!("no address resolved for tracker {authority}"));
            }
        }
    }

    Err(format!("could not resolve tracker address: {authority}"))
}

/// Parse an explicit tracker scheme. Bare host:port entries remain HTTP for
/// backwards compatibility with the previous tracker list parser.
pub fn make_addr(tracker: &str) -> Result<Addr, String> {
    let (scheme, address) = tracker.split_once("://").unwrap_or(("http", tracker));
    let authority = address.split('/').next().unwrap_or("").trim();

    if authority.is_empty() {
        return Err("tracker address is empty".to_string());
    }

    let protocol = match scheme {
        "udp" => 0,
        "http" => 1,
        "zero" => 2,
        "https" => return Err("https:// trackers are not implemented".to_string()),
        other => return Err(format!("unsupported tracker scheme: {other}")),
    };

    if scheme == "zero" && authority.contains('#') {
        return Err(
            "zero:// certificate-pinned trackers require TLS support, which is not enabled"
                .to_string(),
        );
    }

    let socket = resolve_authority(authority)?;
    match protocol {
        0 => Ok(Addr::Udp(socket)),
        1 => Ok(Addr::Http(socket)),
        _ => Ok(Addr::Zero(socket)),
    }
}

/// Announce using the selected tracker protocol.
///
/// info_hash is the 20-byte SHA-1 used by BitTorrent HTTP/UDP trackers.
/// site_hash is the 32-byte SHA-256 ZeroNet address hash used by zero://.
pub async fn announce(
    addr: Addr,
    info_hash: [u8; 20],
    site_hash: Vec<u8>,
    port: u16,
    peer_id: &str,
) -> Result<Vec<PeerAddr>, Error> {
    match addr {
        Addr::Zero(address) => zero::zero_announce(address, site_hash, port).await,
        Addr::Http(address) => {
            let peers = http_announce(address, info_hash, port, None, peer_id).await?;
            Ok(peers
                .into_iter()
                .map(|peer| PeerAddr::IPV4(peer.ip.to_be_bytes(), peer.port))
                .collect())
        }
        Addr::Udp(address) => {
            let peers = udp_announce(address, info_hash, port, peer_id).await?;
            Ok(peers
                .into_iter()
                .map(|peer| PeerAddr::IPV4(peer.ip.to_be_bytes(), peer.port))
                .collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_info_hash() {
        let addr = "15UYrA7aXr2Nto1Gg4yWXpY3EAJwafMTNk".to_string();
        let info_hash = get_info_hash(addr);
        assert_eq!(
            hex::encode(info_hash),
            "29d191d7caf351ba054a9cb38e8d8477c19bdd1c"
        );
    }

    #[test]
    fn parses_zero_tracker_as_its_own_protocol() {
        assert!(matches!(
            make_addr("zero://127.0.0.1:15441").unwrap(),
            Addr::Zero(_)
        ));
    }

    #[test]
    fn parses_unbracketed_ipv6_zero_tracker() {
        assert!(matches!(
            make_addr("zero://[::1]:15441").unwrap(),
            Addr::Zero(_)
        ));
        assert!(matches!(
            make_addr("zero://::1:15441").unwrap(),
            Addr::Zero(_)
        ));
    }

    #[test]
    fn keeps_http_and_udp_tracker_variants_distinct() {
        assert!(matches!(
            make_addr("http://127.0.0.1:8080/announce").unwrap(),
            Addr::Http(_)
        ));
        assert!(matches!(
            make_addr("udp://127.0.0.1:6969/announce").unwrap(),
            Addr::Udp(_)
        ));
    }

    #[test]
    fn rejects_unknown_and_unsupported_schemes() {
        assert!(make_addr("ftp://127.0.0.1:21").is_err());
        assert!(make_addr("https://127.0.0.1:443/announce").is_err());
    }

    #[test]
    fn rejects_pinned_zero_trackers_without_tls() {
        assert!(make_addr(
            "zero://tracker.example#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:443"
        )
        .is_err());
    }
}
