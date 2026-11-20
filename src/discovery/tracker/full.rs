//! Periodic complete-site announces for native ZeroNet trackers.
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, RwLock},
    time::Duration,
};

use decentnet_protocol::address::PeerAddr;
use log::{debug, error, info, warn};

use crate::{
    core::site_registry::SiteRegistry,
    discovery::reachability::ReachabilityEvidence,
    discovery::tracker::{
        make_addr,
        zero::{zero_announce_many, zero_announce_many_onion},
        Addr,
    },
    environment::ENV,
};

/// Peer addresses learned for each locally served site from tracker full-syncs.
#[derive(Clone, Default)]
pub struct TrackerPeerCache {
    peers: Arc<RwLock<BTreeMap<String, Vec<PeerAddr>>>>,
}

impl TrackerPeerCache {
    /// Replace the complete cache after a successful tracker refresh.
    pub fn replace(&self, peers: BTreeMap<String, Vec<PeerAddr>>) {
        *self
            .peers
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = peers;
    }

    /// Keep cached peers for active sites if every tracker is temporarily down.
    pub fn retain_active_sites(&self, active: &[String]) {
        let active = active.iter().map(String::as_str).collect::<BTreeSet<_>>();
        self.peers
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|address, _| active.contains(address.as_str()));
    }

    /// Return a snapshot of peers for one site.
    pub fn peers_for(&self, address: &str) -> Vec<PeerAddr> {
        self.peers
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(address)
            .cloned()
            .unwrap_or_default()
    }
}

/// Run one complete announce cycle and atomically publish the resulting peer
/// cache. Only native zero:// trackers receive ZeroNet delete semantics.
pub async fn announce_once(
    registry: &SiteRegistry,
    reachability: &ReachabilityEvidence,
    cache: &TrackerPeerCache,
) -> usize {
    let sites = registry.snapshot();
    // SiteRegistry is address-sorted, so response groups map deterministically
    // back to these addresses.
    let hashes = sites.iter().map(|site| site.hash.clone()).collect::<Vec<_>>();
    let add_types = reachability.service_types();

    let mut merged = sites
        .iter()
        .map(|site| (site.address.clone(), Vec::new()))
        .collect::<BTreeMap<_, _>>();
    let mut successful_trackers = 0usize;

    for configured in ENV.trackers.iter() {
        let tracker = match make_addr(configured) {
            Ok(tracker) => tracker,
            Err(error) => {
                warn!("Skipping invalid tracker {configured}: {error}");
                continue;
            }
        };

        let response = match tracker {
            Addr::Zero(address) => {
                zero_announce_many(
                    address,
                    hashes.clone(),
                    ENV.fileserver_port,
                    add_types.clone(),
                )
                .await
            }
            Addr::ZeroOnion { host, port } => {
                zero_announce_many_onion(
                    host,
                    port,
                    hashes.clone(),
                    ENV.fileserver_port,
                    add_types.clone(),
                )
                .await
            }
            Addr::Http(_) | Addr::Udp(_) => {
                // The full-sync delete flag is ZeroNet-specific. BitTorrent
                // trackers retain their existing per-site announce path.
                debug!("Skipping non-ZeroNet tracker during full announce: {configured}");
                continue;
            }
        };

        match response {
            Ok(groups) if groups.len() == sites.len() => {
                successful_trackers += 1;
                for (site, peers) in sites.iter().zip(groups) {
                    if let Some(target) = merged.get_mut(&site.address) {
                        for peer in peers {
                            if !target.contains(&peer) {
                                target.push(peer);
                            }
                        }
                    }
                }
                info!(
                    "Completed ZeroNet full announce to {configured} for {} served sites (add={add_types:?})",
                    sites.len()
                );
            }
            Ok(groups) => {
                error!(
                    "ZeroNet tracker {configured} returned {} peer groups for {} served sites; discarding the response",
                    groups.len(),
                    sites.len()
                );
            }
            Err(error) => {
                warn!("ZeroNet full announce failed for {configured}: {error}");
            }
        }
    }

    if successful_trackers > 0 {
        // Replace rather than append: this drops peers for sites removed or
        // paused since the last successful complete announce.
        cache.replace(merged);
    } else {
        let active = sites
            .iter()
            .map(|site| site.address.clone())
            .collect::<Vec<_>>();
        cache.retain_active_sites(&active);
    }

    successful_trackers
}

/// Keep the tracker view fresh without making ordinary startup depend on a
/// public tracker being online. A complete announce is sent immediately and
/// then every 15 minutes.
pub async fn run(
    registry: SiteRegistry,
    reachability: ReachabilityEvidence,
    cache: TrackerPeerCache,
) {
    loop {
        announce_once(&registry, &reachability, &cache).await;
        tokio::time::sleep(Duration::from_secs(15 * 60)).await;
    }
}
