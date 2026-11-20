//! Native ZeroNet tracker protocol (zero://).
//!
//! ZeroNet trackers are regular ZeroNet peers: they exchange the normal
//! MessagePack handshake and then receive an announce command on the same TCP
//! stream. This is intentionally distinct from BitTorrent HTTP/UDP announce.
//!
//! This implementation uses the existing ZeroNet handshake builder and the
//! same named MessagePack framing as ZeroConnection. Clearnet trackers use
//! clear TCP; .onion trackers use the configured local Tor SOCKS5 proxy.
//! Certificate-pinned TLS and onion RSA signing remain unsupported.

use std::{
    collections::HashSet,
    io::{self, ErrorKind, Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

use decentnet_protocol::address::PeerAddr;
use serde::{Deserialize, Serialize};
use serde_bytes::ByteBuf;
use tor_stream::TorStream;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const IO_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Serialize)]
struct ZeroRequest<'a, T> {
    cmd: &'a str,
    req_id: usize,
    params: T,
}

#[derive(Deserialize)]
struct ReplyAck {
    cmd: String,
    to: usize,
}

#[derive(Serialize, Deserialize)]
struct AnnounceRequest {
    hashes: Vec<ByteBuf>,
    onions: Vec<String>,
    port: usize,
    need_types: Vec<String>,
    need_num: usize,
    add: Vec<String>,
    /// Reconcile this peer's complete tracker state. Omitted on ordinary
    /// single-site discovery announces for backwards compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    delete: Option<bool>,
}

#[derive(Deserialize)]
struct AnnounceReply {
    cmd: String,
    to: usize,
    #[serde(default)]
    peers: Option<Vec<PeerGroup>>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    onion_sign_this: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PeerGroup {
    ip4: Vec<ByteBuf>,
    ipv4: Vec<ByteBuf>,
    ipv6: Vec<ByteBuf>,
    onion: Vec<ByteBuf>,
}

fn invalid_data(message: impl ToString) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, message.to_string())
}

fn send_request<W, T>(writer: &mut W, cmd: &'static str, req_id: usize, params: T) -> io::Result<()>
where
    W: Write,
    T: Serialize,
{
    let request = ZeroRequest {
        cmd,
        req_id,
        params,
    };

    // ZeroNet messages are consecutive named MessagePack objects, with no
    // length prefix. This is the same framing used by ZeroConnection.
    rmp_serde::encode::write_named(writer, &request).map_err(invalid_data)?;
    writer.flush()
}

fn read_ack<R: Read>(reader: &mut R, expected_to: usize) -> io::Result<()> {
    let reply: ReplyAck = rmp_serde::from_read(reader).map_err(invalid_data)?;
    if reply.cmd != "response" || reply.to != expected_to {
        return Err(invalid_data(format!(
            "unexpected ZeroNet handshake response: cmd={}, to={}",
            reply.cmd, reply.to
        )));
    }
    Ok(())
}

fn parse_announce_response_groups(
    response: AnnounceReply,
    expected_to: usize,
    expected_groups: usize,
) -> io::Result<Vec<Vec<PeerAddr>>> {
    if response.cmd != "response" || response.to != expected_to {
        return Err(invalid_data(format!(
            "unexpected ZeroNet announce response: cmd={}, to={}",
            response.cmd, response.to
        )));
    }

    if let Some(error) = response.error {
        return Err(invalid_data(format!("ZeroNet tracker error: {error}")));
    }

    // Python AnnounceZero can request an RSA proof for onion addresses. This
    // client does not advertise onion services yet, so surface the challenge.
    if response.onion_sign_this.is_some() {
        return Err(io::Error::new(
            ErrorKind::Unsupported,
            "tracker requested onion signing, which is not enabled",
        ));
    }

    let peer_groups = response
        .peers
        .ok_or_else(|| invalid_data("ZeroNet tracker response is missing peers"))?;

    // Compatible trackers differ on the all-empty shape: some return
    // peers=[], others return one empty map per requested hash. A partially
    // populated group list is ambiguous and must not be zipped to site hashes.
    if peer_groups.is_empty() {
        return Ok((0..expected_groups).map(|_| Vec::new()).collect());
    }

    if peer_groups.len() != expected_groups {
        return Err(invalid_data(format!(
            "ZeroNet tracker returned {} peer groups for {} requested site hashes",
            peer_groups.len(),
            expected_groups
        )));
    }

    let mut grouped_peers = Vec::with_capacity(peer_groups.len());
    for group in peer_groups {
        let mut peers = Vec::new();
        let mut seen = HashSet::new();

        // ip4 is the legacy ZeroNet spelling. ipv4 and ipv6 are also emitted
        // by trackers. Onion records are decoded when the Tor feature is on.
        for packed in group
            .ip4
            .iter()
            .chain(group.ipv4.iter())
            .chain(group.ipv6.iter())
            .chain(group.onion.iter())
        {
            let peer = PeerAddr::unpack(packed.as_ref())
                .map_err(|error| invalid_data(format!("invalid packed tracker peer: {error}")))?;
            // The same peer may serve several sites, so deduplicate only
            // inside the current site group.
            if seen.insert(peer.clone()) {
                peers.push(peer);
            }
        }
        grouped_peers.push(peers);
    }

    Ok(grouped_peers)
}

fn parse_announce_response(
    response: AnnounceReply,
    expected_to: usize,
) -> io::Result<Vec<PeerAddr>> {
    let mut groups = parse_announce_response_groups(response, expected_to, 1)?;
    Ok(groups.pop().unwrap_or_default())
}

fn validate_site_hash(site_hash: &[u8]) -> io::Result<()> {
    if site_hash.len() != 32 {
        return Err(invalid_data(format!(
            "ZeroNet site hash must contain 32 bytes, got {}",
            site_hash.len()
        )));
    }
    Ok(())
}

/// Shared ZeroNet handshake/announce exchange for direct and Tor streams.
fn announce_stream<S: Read + Write>(
    stream: &mut S,
    site_hashes: Vec<Vec<u8>>,
    port: u16,
    add_types: Vec<String>,
    delete: Option<bool>,
    handshake_port_opened: Option<bool>,
) -> io::Result<Vec<Vec<PeerAddr>>> {
    for site_hash in &site_hashes {
        validate_site_hash(site_hash)?;
    }

    // Reuse the project's canonical ZeroNet handshake builder and serializer.
    let mut handshake = crate::net::handshake().1;
    handshake.fileserver_port = port as usize;
    handshake.port_opened = handshake_port_opened;
    send_request(stream, "handshake", 0, handshake)?;
    read_ack(stream, 0)?;

    let announce = AnnounceRequest {
        hashes: site_hashes.into_iter().map(ByteBuf::from).collect(),
        onions: Vec::new(),
        port: port as usize,
        // ip4 preserves compatibility with older AnnounceZero trackers.
        need_types: vec!["ip4".into(), "ipv4".into(), "ipv6".into()],
        add: add_types,
        need_num: 20,
        delete,
    };
    let expected_groups = announce.hashes.len();

    send_request(stream, "announce", 1, announce)?;
    let response: AnnounceReply = rmp_serde::from_read(stream).map_err(invalid_data)?;
    parse_announce_response_groups(response, 1, expected_groups)
}

fn announce_blocking(
    address: SocketAddr,
    site_hashes: Vec<Vec<u8>>,
    port: u16,
    add_types: Vec<String>,
    delete: Option<bool>,
) -> io::Result<Vec<Vec<PeerAddr>>> {
    for site_hash in &site_hashes {
        validate_site_hash(site_hash)?;
    }

    let tracker_family_type = match address.ip() {
        std::net::IpAddr::V4(_) => "ipv4",
        std::net::IpAddr::V6(ip) if ip.to_ipv4_mapped().is_some() => "ipv4",
        std::net::IpAddr::V6(_) => "ipv6",
    };
    let port_opened = Some(
        add_types
            .iter()
            .any(|service| service == tracker_family_type),
    );

    let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    stream.set_nodelay(true)?;

    announce_stream(
        &mut stream,
        site_hashes,
        port,
        add_types,
        delete,
        port_opened,
    )
}

fn connect_tor_tracker(target: &str) -> io::Result<TorStream> {
    // The dependency defaults to 127.0.0.1:9050. Allow deployments with a
    // non-default Tor SOCKS5 listener to configure it without recompiling.
    match std::env::var("ZERONET_TOR_SOCKS") {
        Ok(proxy) => {
            let proxy = proxy.parse().map_err(|error| {
                io::Error::new(
                    ErrorKind::InvalidInput,
                    format!("invalid ZERONET_TOR_SOCKS address: {error}"),
                )
            })?;
            TorStream::connect_with_address(proxy, target)
        }
        Err(std::env::VarError::NotPresent) => TorStream::connect(target),
        Err(error) => Err(io::Error::other(format!(
            "could not read ZERONET_TOR_SOCKS: {error}"
        ))),
    }
}

fn announce_onion_blocking(
    host: String,
    tracker_port: u16,
    site_hashes: Vec<Vec<u8>>,
    client_port: u16,
    add_types: Vec<String>,
    delete: Option<bool>,
) -> io::Result<Vec<Vec<PeerAddr>>> {
    for site_hash in &site_hashes {
        validate_site_hash(site_hash)?;
    }

    // Keep the .onion hostname intact. The SOCKS5 proxy must resolve it;
    // resolving it locally would leak the lookup and fail for onion services.
    let target = format!("{host}:{tracker_port}");
    let mut stream = connect_tor_tracker(&target)?;
    stream.get_ref().set_read_timeout(Some(IO_TIMEOUT))?;
    stream.get_ref().set_write_timeout(Some(IO_TIMEOUT))?;
    stream.get_ref().set_nodelay(true)?;

    // A Tor tracker route does not establish which public IP family is
    // reachable. Do not derive port_opened from the tracker transport.
    announce_stream(
        &mut stream,
        site_hashes,
        client_port,
        add_types,
        delete,
        None,
    )
}

/// Announce a complete set of site hashes through a native clearnet tracker.
/// delete=true is safe here because the hashes are a complete serving snapshot.
pub async fn zero_announce_many(
    address: SocketAddr,
    site_hashes: Vec<Vec<u8>>,
    port: u16,
    add_types: Vec<String>,
) -> io::Result<Vec<Vec<PeerAddr>>> {
    tokio::task::spawn_blocking(move || {
        announce_blocking(address, site_hashes, port, add_types, Some(true))
    })
    .await
    .map_err(|error| io::Error::other(format!("tracker worker failed: {error}")))?
}

/// Onion counterpart of the complete native ZeroNet tracker announce.
pub async fn zero_announce_many_onion(
    host: String,
    tracker_port: u16,
    site_hashes: Vec<Vec<u8>>,
    client_port: u16,
    add_types: Vec<String>,
) -> io::Result<Vec<Vec<PeerAddr>>> {
    tokio::task::spawn_blocking(move || {
        announce_onion_blocking(
            host,
            tracker_port,
            site_hashes,
            client_port,
            add_types,
            Some(true),
        )
    })
    .await
    .map_err(|error| io::Error::other(format!("Tor tracker worker failed: {error}")))?
}

/// Announce one site's SHA-256 hash to a native ZeroNet tracker over Tor.
pub async fn zero_announce_onion(
    host: String,
    tracker_port: u16,
    site_hash: Vec<u8>,
    client_port: u16,
) -> io::Result<Vec<PeerAddr>> {
    let mut groups = tokio::task::spawn_blocking(move || {
        announce_onion_blocking(
            host,
            tracker_port,
            vec![site_hash],
            client_port,
            Vec::new(),
            None,
        )
    })
    .await
    .map_err(|error| io::Error::other(format!("Tor tracker worker failed: {error}")))??;
    Ok(groups.pop().unwrap_or_default())
}

/// Announce one site's SHA-256 address hash to a native zero:// tracker.
pub async fn zero_announce(
    address: SocketAddr,
    site_hash: Vec<u8>,
    port: u16,
) -> io::Result<Vec<PeerAddr>> {
    let mut groups = tokio::task::spawn_blocking(move || {
        announce_blocking(address, vec![site_hash], port, Vec::new(), None)
    })
    .await
    .map_err(|error| io::Error::other(format!("tracker worker failed: {error}")))??;
    Ok(groups.pop().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::TcpListener, thread};

    const PYTHON_RESPONSE_FIXTURE: &str =
        include_str!("fixtures/zero_announce_response.msgpack.hex");
    const PYTHON_EMPTY_RESPONSE_FIXTURE: &str =
        include_str!("fixtures/zero_announce_empty.msgpack.hex");

    fn fixture(hex: &str) -> Vec<u8> {
        hex::decode(hex.trim()).expect("valid MessagePack hex fixture")
    }

    #[derive(Deserialize)]
    struct IncomingHandshake {
        cmd: String,
        req_id: usize,
    }

    #[derive(Deserialize)]
    struct IncomingAnnounce {
        cmd: String,
        req_id: usize,
        params: AnnounceRequest,
    }

    #[test]
    fn parses_python_announce_response_fixture() {
        let bytes = fixture(PYTHON_RESPONSE_FIXTURE);
        let response: AnnounceReply = rmp_serde::from_slice(&bytes).unwrap();
        let peers = parse_announce_response(response, 1).unwrap();

        assert_eq!(peers.len(), 2);
        assert_eq!(peers[0].to_string(), "8.8.8.8:15441");
        assert_eq!(peers[1].to_string(), "[2001:db8::1]:15441");
    }

    #[test]
    fn accepts_empty_peer_set() {
        let bytes = fixture(PYTHON_EMPTY_RESPONSE_FIXTURE);
        let response: AnnounceReply = rmp_serde::from_slice(&bytes).unwrap();
        assert!(parse_announce_response(response, 1).unwrap().is_empty());
    }

    #[test]
    fn rejects_malformed_messagepack() {
        assert!(rmp_serde::from_slice::<AnnounceReply>(&[0xc1]).is_err());
    }

    #[test]
    fn rejects_missing_peers_wrong_request_id_and_tracker_errors() {
        let missing = AnnounceReply {
            cmd: "response".into(),
            to: 1,
            peers: None,
            error: None,
            onion_sign_this: None,
        };
        assert_eq!(
            parse_announce_response(missing, 1).unwrap_err().kind(),
            ErrorKind::InvalidData
        );

        let wrong_id = AnnounceReply {
            cmd: "response".into(),
            to: 2,
            peers: Some(Vec::new()),
            error: None,
            onion_sign_this: None,
        };
        assert_eq!(
            parse_announce_response(wrong_id, 1).unwrap_err().kind(),
            ErrorKind::InvalidData
        );

        let tracker_error = AnnounceReply {
            cmd: "response".into(),
            to: 1,
            peers: None,
            error: Some("temporarily unavailable".into()),
            onion_sign_this: None,
        };
        assert_eq!(
            parse_announce_response(tracker_error, 1)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidData
        );
    }

    #[test]
    fn rejects_invalid_site_hash_length() {
        let result = announce_blocking(
            "127.0.0.1:1".parse().unwrap(),
            vec![0; 20],
            15441,
            Vec::new(),
            None,
        );
        assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidData);
    }

    #[test]
    fn preserves_per_site_groups_and_deduplicates_only_within_each_group() {
        let packed_peer = ByteBuf::from(vec![8, 8, 8, 8, 0x51, 0x3c]);
        let response = AnnounceReply {
            cmd: "response".into(),
            to: 1,
            peers: Some(vec![
                PeerGroup {
                    ip4: vec![packed_peer.clone(), packed_peer.clone()],
                    ipv4: vec![packed_peer.clone()],
                    ..PeerGroup::default()
                },
                PeerGroup {
                    ipv4: vec![packed_peer],
                    ..PeerGroup::default()
                },
            ]),
            error: None,
            onion_sign_this: None,
        };

        let groups = parse_announce_response_groups(response, 1, 2).unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].len(), 1);
        assert_eq!(groups[1].len(), 1);
        assert_eq!(groups[0][0], groups[1][0]);
    }

    #[test]
    fn accepts_all_empty_compatibility_response_but_rejects_partial_groups() {
        let empty = AnnounceReply {
            cmd: "response".into(),
            to: 1,
            peers: Some(Vec::new()),
            error: None,
            onion_sign_this: None,
        };
        let groups = parse_announce_response_groups(empty, 1, 3).unwrap();
        assert_eq!(groups, vec![Vec::new(), Vec::new(), Vec::new()]);

        let partial = AnnounceReply {
            cmd: "response".into(),
            to: 1,
            peers: Some(vec![PeerGroup::default()]),
            error: None,
            onion_sign_this: None,
        };
        assert_eq!(
            parse_announce_response_groups(partial, 1, 2)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidData
        );
    }

    #[tokio::test]
    async fn speaks_zero_handshake_then_announce_over_tcp() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let fixture = fixture(PYTHON_RESPONSE_FIXTURE);

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let handshake: IncomingHandshake =
                rmp_serde::from_read(&mut stream).expect("handshake request");
            assert_eq!(handshake.cmd, "handshake");
            assert_eq!(handshake.req_id, 0);

            let ack = serde_json::json!({"cmd": "response", "to": 0});
            rmp_serde::encode::write_named(&mut stream, &ack).unwrap();

            let announce: IncomingAnnounce =
                rmp_serde::from_read(&mut stream).expect("announce request");
            assert_eq!(announce.cmd, "announce");
            assert_eq!(announce.req_id, 1);
            assert_eq!(announce.params.hashes.len(), 1);
            assert_eq!(announce.params.hashes[0].len(), 32);
            assert_eq!(announce.params.need_num, 20);
            stream.write_all(&fixture).unwrap();
        });

        let site_hash = vec![0x42; 32];
        let peers = zero_announce(address, site_hash, 15441).await.unwrap();
        server.join().unwrap();

        assert_eq!(peers.len(), 2);
        assert_eq!(peers[0].to_string(), "8.8.8.8:15441");
    }

    #[tokio::test]
    async fn routes_onion_tracker_through_socks5_without_local_dns() {
        let tracker_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let tracker_address = tracker_listener.local_addr().unwrap();
        let socks_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let socks_address = socks_listener.local_addr().unwrap();
        let fixture = fixture(PYTHON_RESPONSE_FIXTURE);

        let tracker = thread::spawn(move || {
            let (mut stream, _) = tracker_listener.accept().unwrap();
            let handshake: IncomingHandshake =
                rmp_serde::from_read(&mut stream).expect("handshake request");
            assert_eq!(handshake.cmd, "handshake");

            let ack = serde_json::json!({"cmd": "response", "to": 0});
            rmp_serde::encode::write_named(&mut stream, &ack).unwrap();

            let announce: IncomingAnnounce =
                rmp_serde::from_read(&mut stream).expect("announce request");
            assert_eq!(announce.cmd, "announce");
            stream.write_all(&fixture).unwrap();
        });

        // This local SOCKS5 test proxy verifies that the onion name is sent
        // as a domain (ATYP=3), not resolved by the client before connecting.
        let socks = thread::spawn(move || {
            let (mut client, _) = socks_listener.accept().unwrap();

            let mut greeting = [0u8; 2];
            client.read_exact(&mut greeting).unwrap();
            assert_eq!(greeting[0], 5);
            let mut methods = vec![0u8; greeting[1] as usize];
            client.read_exact(&mut methods).unwrap();
            assert!(methods.contains(&0));
            client.write_all(&[5, 0]).unwrap();

            let mut request = [0u8; 4];
            client.read_exact(&mut request).unwrap();
            assert_eq!(request, [5, 1, 0, 3]);

            let mut host_len = [0u8; 1];
            client.read_exact(&mut host_len).unwrap();
            let mut host = vec![0u8; host_len[0] as usize];
            client.read_exact(&mut host).unwrap();
            assert_eq!(String::from_utf8(host).unwrap(), "tracker.example.onion");

            let mut target_port = [0u8; 2];
            client.read_exact(&mut target_port).unwrap();
            assert_eq!(u16::from_be_bytes(target_port), 15441);

            let mut remote = TcpStream::connect(tracker_address).unwrap();
            client.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).unwrap();

            let mut client_reader = client.try_clone().unwrap();
            let mut remote_writer = remote.try_clone().unwrap();
            let upstream = thread::spawn(move || io::copy(&mut client_reader, &mut remote_writer));
            let _ = io::copy(&mut remote, &mut client);
            drop(client);
            let _ = upstream.join();
        });

        let previous_proxy = std::env::var_os("ZERONET_TOR_SOCKS");
        std::env::set_var("ZERONET_TOR_SOCKS", socks_address.to_string());
        let result = zero_announce_onion(
            "tracker.example.onion".to_string(),
            15441,
            vec![0x42; 32],
            15441,
        )
        .await;
        match previous_proxy {
            Some(value) => std::env::set_var("ZERONET_TOR_SOCKS", value),
            None => std::env::remove_var("ZERONET_TOR_SOCKS"),
        }

        let peers = result.unwrap();
        tracker.join().unwrap();
        socks.join().unwrap();
        assert_eq!(peers.len(), 2);
    }

    #[tokio::test]
    async fn returns_connection_errors_without_panicking() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);

        let result = zero_announce(address, vec![0x42; 32], 15441).await;
        assert!(result.is_err());
    }
    #[tokio::test]
    async fn full_announce_sends_complete_ordered_hashes_and_delete_flag() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let handshake: IncomingHandshake =
                rmp_serde::from_read(&mut stream).expect("handshake request");
            assert_eq!(handshake.cmd, "handshake");
            assert_eq!(handshake.req_id, 0);

            let ack = serde_json::json!({"cmd": "response", "to": 0});
            rmp_serde::encode::write_named(&mut stream, &ack).unwrap();

            let announce: IncomingAnnounce =
                rmp_serde::from_read(&mut stream).expect("announce request");
            assert_eq!(announce.cmd, "announce");
            assert_eq!(announce.req_id, 1);
            assert_eq!(announce.params.hashes.len(), 2);
            assert_eq!(announce.params.hashes[0].as_ref(), &[0x11; 32]);
            assert_eq!(announce.params.hashes[1].as_ref(), &[0x22; 32]);
            assert_eq!(announce.params.delete, Some(true));
            assert_eq!(announce.params.add, vec!["ipv4"]);
            assert_eq!(announce.params.need_types, vec!["ip4", "ipv4", "ipv6"]);

            let packed = vec![8, 8, 8, 8, 0x51, 0x3c];
            let response = serde_json::json!({
                "cmd": "response",
                "to": 1,
                "peers": [
                    {"ipv4": [packed]},
                    {}
                ]
            });
            let bytes = rmp_serde::to_vec_named(&response).unwrap();
            stream.write_all(&bytes).unwrap();
        });

        let groups = zero_announce_many(
            address,
            vec![vec![0x11; 32], vec![0x22; 32]],
            15441,
            vec!["ipv4".into()],
        )
        .await
        .unwrap();
        server.join().unwrap();

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].len(), 1);
        assert!(groups[1].is_empty());
        assert_eq!(groups[0][0].to_string(), "8.8.8.8:15441");
    }

    #[tokio::test]
    async fn full_announce_can_delete_previous_peer_when_no_sites_are_served() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let empty_response = fixture(PYTHON_EMPTY_RESPONSE_FIXTURE);
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _: IncomingHandshake =
                rmp_serde::from_read(&mut stream).expect("handshake request");
            let ack = serde_json::json!({"cmd": "response", "to": 0});
            rmp_serde::encode::write_named(&mut stream, &ack).unwrap();

            let announce: IncomingAnnounce =
                rmp_serde::from_read(&mut stream).expect("announce request");
            assert!(announce.params.hashes.is_empty());
            assert_eq!(announce.params.delete, Some(true));
            stream.write_all(&empty_response).unwrap();
        });

        let groups = zero_announce_many(address, Vec::new(), 15441, Vec::new())
            .await
            .unwrap();
        server.join().unwrap();
        assert!(groups.is_empty());
    }
}
