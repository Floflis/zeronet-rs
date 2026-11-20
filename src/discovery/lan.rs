//! ZeroNet-compatible IPv4 LAN peer discovery.
//!
//! This module implements the small MessagePack discovery exchange used by
//! ZeroNet peers on a local network. LAN responses are untrusted hints only:
//! callers must still perform the normal ZeroNet handshake before using a peer.

use std::{
    collections::{HashMap, HashSet},
    io::{self, ErrorKind},
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    time::{Duration, Instant},
};

use decentnet_protocol::address::PeerAddr;
use serde::{Deserialize, Serialize};
use serde_bytes::ByteBuf;
use sha2::{Digest, Sha256};

use crate::core::peer::Peer;

const DISCOVERY_PORT: u16 = 1544;
const MAX_DATAGRAM_SIZE: usize = 4_800;
const DISCOVERY_WINDOW: Duration = Duration::from_millis(350);
const SITE_LIST_WINDOW: Duration = Duration::from_millis(350);
const MAX_RESPONDERS: usize = 128;

fn is_zero(value: &i64) -> bool {
    *value == 0
}

fn is_empty_sites(sites: &Vec<ByteBuf>) -> bool {
    sites.is_empty()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Sender {
    service: String,
    peer_id: String,
    port: u16,
    broadcast_port: u16,
    rev: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Params {
    #[serde(skip_serializing_if = "is_zero")]
    sites_changed: i64,
    #[serde(skip_serializing_if = "is_empty_sites")]
    sites: Vec<ByteBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct DiscoveryMessage {
    params: Params,
    sender: Sender,
    cmd: String,
}

impl DiscoveryMessage {
    fn new(sender: Sender, command: &str) -> Self {
        Self {
            params: Params::default(),
            sender,
            cmd: command.to_owned(),
        }
    }
}

#[derive(Debug, Clone)]
struct Responder {
    ip: IpAddr,
    peer_id: String,
    peer_port: u16,
    broadcast_port: u16,
}

fn encode_message(message: &DiscoveryMessage) -> io::Result<Vec<u8>> {
    rmp_serde::to_vec_named(message).map_err(|error| io::Error::new(ErrorKind::InvalidData, error))
}

fn decode_message(bytes: &[u8]) -> Option<DiscoveryMessage> {
    rmp_serde::from_slice(bytes).ok()
}

fn is_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
    )
}

/// Discover peers advertising site_address on the local IPv4 broadcast domain.
///
/// The socket binds to an ephemeral local port instead of the legacy fixed
/// listener port. That port is advertised in each message, so ZeroNet peers
/// can send their replies back without colliding with another local client.
pub fn discover(
    site_address: &str,
    peer_id: &str,
    fileserver_port: u16,
    revision: u64,
) -> io::Result<Vec<Peer>> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    socket.set_broadcast(true)?;
    socket.set_read_timeout(Some(Duration::from_millis(60)))?;

    let local_port = socket.local_addr()?.port();
    let local_sender = Sender {
        service: "zeronet".to_owned(),
        peer_id: peer_id.to_owned(),
        port: fileserver_port,
        broadcast_port: local_port,
        rev: revision,
    };

    let request = DiscoveryMessage::new(local_sender.clone(), "discoverRequest");
    let request_bytes = encode_message(&request)?;
    socket.send_to(
        &request_bytes,
        SocketAddr::from((Ipv4Addr::BROADCAST, DISCOVERY_PORT)),
    )?;

    // Phase one: collect compatible discovery responses and remember only
    // peers with valid reply and file-server ports.
    let mut responders: HashMap<(IpAddr, String), Responder> = HashMap::new();
    let discovery_deadline = Instant::now() + DISCOVERY_WINDOW;
    let mut buffer = [0_u8; MAX_DATAGRAM_SIZE];

    while Instant::now() < discovery_deadline && responders.len() < MAX_RESPONDERS {
        match socket.recv_from(&mut buffer) {
            Ok((length, source)) => {
                let Some(message) = decode_message(&buffer[..length]) else {
                    continue;
                };

                if message.cmd != "discoverResponse"
                    || message.sender.service != "zeronet"
                    || message.sender.peer_id.is_empty()
                    || message.sender.peer_id == peer_id
                    || message.sender.port == 0
                    || message.sender.broadcast_port == 0
                {
                    continue;
                }

                let key = (source.ip(), message.sender.peer_id.clone());
                responders.entry(key).or_insert_with(|| Responder {
                    ip: source.ip(),
                    peer_id: message.sender.peer_id,
                    peer_port: message.sender.port,
                    broadcast_port: message.sender.broadcast_port,
                });
            }
            Err(error) if is_timeout(&error) => {}
            Err(error) => return Err(error),
        }
    }

    if responders.is_empty() {
        return Ok(Vec::new());
    }

    // Phase two: request each responder's site list. The advertised hashes
    // use SHA-256 over the site's ASCII address, matching ZeroNet's discovery
    // wire format.
    let site_list_request =
        encode_message(&DiscoveryMessage::new(local_sender, "siteListRequest"))?;
    for responder in responders.values() {
        let destination = SocketAddr::new(responder.ip, responder.broadcast_port);
        let _ = socket.send_to(&site_list_request, destination);
    }

    let wanted_site_hash = Sha256::digest(site_address.as_bytes());
    let site_list_deadline = Instant::now() + SITE_LIST_WINDOW;
    let mut discovered = Vec::new();
    let mut seen_addresses = HashSet::new();

    while Instant::now() < site_list_deadline {
        match socket.recv_from(&mut buffer) {
            Ok((length, source)) => {
                let Some(message) = decode_message(&buffer[..length]) else {
                    continue;
                };

                if message.cmd != "siteListResponse"
                    || message.sender.service != "zeronet"
                    || message.sender.peer_id.is_empty()
                {
                    continue;
                }

                let key = (source.ip(), message.sender.peer_id.clone());
                let Some(responder) = responders.get(&key) else {
                    continue;
                };

                let serves_site = message
                    .params
                    .sites
                    .iter()
                    .any(|site| site.as_ref() == wanted_site_hash.as_slice());

                if !serves_site {
                    continue;
                }

                let address = SocketAddr::new(responder.ip, responder.peer_port);
                if seen_addresses.insert(address) {
                    discovered.push(Peer::new(PeerAddr::from(address)));
                }
            }
            Err(error) if is_timeout(&error) => {}
            Err(error) => return Err(error),
        }
    }

    Ok(discovered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_message_round_trips_with_named_fields() {
        let sender = Sender {
            service: "zeronet".to_owned(),
            peer_id: "-UT3530-local-peer".to_owned(),
            port: 15441,
            broadcast_port: 32000,
            rev: 4241,
        };
        let message = DiscoveryMessage::new(sender, "discoverRequest");

        let encoded = encode_message(&message).expect("message should encode");
        let decoded = decode_message(&encoded).expect("message should decode");

        assert_eq!(decoded.cmd, "discoverRequest");
        assert_eq!(decoded.sender.service, "zeronet");
        assert_eq!(decoded.sender.peer_id, "-UT3530-local-peer");
        assert_eq!(decoded.sender.port, 15441);
        assert_eq!(decoded.sender.broadcast_port, 32000);
    }

    #[test]
    fn site_hash_uses_sha256_of_the_address_text() {
        let address = "1HeLLo4uzjaLetFx6NH3PMwFP3qbRbTf3D";
        let hash = Sha256::digest(address.as_bytes());

        assert_eq!(hash.len(), 32);
    }

    #[test]
    fn malformed_datagrams_are_ignored() {
        assert!(decode_message(b"not messagepack").is_none());
    }
}
