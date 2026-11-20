//! Native ZeroNet tracker protocol (zero://).
//!
//! ZeroNet trackers are regular ZeroNet peers: they exchange the normal
//! MessagePack handshake and then receive an announce command on the same TCP
//! stream. This is intentionally distinct from BitTorrent HTTP/UDP announce.
//!
//! This implementation uses the existing ZeroNet handshake builder and the
//! same named MessagePack framing as ZeroConnection. It currently supports
//! clear-TCP clearnet tracker endpoints. TLS-pinned endpoints and Tor transport
//! / onion signing are reported as unsupported rather than downgraded.

use std::{
    collections::HashSet,
    io::{self, ErrorKind, Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

use decentnet_protocol::address::PeerAddr;
use serde::{Deserialize, Serialize};
use serde_bytes::ByteBuf;

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

fn parse_announce_response(
    response: AnnounceReply,
    expected_to: usize,
) -> io::Result<Vec<PeerAddr>> {
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

    let mut peers = Vec::new();
    let mut seen = HashSet::new();

    for group in peer_groups {
        // ip4 is the legacy ZeroNet spelling. ipv4 and ipv6 are also emitted
        // by trackers. onion records are decoded when the Tor feature is on.
        for packed in group
            .ip4
            .iter()
            .chain(group.ipv4.iter())
            .chain(group.ipv6.iter())
            .chain(group.onion.iter())
        {
            let peer = PeerAddr::unpack(packed.as_ref())
                .map_err(|error| invalid_data(format!("invalid packed tracker peer: {error}")))?;
            if seen.insert(peer.clone()) {
                peers.push(peer);
            }
        }
    }

    Ok(peers)
}

fn announce_blocking(
    address: SocketAddr,
    site_hash: Vec<u8>,
    port: u16,
) -> io::Result<Vec<PeerAddr>> {
    if site_hash.len() != 32 {
        return Err(invalid_data(format!(
            "ZeroNet site hash must contain 32 bytes, got {}",
            site_hash.len()
        )));
    }

    let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    stream.set_nodelay(true)?;

    // Reuse the project's canonical ZeroNet handshake builder and serializer.
    let mut handshake = crate::net::handshake().1;
    handshake.fileserver_port = port as usize;
    send_request(&mut stream, "handshake", 0, handshake)?;
    read_ack(&mut stream, 0)?;

    let announce = AnnounceRequest {
        hashes: vec![ByteBuf::from(site_hash)],
        onions: Vec::new(),
        port: port as usize,
        // ip4 preserves compatibility with older AnnounceZero trackers.
        // The response parser accepts IPv4, IPv6 and onion encodings.
        need_types: vec!["ip4".into(), "ipv4".into(), "ipv6".into()],
        need_num: 20,
        // The current Rust client has not yet implemented the Python
        // getOpenedServiceTypes() equivalent, so it advertises no service type.
        add: Vec::new(),
    };

    send_request(&mut stream, "announce", 1, announce)?;
    let response: AnnounceReply = rmp_serde::from_read(&mut stream).map_err(invalid_data)?;
    parse_announce_response(response, 1)
}

/// Announce one site's SHA-256 address hash to a native zero:// tracker.
pub async fn zero_announce(
    address: SocketAddr,
    site_hash: Vec<u8>,
    port: u16,
) -> io::Result<Vec<PeerAddr>> {
    tokio::task::spawn_blocking(move || announce_blocking(address, site_hash, port))
        .await
        .map_err(|error| io::Error::other(format!("tracker worker failed: {error}")))?
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
        let result = announce_blocking("127.0.0.1:1".parse().unwrap(), vec![0; 20], 15441);
        assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidData);
    }

    #[test]
    fn parses_multiple_site_groups_and_deduplicates_peers() {
        let packed_peer = ByteBuf::from(vec![8, 8, 8, 8, 0x51, 0x3c]);
        let response = AnnounceReply {
            cmd: "response".into(),
            to: 1,
            peers: Some(vec![
                PeerGroup {
                    ip4: vec![packed_peer.clone()],
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

        let peers = parse_announce_response(response, 1).unwrap();
        assert_eq!(peers.len(), 1);
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
    async fn returns_connection_errors_without_panicking() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);

        let result = zero_announce(address, vec![0x42; 32], 15441).await;
        assert!(result.is_err());
    }
}
