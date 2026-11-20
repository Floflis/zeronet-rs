pub mod lan;
pub mod tracker;

use std::collections::HashSet;

use futures::future::join_all;
use log::*;
use rayon::iter::{IntoParallelRefMutIterator, ParallelIterator};

use decentnet_protocol::{address::PeerAddr, interface::RequestImpl};

use crate::{
    core::{discovery::Discovery, error::Error, peer::Peer, site::Site},
    discovery::tracker::{announce, get_info_hash, make_addr},
    environment::ENV,
    io::utils::load_peers,
    net::Protocol,
};

use self::tracker::IpPort;

#[async_trait::async_trait]
impl Discovery for Site {
    // TODO: Make this return a stream of peers instead of collecting all at once.
    async fn discover(&self) -> Result<Vec<Peer>, Error> {
        info!("Discovering peers");
        let mut res_all = vec![];
        let mut futures = vec![];

        for tracker_addr in ENV.trackers.clone() {
            match make_addr(&tracker_addr) {
                Ok(tracker_addr) => {
                    let info_hash = get_info_hash(self.address().to_string());
                    futures.push(announce(tracker_addr, info_hash, 0, &ENV.peer_id));
                }
                Err(err) => {
                    error!("Error parsing tracker address: {err}");
                }
            }
        }

        let results = join_all(futures).await;
        for res in results {
            match res {
                Ok(peers) => {
                    let mut discovered: Vec<Peer> = peers
                        .into_iter()
                        .filter(|peer| peer.port > 1)
                        .map(|peer: IpPort| Peer::new(PeerAddr::parse(peer.to_string()).unwrap()))
                        .collect();
                    res_all.append(&mut discovered);
                }
                Err(error) => error!("Tracker discovery error: {error:?}"),
            }
        }

        // LAN discovery is deliberately best-effort. Tracker discovery must
        // continue working when broadcast is unavailable or filtered by the OS.
        let site_address = self.address().to_owned();
        let peer_id = ENV.peer_id.clone();
        let fileserver_port = ENV.fileserver_port;
        let revision = ENV.rev as u64;

        match tokio::task::spawn_blocking(move || {
            lan::discover(&site_address, &peer_id, fileserver_port, revision)
        })
        .await
        {
            Ok(Ok(mut lan_peers)) => res_all.append(&mut lan_peers),
            Ok(Err(error)) => debug!("LAN discovery unavailable: {error}"),
            Err(error) => warn!("LAN discovery task failed: {error}"),
        }

        // Trackers, LAN discovery and the local peer list can overlap.
        let mut seen = HashSet::new();
        res_all.retain(|peer| seen.insert(peer.address().to_string()));

        Ok(res_all)
    }
}

impl Site {
    pub async fn find_peers(&mut self) -> Result<Vec<Peer>, Error> {
        let peers = self.discover().await?;
        let mut peers = peers
            .into_iter()
            .chain(
                load_peers()
                    .await
                    .iter()
                    .map(|peer| Peer::new(PeerAddr::parse(peer.to_string()).unwrap())),
            )
            .collect::<Vec<_>>();

        let mut connections = peers
            .par_iter_mut()
            .filter_map(|peer| {
                let res = peer.connect();
                if let Err(error) = &res {
                    error!("Error connecting to peer: {error:?}");
                    let address = peer.clone().address().to_string();
                    error!("{address}");
                    None
                } else {
                    info!("Connection successful to {peer:?}");
                    Some(peer)
                }
            })
            .collect::<Vec<_>>();

        let valid_connections = connections
            .iter_mut()
            .map(|peer| async {
                let res = Protocol::new((*peer).connection_mut().unwrap())
                    .handshake()
                    .await;
                if let Err(error) = res {
                    let address = &peer.address().to_string();
                    error!("Handshake failed with peer {address:?}: {error:?}");
                    None
                } else {
                    Some((*peer).clone())
                }
            })
            .collect::<Vec<_>>();

        let valid_connections = join_all(valid_connections)
            .await
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();

        Ok(valid_connections)
    }
}
