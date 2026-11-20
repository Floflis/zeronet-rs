//! Native ZeroNet LAN peer discovery.
//!
//! This service is part of the client core. AnnounceLocal is used only as a
//! wire-compatibility reference; no plugin or Python runtime is required.

use std::{
    collections::HashMap,
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use decentnet_protocol::address::PeerAddr;
use lazy_static::lazy_static;
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use serde_bytes::ByteBuf;
use sha2::{Digest, Sha256};
use tokio::{net::UdpSocket, time};

use crate::core::peer::Peer;

const DISCOVERY_PORT: u16 = 1544;
const MAX_DATAGRAM_SIZE: usize = 8_192;
const MAX_SITES_PER_PACKET: usize = 100;
const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(300);
const PEER_TIMEOUT: Duration = Duration::from_secs(20 * 60);
const QUERY_WINDOW: Duration = Duration::from_millis(900);

lazy_static! {
    static ref LAN_DISCOVERY: Mutex<Option<LocalDiscovery>> = Mutex::new(None);
}

fn is_zero(value: &u64) -> bool {
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
    sites_changed: u64,
    #[serde(skip_serializing_if = "is_empty_sites")]
    sites: Vec<ByteBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct DiscoveryMessage {
    cmd: String,
    params: Params,
    sender: Sender,
}

impl DiscoveryMessage {
    fn new(sender: Sender, command: &str, params: Params) -> Self {
        Self {
            cmd: command.to_owned(),
            params,
            sender,
        }
    }
}

#[derive(Debug, Clone)]
struct KnownPeer {
    address: SocketAddr,
    broadcast_port: u16,
    sites_changed: u64,
    last_seen: Instant,
}

#[derive(Default)]
struct DiscoveryState {
    local_sites: HashMap<Vec<u8>, String>,
    watched_sites: HashMap<Vec<u8>, String>,
    known_peers: HashMap<(IpAddr, String), KnownPeer>,
    peers_by_site: HashMap<String, HashMap<SocketAddr, Instant>>,
    sites_changed: u64,
}

#[derive(Clone)]
struct LocalDiscovery {
    socket: Arc<UdpSocket>,
    sender: Sender,
    state: Arc<Mutex<DiscoveryState>>,
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn site_hash(address: &str) -> Vec<u8> {
    Sha256::digest(address.as_bytes()).to_vec()
}

fn encode_message(message: &DiscoveryMessage) -> io::Result<Vec<u8>> {
    rmp_serde::to_vec_named(message)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn decode_message(bytes: &[u8]) -> Option<DiscoveryMessage> {
    rmp_serde::from_slice(bytes).ok()
}

impl LocalDiscovery {
    fn update_local_sites(&self, addresses: Vec<String>) {
        let mut next_sites = HashMap::new();
        for address in addresses {
            next_sites.insert(site_hash(&address), address);
        }

        let mut state = self.state.lock().expect("LAN discovery state poisoned");
        let changed = state.local_sites.len() != next_sites.len()
            || state
                .local_sites
                .keys()
                .any(|hash| !next_sites.contains_key(hash));

        if changed {
            state.sites_changed = unix_time();
        }
        state.watched_sites.extend(next_sites.clone());
        state.local_sites = next_sites;
    }

    async fn send_message(
        &self,
        destination: SocketAddr,
        command: &str,
        params: Params,
    ) -> io::Result<()> {
        let message = DiscoveryMessage::new(self.sender.clone(), command, params);
        let bytes = encode_message(&message)?;
        self.socket.send_to(&bytes, destination).await?;
        Ok(())
    }

    async fn announce(&self) -> io::Result<()> {
        self.send_message(
            SocketAddr::from((Ipv4Addr::BROADCAST, DISCOVERY_PORT)),
            "discoverRequest",
            Params::default(),
        )
        .await
    }

    async fn request_site_list(&self, peer: &KnownPeer) -> io::Result<()> {
        self.send_message(
            SocketAddr::new(peer.address.ip(), peer.broadcast_port),
            "siteListRequest",
            Params::default(),
        )
        .await
    }

    async fn respond_with_site_list(&self, destination: SocketAddr) -> io::Result<()> {
        let (sites_changed, sites) = {
            let state = self.state.lock().expect("LAN discovery state poisoned");
            (
                state.sites_changed,
                state
                    .local_sites
                    .keys()
                    .cloned()
                    .map(ByteBuf::from)
                    .collect::<Vec<_>>(),
            )
        };

        // Match AnnounceLocal's 100-hash UDP response batches.
        for chunk in sites.chunks(MAX_SITES_PER_PACKET) {
            self.send_message(
                destination,
                "siteListResponse",
                Params {
                    sites_changed,
                    sites: chunk.to_vec(),
                },
            )
            .await?;
        }
        Ok(())
    }

    fn handle_site_list(&self, source: SocketAddr, message: &DiscoveryMessage) {
        if message.sender.port <= 1 || message.sender.peer_id.is_empty() {
            return;
        }

        let mut state = self.state.lock().expect("LAN discovery state poisoned");
        let key = (source.ip(), message.sender.peer_id.clone());
        let peer_address = SocketAddr::new(source.ip(), message.sender.port);

        if let Some(peer) = state.known_peers.get_mut(&key) {
            peer.last_seen = Instant::now();
            peer.sites_changed = message.params.sites_changed;
            peer.address = peer_address;
            peer.broadcast_port = message.sender.broadcast_port;
        } else {
            state.known_peers.insert(
                key,
                KnownPeer {
                    address: peer_address,
                    broadcast_port: message.sender.broadcast_port,
                    sites_changed: message.params.sites_changed,
                    last_seen: Instant::now(),
                },
            );
        }

        for hash in message.params.sites.iter().take(5_000) {
            if let Some(site_address) = state.watched_sites.get(hash.as_ref()).cloned() {
                state
                    .peers_by_site
                    .entry(site_address)
                    .or_default()
                    .insert(peer_address, Instant::now());
            }
        }
    }

    async fn handle_message(&self, source: SocketAddr, message: DiscoveryMessage) {
        if message.sender.service != "zeronet"
            || message.sender.peer_id.is_empty()
            || message.sender.peer_id == self.sender.peer_id
            || message.sender.broadcast_port == 0
        {
            return;
        }

        let reply_address = SocketAddr::new(source.ip(), message.sender.broadcast_port);
        match message.cmd.as_str() {
            "discoverRequest" => {
                let sites_changed = self
                    .state
                    .lock()
                    .expect("LAN discovery state poisoned")
                    .sites_changed;
                let params = Params {
                    sites_changed,
                    sites: Vec::new(),
                };
                if let Err(error) = self
                    .send_message(reply_address, "discoverResponse", params)
                    .await
                {
                    debug!("Could not answer LAN discover request: {error}");
                }
            }
            "discoverResponse" => {
                if message.sender.port <= 1 {
                    return;
                }
                let key = (source.ip(), message.sender.peer_id.clone());
                let (peer, changed) = {
                    let mut state = self.state.lock().expect("LAN discovery state poisoned");
                    match state.known_peers.get_mut(&key) {
                        Some(peer) => {
                            let changed = peer.sites_changed != message.params.sites_changed;
                            peer.last_seen = Instant::now();
                            peer.sites_changed = message.params.sites_changed;
                            peer.address = SocketAddr::new(source.ip(), message.sender.port);
                            peer.broadcast_port = message.sender.broadcast_port;
                            (peer.clone(), changed)
                        }
                        None => {
                            let peer = KnownPeer {
                                address: SocketAddr::new(source.ip(), message.sender.port),
                                broadcast_port: message.sender.broadcast_port,
                                sites_changed: message.params.sites_changed,
                                last_seen: Instant::now(),
                            };
                            state.known_peers.insert(key, peer.clone());
                            (peer, true)
                        }
                    }
                };

                if changed {
                    if let Err(error) = self.request_site_list(&peer).await {
                        debug!("Could not request LAN site list: {error}");
                    }
                }
            }
            "siteListRequest" => {
                if let Err(error) = self.respond_with_site_list(reply_address).await {
                    debug!("Could not answer LAN site-list request: {error}");
                }
            }
            "siteListResponse" => self.handle_site_list(source, &message),
            _ => {}
        }
    }

    fn prune(&self) {
        let now = Instant::now();
        let mut state = self.state.lock().expect("LAN discovery state poisoned");
        state
            .known_peers
            .retain(|_, peer| now.duration_since(peer.last_seen) <= PEER_TIMEOUT);
        for peers in state.peers_by_site.values_mut() {
            peers.retain(|_, last_seen| now.duration_since(*last_seen) <= PEER_TIMEOUT);
        }
        state.peers_by_site.retain(|_, peers| !peers.is_empty());
    }

    async fn run(self) {
        let mut announce_interval = time::interval(ANNOUNCE_INTERVAL);
        let mut maintenance_interval = time::interval(Duration::from_secs(60));
        let mut buffer = [0_u8; MAX_DATAGRAM_SIZE];

        loop {
            tokio::select! {
                _ = announce_interval.tick() => {
                    if let Err(error) = self.announce().await {
                        debug!("LAN discovery broadcast unavailable: {error}");
                    }
                }
                _ = maintenance_interval.tick() => self.prune(),
                received = self.socket.recv_from(&mut buffer) => {
                    match received {
                        Ok((length, source)) => {
                            if let Some(message) = decode_message(&buffer[..length]) {
                                self.handle_message(source, message).await;
                            } else {
                                debug!("Ignoring malformed LAN discovery datagram from {source}");
                            }
                        }
                        Err(error) => {
                            warn!("LAN discovery socket receive failed: {error}");
                            time::sleep(Duration::from_secs(1)).await;
                        }
                    }
                }
            }
        }
    }

    async fn discover_site(&self, site_address: &str) -> Vec<Peer> {
        let hash = site_hash(site_address);
        let known_peers = {
            let mut state = self.state.lock().expect("LAN discovery state poisoned");
            state.watched_sites.insert(hash, site_address.to_owned());
            state.known_peers.values().cloned().collect::<Vec<_>>()
        };

        // A new site query still needs current lists from peers with unchanged revisions.
        for peer in &known_peers {
            let _ = self.request_site_list(peer).await;
        }

        if let Err(error) = self.announce().await {
            debug!("LAN discovery query broadcast unavailable: {error}");
        }
        time::sleep(QUERY_WINDOW).await;

        let state = self.state.lock().expect("LAN discovery state poisoned");
        state
            .peers_by_site
            .get(site_address)
            .map(|peers| {
                peers
                    .keys()
                    .copied()
                    .map(|address| Peer::new(PeerAddr::from(address)))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Start the native LAN discovery service once during client startup.
/// It is best-effort and must never prevent tracker discovery or client startup.
pub async fn start(
    local_sites: Vec<String>,
    peer_id: String,
    fileserver_port: u16,
    revision: u64,
) -> io::Result<()> {
    if LAN_DISCOVERY
        .lock()
        .expect("LAN discovery handle poisoned")
        .is_some()
    {
        set_local_sites(local_sites);
        return Ok(());
    }

    let socket = Arc::new(UdpSocket::bind((Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT)).await?);
    socket.set_broadcast(true)?;

    let mut local_site_map = HashMap::new();
    for address in &local_sites {
        local_site_map.insert(site_hash(address), address.clone());
    }

    let state = Arc::new(Mutex::new(DiscoveryState {
        watched_sites: local_site_map.clone(),
        local_sites: local_site_map,
        known_peers: HashMap::new(),
        peers_by_site: HashMap::new(),
        sites_changed: unix_time(),
    }));

    let service = LocalDiscovery {
        socket,
        sender: Sender {
            service: "zeronet".to_owned(),
            peer_id,
            port: fileserver_port,
            broadcast_port: DISCOVERY_PORT,
            rev: revision,
        },
        state,
    };

    *LAN_DISCOVERY.lock().expect("LAN discovery handle poisoned") = Some(service.clone());
    info!("Native LAN peer discovery listening on UDP {DISCOVERY_PORT}");
    tokio::spawn(service.run());
    Ok(())
}

/// Replace the addresses advertised by the native LAN discovery service.
pub fn set_local_sites(addresses: Vec<String>) {
    if let Some(service) = LAN_DISCOVERY
        .lock()
        .expect("LAN discovery handle poisoned")
        .as_ref()
    {
        service.update_local_sites(addresses);
    }
}

/// Discover LAN peers for one site using the already-running core service.
pub async fn discover(site_address: &str) -> Vec<Peer> {
    let service = LAN_DISCOVERY
        .lock()
        .expect("LAN discovery handle poisoned")
        .clone();
    match service {
        Some(service) => service.discover_site(site_address).await,
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_round_trips_with_zero_net_field_names() {
        let message = DiscoveryMessage::new(
            Sender {
                service: "zeronet".to_owned(),
                peer_id: "-UT3530-local-peer".to_owned(),
                port: 15441,
                broadcast_port: DISCOVERY_PORT,
                rev: 4241,
            },
            "discoverRequest",
            Params::default(),
        );

        let encoded = encode_message(&message).expect("message should encode");
        let decoded = decode_message(&encoded).expect("message should decode");

        assert_eq!(decoded.cmd, "discoverRequest");
        assert_eq!(decoded.sender.service, "zeronet");
        assert_eq!(decoded.sender.peer_id, "-UT3530-local-peer");
        assert_eq!(decoded.sender.port, 15441);
        assert_eq!(decoded.sender.broadcast_port, DISCOVERY_PORT);
    }

    #[test]
    fn site_hash_matches_sha256_address_hash() {
        let address = "1HeLLo4uzjaLetFx6NH3PMwFP3qbRbTf3D";
        assert_eq!(site_hash(address).len(), 32);
        assert_eq!(
            site_hash(address),
            Sha256::digest(address.as_bytes()).to_vec()
        );
    }

    #[test]
    fn malformed_datagrams_are_ignored() {
        assert!(decode_message(b"not messagepack").is_none());
    }

    #[tokio::test]
    async fn core_answers_discover_requests_without_a_plugin() {
        let service_socket = Arc::new(
            UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .expect("bind service socket"),
        );
        let client_socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind client socket");
        let client_address = client_socket.local_addr().expect("client address");

        let service = LocalDiscovery {
            socket: service_socket,
            sender: Sender {
                service: "zeronet".to_owned(),
                peer_id: "-UT3530-rust-core".to_owned(),
                port: 15441,
                broadcast_port: DISCOVERY_PORT,
                rev: 1,
            },
            state: Arc::new(Mutex::new(DiscoveryState {
                sites_changed: 123,
                ..DiscoveryState::default()
            })),
        };

        let request = DiscoveryMessage::new(
            Sender {
                service: "zeronet".to_owned(),
                peer_id: "-UT3530-python-peer".to_owned(),
                port: 15442,
                broadcast_port: client_address.port(),
                rev: 1,
            },
            "discoverRequest",
            Params::default(),
        );

        service.handle_message(client_address, request).await;

        let mut buffer = [0_u8; MAX_DATAGRAM_SIZE];
        let (length, _) =
            time::timeout(Duration::from_secs(1), client_socket.recv_from(&mut buffer))
                .await
                .expect("core should answer promptly")
                .expect("receive response");
        let response = decode_message(&buffer[..length]).expect("valid MessagePack response");

        assert_eq!(response.cmd, "discoverResponse");
        assert_eq!(response.params.sites_changed, 123);
        assert_eq!(response.sender.peer_id, "-UT3530-rust-core");
    }

    #[tokio::test]
    async fn site_list_responses_are_cached_for_watched_sites() {
        let socket = Arc::new(
            UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .expect("bind service socket"),
        );
        let address = "1HeLLo4uzjaLetFx6NH3PMwFP3qbRbTf3D";
        let mut state = DiscoveryState::default();
        state
            .watched_sites
            .insert(site_hash(address), address.to_owned());

        let service = LocalDiscovery {
            socket,
            sender: Sender {
                service: "zeronet".to_owned(),
                peer_id: "-UT3530-rust-core".to_owned(),
                port: 15441,
                broadcast_port: DISCOVERY_PORT,
                rev: 1,
            },
            state: Arc::new(Mutex::new(state)),
        };

        let source: SocketAddr = "192.0.2.10:50000".parse().unwrap();
        let response = DiscoveryMessage::new(
            Sender {
                service: "zeronet".to_owned(),
                peer_id: "-UT3530-remote-peer".to_owned(),
                port: 15442,
                broadcast_port: DISCOVERY_PORT,
                rev: 1,
            },
            "siteListResponse",
            Params {
                sites_changed: 456,
                sites: vec![ByteBuf::from(site_hash(address))],
            },
        );

        service.handle_site_list(source, &response);

        let state = service.state.lock().unwrap();
        let peer_address: SocketAddr = "192.0.2.10:15442".parse().unwrap();
        assert!(state
            .peers_by_site
            .get(address)
            .unwrap()
            .contains_key(&peer_address));
    }
}
