use std::{collections::HashMap, fs::File, io::Read, sync::Arc, time::Duration};

use itertools::Itertools;
use log::*;
use serde_bytes::ByteBuf;
use std::time::Instant;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::Mutex,
};

use decentnet_protocol::{
    address::PeerAddr,
    builders,
    interface::ResponseImpl,
    message::{Request as ZeroNetRequest, ResponseType},
    templates::*,
};
use zeronet_protocol::ZeroConnection;
use zerucontent::Content;

use crate::{
    core::{error::Error, io::SiteIO, peer::Peer},
    environment::ENV,
    net::Protocol,
    SitesController,
};

pub struct ConnectionController {
    listener: Arc<TcpListener>,
    sites_controller: SitesController,
    conn_len: usize,
    connections: HashMap<String, usize>,
}

impl ConnectionController {
    pub async fn new(sites_controller: SitesController) -> Result<Self, Error> {
        let ser_addr = format!("{}:{}", ENV.fileserver_ip, ENV.fileserver_port);
        info!("Listening on {ser_addr}");
        let listener = Arc::new(TcpListener::bind(ser_addr).await?);
        Ok(Self {
            listener,
            sites_controller,
            conn_len: 0,
            connections: HashMap::new(),
        })
    }

    /// Accept peer sessions continuously and dispatch each socket independently.
    ///
    /// Request handling is serialized only while mutating the shared site
    /// registry. A slow or long-lived peer therefore cannot block new accepts.
    pub async fn run(self) -> Result<(), Error> {
        let listener = Arc::clone(&self.listener);
        let controller = Arc::new(Mutex::new(self));

        loop {
            match listener.accept().await {
                Ok((stream, peer_addr)) => {
                    let peer_ip = peer_addr.ip().to_string();
                    {
                        let mut state = controller.lock().await;
                        state.conn_len += 1;
                        *state.connections.entry(peer_ip.clone()).or_insert(0) += 1;
                    }

                    let controller = Arc::clone(&controller);
                    tokio::spawn(async move {
                        if let Err(error) =
                            Self::handle_connection(stream, Arc::clone(&controller)).await
                        {
                            debug!("Peer connection from {peer_addr} closed: {error}");
                        }

                        // Keep the counters accurate even when a peer disconnects
                        // while a request is being processed.
                        let mut state = controller.lock().await;
                        state.conn_len = state.conn_len.saturating_sub(1);
                        let remove_peer = if let Some(count) = state.connections.get_mut(&peer_ip) {
                            *count = count.saturating_sub(1);
                            *count == 0
                        } else {
                            false
                        };
                        if remove_peer {
                            state.connections.remove(&peer_ip);
                        }
                    });
                }
                Err(error) => {
                    // A transient accept error must not silently kill the
                    // long-running peer service.
                    warn!("Peer TCP accept failed; retrying: {error}");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }

    async fn handle_connection(
        stream: TcpStream,
        controller: Arc<Mutex<Self>>,
    ) -> Result<(), Error> {
        let peer_addr = stream.peer_addr()?;
        let stream = stream.into_std()?;
        let reader = Box::new(stream.try_clone()?);
        let writer = Box::new(stream);
        let mut connection = ZeroConnection::new(reader, writer)?;

        loop {
            let request = match connection.recv().await {
                Ok(request) => request,
                Err(error) => {
                    debug!("Could not receive request from {peer_addr}: {error}");
                    break;
                }
            };

            let mut protocol = Protocol::new(&mut connection);
            match request.cmd.as_str() {
                "handshake" => {
                    if let Err(error) = protocol.handshake(request.req_id).await {
                        warn!("Could not send handshake response to {peer_addr}: {error}");
                        break;
                    }
                }
                "ping" => {
                    if let Err(error) = protocol.ping(request.req_id).await {
                        warn!("Could not send ping response to {peer_addr}: {error}");
                        break;
                    }
                }
                _cmd => {
                    debug!(
                        "From {peer_addr}: command={} request_id={}",
                        request.cmd, request.req_id
                    );
                    let time = Instant::now();

                    // Unknown sites can be rejected without entering the shared
                    // site-request path.
                    #[derive(serde::Serialize, serde::Deserialize)]
                    struct SiteRequest {
                        site: String,
                    }

                    if request.body::<SiteRequest>().is_err() {
                        let res = Self::unknown_site_response();
                        let site = request
                            .body::<SiteRequest>()
                            .map(|body| body.site)
                            .unwrap_or_default();
                        debug!("Unknown or malformed site request: {site}");
                        let _ = protocol.0.respond(request.req_id, res).await;
                        debug!(
                            "{} request {} took {:?}",
                            request.cmd,
                            request.req_id,
                            time.elapsed()
                        );
                        continue;
                    }

                    // Only one request mutates the site registry at a time;
                    // socket reads/writes and handshake traffic stay concurrent.
                    let response = {
                        let mut state = controller.lock().await;
                        state.handle_request(request.clone()).await
                    };
                    debug!(
                        "{} request {} took {:?}",
                        request.cmd,
                        request.req_id,
                        time.elapsed()
                    );

                    if let Err(error) = protocol.0.respond(request.req_id, response.clone()).await {
                        warn!("Could not respond to {peer_addr}: {error}");
                        break;
                    }
                    debug!(
                        "Sent response {}",
                        serde_json::to_string(&response).unwrap()
                    );
                }
            }
        }

        Ok(())
    }

    async fn handle_request(&mut self, req: ZeroNetRequest) -> ResponseType {
        match req.cmd.as_str() {
            "pex" => self.handle_pex(req),
            "getHashfield" => self.get_hashfield(req),
            "getFile" => self.handle_get_file(req, false),
            "streamFile" => self.handle_get_file(req, true),
            "update" => self.handle_update(req),
            _ => {
                warn!("Unknown cmd {}", req.cmd);
                ResponseType::UnknownCmd
            }
        }
    }
}

impl ConnectionController {
    fn unknown_site_response() -> ResponseType {
        let res = ErrorResponse {
            error: "Unknown site".to_string(),
        };
        ResponseType::Err(res)
    }

    fn handle_pex(&mut self, req: ZeroNetRequest) -> ResponseType {
        if let Ok(res) = req.body::<Pex>() {
            let site = &res.site;
            let need = res.need;
            if self.sites_controller.sites.contains_key(site) {
                let site = self.sites_controller.sites.get_mut(site).unwrap();
                let mut peers = res.peers;
                if let Some(peers_onion) = res.peers_onion {
                    peers.extend(peers_onion);
                }
                if let Some(peers_ipv6) = res.peers_ipv6 {
                    peers.extend(peers_ipv6);
                }
                let mut pex_peers = vec![];
                for peer in peers {
                    let addr = PeerAddr::unpack(peer.as_slice());
                    if let Ok(addr) = addr {
                        let peer = Peer::new(addr);
                        pex_peers.push(peer);
                    }
                }
                let keys = pex_peers
                    .iter()
                    .map(|p| p.address().to_string())
                    .collect::<Vec<_>>();
                for peer in pex_peers {
                    site.add_peer(peer);
                }
                //TODO! Only send connectable peers instead of all peers
                let mut retrieved = 0;
                let peers = site
                    .peers
                    .iter()
                    .filter_map(|(key, peer)| {
                        if keys.contains(key) {
                            None
                        } else if retrieved < need {
                            retrieved += 1;
                            let packed = peer.clone().address().pack();
                            match peer.address() {
                                PeerAddr::IPV4(_, _) => Some(("ipv4", packed)),
                                PeerAddr::IPV6(_, _) => Some(("ipv6", packed)),
                                PeerAddr::OnionV2(_, _) | PeerAddr::OnionV3(_, _) => {
                                    Some(("onion", packed))
                                }
                            }
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>();
                let mut ip_v4 = vec![];
                let mut ip_v6 = vec![];
                let mut onion = vec![];
                for (key, packed) in peers {
                    match key {
                        "ipv4" => ip_v4.push(ByteBuf::from(packed)),
                        "ipv6" => ip_v6.push(ByteBuf::from(packed)),
                        "onion" => onion.push(ByteBuf::from(packed)),
                        _ => unimplemented!(),
                    }
                }
                ResponseType::Pex(builders::response::pex(ip_v4, ip_v6, onion))
            } else {
                Self::unknown_site_response()
            }
        } else {
            error!("Invalid Pex Request {req:?}");
            ResponseType::InvalidRequest
        }
    }

    fn get_hashfield(&mut self, req: ZeroNetRequest) -> ResponseType {
        if let Ok(res) = req.body::<GetHashfield>() {
            let site = &res.site;
            if self.sites_controller.sites.contains_key(site) {
                unimplemented!();
            } else {
                Self::unknown_site_response()
            }
        } else {
            error!("Invalid GetHashfield Request {req:?}");
            ResponseType::InvalidRequest
        }
    }

    fn handle_get_file(&mut self, req: ZeroNetRequest, streaming: bool) -> ResponseType {
        if let Ok(res) = req.body::<GetFile>() {
            let site = &res.site;
            if self.sites_controller.sites.contains_key(site) {
                let site = self.sites_controller.sites.get_mut(site).unwrap();
                let inner_path = &res.inner_path;
                match site.get_path(inner_path) {
                    Ok(path) => {
                        let location = res.location;
                        let read_bytes = res.read_bytes.unwrap_or(512 * 1024);
                        let file_size = res.file_size;
                        let file = File::open(path).unwrap();
                        #[allow(clippy::unbuffered_bytes)]
                        let bytes = file.bytes();
                        let file_size_actual = bytes.size_hint().1.unwrap();
                        if location > file_size_actual {
                            ResponseType::Err(ErrorResponse {
                                error: "File read error, Bad file location".to_string(),
                            })
                        } else if file_size > file_size_actual {
                            ResponseType::Err(ErrorResponse {
                                error: "File read error, Bad file size".to_string(),
                            })
                        } else if read_bytes > file_size_actual {
                            ResponseType::Err(ErrorResponse {
                                error: "File read error, File size does not match".to_string(),
                            })
                        } else {
                            let bytes = bytes
                                .skip(location)
                                .take(read_bytes)
                                .filter_map(|a| a.ok())
                                .collect_vec();
                            if streaming {
                                ResponseType::StreamFile(
                                    builders::response::stream_file(
                                        bytes.len(),
                                        location,
                                        file_size,
                                    ),
                                    ByteBuf::from(bytes),
                                )
                            } else {
                                ResponseType::GetFile(builders::response::get_file(
                                    ByteBuf::from(bytes),
                                    file_size_actual,
                                    location + read_bytes,
                                ))
                            }
                        }
                    }
                    Err(err) => ResponseType::Err(ErrorResponse {
                        error: format!("{err:?}"),
                    }),
                }
            } else {
                Self::unknown_site_response()
            }
        } else {
            error!("Invalid GetFile Request {req:?}");
            ResponseType::InvalidRequest
        }
    }

    fn handle_update(&mut self, req: ZeroNetRequest) -> ResponseType {
        if let Ok(res) = req.body::<Update>() {
            let site = &res.site;
            if self.sites_controller.sites.contains_key(site) {
                let inner_path = &res.inner_path;
                let content_modified = res.modified;
                if !inner_path.ends_with("content.json") {
                    return ResponseType::Err(ErrorResponse {
                        error: "Only content.json update allowed".to_string(),
                    });
                }
                let validate_content = {
                    let site = self.sites_controller.sites.get(site).unwrap();
                    if !site.inner_content_exists(inner_path) {
                        false
                    } else {
                        let exists_in_includes = site
                            .content(None)
                            .unwrap()
                            .includes
                            .keys()
                            .any(|k| k.ends_with(inner_path));
                        if exists_in_includes {
                            true
                        } else {
                            let site_content_modified =
                                site.content(Some(inner_path)).unwrap().modified.clone();
                            content_modified > Into::<usize>::into(site_content_modified)
                        }
                    }
                };
                if !validate_content {
                    return ResponseType::Ok(OkResponse {
                        ok: "File not changed".to_string(),
                    });
                }
                let body = res.body;
                if body.is_empty() {
                    unimplemented!()
                }
                let content = Content::from_buf(body);
                //TODO!
                if let Ok(_content) = content {
                    let _site = self.sites_controller.sites.get_mut(site).unwrap();
                    ResponseType::Ok(OkResponse {
                        ok: "File updated".to_string(),
                    })
                } else {
                    ResponseType::Err(ErrorResponse {
                        error: "File invalid JSON".to_string(),
                    })
                }
            } else {
                Self::unknown_site_response()
            }
        } else {
            error!("Error Parsing Request Body: Invalid Update Request {req:?}");
            ResponseType::InvalidRequest
        }
    }
}
