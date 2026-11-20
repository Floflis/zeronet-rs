#![feature(trait_alias)]

pub mod bridge;
pub mod common;
pub mod controllers;
#[allow(unused)]
pub mod core;
pub mod discovery;
pub mod environment;
pub mod io;
pub mod net;
pub mod plugins;
pub mod utils;

use std::path::PathBuf;

use log::*;

use crate::{
    common::*,
    controllers::{connections::ConnectionController, sites::SitesController, *},
    core::{error::Error, io::SiteIO, site::Site, site_registry::SiteRegistry},
    environment::*,
    io::db::DbManager,
    plugins::core::manifest::PluginManifest,
};

#[actix_web::main]
async fn main() -> Result<(), Error> {
    // The bridge owns its stdout stream. Enter it before normal startup so
    // logs, the web server, and the default LAN service cannot corrupt JSONL.
    if matches!((*MATCHES).subcommand(), Some(("bridge", _))) {
        return bridge::run().await;
    }

    let plugins = &*PLUGINS;
    let user_storage = &*USER_STORAGE;
    let site_storage = &*SITE_STORAGE;

    // Start native LAN discovery for every local site with a content file.
    // The service is best-effort and remains part of the core, independent of plugins.
    let local_sites = site_storage
        .keys()
        .filter(|address| {
            ENV.data_path
                .join(address.as_str())
                .join("content.json")
                .is_file()
        })
        .cloned()
        .collect::<Vec<_>>();
    if let Err(error) = discovery::lan::start(
        local_sites,
        ENV.peer_id.clone(),
        ENV.fileserver_port,
        ENV.rev as u64,
    )
    .await
    {
        warn!("Native LAN discovery unavailable: {error}");
    }

    let mut db_manager = DbManager::new();
    let mut user = user_storage.values().next().unwrap().clone();
    let sub_cmd = (*MATCHES).subcommand();
    if let Some((cmd, args)) = sub_cmd {
        if cmd.starts_with("site") || cmd.starts_with("db") {
            if let Some(mut site_args) = args.get_many::<String>("site") {
                let site_addr = site_args.next().unwrap();
                let mut site = Site::new(site_addr, (ENV.data_path.clone()).join(site_addr))?;
                if let Some(storage) = site_storage.get(site_addr).cloned() {
                    site.modify_storage(storage);
                }
                match cmd {
                    "siteFindPeers" | "siteNeedFile" | "siteDownload" | "siteUpdate"
                    | "sitePeerExchange" | "siteFetchChanges" => {
                        add_peers_to_site(&mut site).await?;
                        let mut found_connectable_peer = false;
                        while !found_connectable_peer {
                            if let Some(peer_id) = site.peers.keys().next() {
                                let peer_id = peer_id.clone();
                                let peer = site.peers.values_mut().next().unwrap();
                                let conn = peer.connection_mut().unwrap();
                                let mut protocol = net::Protocol::new(conn);
                                use decentnet_protocol::{interface::RequestImpl, Either};
                                let res = protocol
                                    .get_file(site_addr, "content.json", 0, 0, Some(1))
                                    .await;
                                if let Ok(res) = res {
                                    match res {
                                        Either::Success(_) => {
                                            found_connectable_peer = true;
                                        }
                                        Either::Error(err) => {
                                            if err.error == "Unknown site" {
                                                debug!(
                                                    "Site Not Served by Peer Querying Next Peer"
                                                );
                                            } else {
                                                error!("Unknown Error : {}", err.error);
                                            }
                                            site.peers.remove(&peer_id);
                                        }
                                    }
                                } else {
                                    error!("Communication Error {res:#?}");
                                }
                            }
                        }
                    }
                    _ => {}
                }
                match cmd {
                    "siteCreate" => site_create(&mut user, true).await?,
                    "siteNeedFile" => {
                        let inner_path = site_args.next().unwrap();
                        site_need_file(&mut site, inner_path.into()).await?
                    }
                    "siteDownload" => download_site(&mut site).await?,
                    "siteSign" => {
                        let private_key = if let Some(private_key) = site_args.next() {
                            private_key.to_owned()
                        } else if let Some(key) = user.sites.get(site.address()) {
                            if let Some(key) = key.get_privkey() {
                                key
                            } else {
                                unreachable!("No private key for site");
                            }
                        } else {
                            unreachable!("No private key for site");
                        };
                        site_sign(&mut site, private_key).await?
                    }
                    "siteFileEdit" => {
                        let inner_path = site_args.next().unwrap();
                        site_file_edit(&mut site, inner_path.into()).await?;
                    }
                    "siteUpdate" => {
                        let inner_path = site_args.next().unwrap();
                        site_update(&mut site, Some(inner_path)).await?
                    }
                    "siteVerify" => check_site_integrity(&mut site).await?,
                    "dbRebuild" => rebuild_db(&mut site, &mut db_manager).await?,
                    "dbQuery" => {
                        db_query(&mut site, &mut db_manager, site_args.next().unwrap()).await?
                    }
                    "siteFindPeers" => {
                        let mut connectable_peers =
                            site.peers.values().map(|peer| peer.address().to_string());
                        save_peers(&mut connectable_peers).await;
                    }
                    "sitePeerExchange" => peer_exchange(&mut site).await?,
                    "siteFetchChanges" => fetch_changes(&mut site).await?,
                    _ => {
                        warn!("Unknown command: {cmd}");
                    }
                }
                user.get_site_data(site_addr, true);
                let mut storage = site.storage.clone();
                storage.settings.serving = true;
                site.modify_storage(storage);
                site.save_storage().await?;
            }
        } else if cmd.starts_with("peer") {
            if let Some(mut peer_args) = args.get_many::<String>("peer") {
                let peer = peer_args.next().unwrap();
                info!("{peer:?}");
                match cmd {
                    "peerPing" => peer_ping(peer).await?,
                    _ => {
                        warn!("Unknown command: {cmd}");
                    }
                }
            }
        } else if cmd.starts_with("crypt") {
            match cmd {
                "cryptKeyPair" => {
                    let (priv_key, pub_key) = zeronet_cryptography::create();
                    info!(
                        "Your Private key : {}",
                        zeronet_cryptography::privkey_to_wif(priv_key)
                    );
                    info!("Your Public key : {pub_key}");
                }
                "cryptSign" => {
                    let mut args = args.get_many::<String>("data").unwrap();
                    let data = args.next().unwrap();
                    if let Some(priv_key) = args.next() {
                        match zeronet_cryptography::sign(data.as_str(), priv_key) {
                            Ok(signature) => info!("{signature}"),
                            Err(err) => error!("{err}"),
                        }
                    } else {
                        error!("cryptSign cmd requires private key to sign");
                    };
                }
                "cryptVerify" => {
                    let mut args = args.get_many::<String>("data").unwrap();
                    let data = args.next().unwrap();
                    if let Some(pub_key) = args.next() {
                        if let Some(signature) = args.next() {
                            match zeronet_cryptography::verify(data.as_str(), pub_key, signature) {
                                Ok(_) => info!("Signature Successfully verified"),
                                Err(err) => error!("{err}"),
                            }
                        } else {
                            error!("cryptVerify cmd requires signature as arg to verify");
                        }
                    } else {
                        error!("cryptVerify cmd requires Public key & Signature as args");
                    };
                }
                _ => {
                    warn!("Unknown command: {cmd}");
                }
            }
        } else if cmd.starts_with("plugin") {
            match cmd {
                "pluginSign" => {
                    let mut args = args.get_many::<String>("name").unwrap();
                    let name = args.next().unwrap();
                    let manifest = PluginManifest::load(name).await;
                    let manifest_path = PathBuf::from(format!("plugins/{name}/manifest.json"));
                    if manifest.is_ok() {
                        if let Some(private_key) = args.next() {
                            let manifest =
                                manifest.unwrap().sign_plugin(private_key).await.unwrap();
                            let contents = serde_json::to_string_pretty(&manifest).unwrap();
                            tokio::fs::write(manifest_path, contents).await?;
                        } else {
                            error!("pluginSign cmd requires private key to sign");
                        }
                    } else {
                        error!("pluginSign cmd requires private key to sign");
                    };
                }
                "pluginVerify" => {
                    let mut args = args.get_many::<String>("name").unwrap();
                    let name = args.next().unwrap();
                    let manifest = PluginManifest::load(name).await.unwrap();
                    let verified = manifest.verify_plugin().await.unwrap_or(false);
                    println!("Plugin Verified : {verified}");
                }
                _ => {
                    warn!("Unknown command: {cmd}");
                }
            }
        } else {
            match cmd {
                "getConfig" => info!("{}", serde_json::to_string_pretty(&client_info())?),
                _ => {
                    warn!("Unknown command: {cmd}");
                }
            }
        }
    } else if false {
        let conn = DbManager::connect_db_from_path(&ENV.data_path.join("content.db"))?;
        db_manager.insert_connection("content_db", conn);
        let mut controller = SitesController::new(db_manager);
        controller
            .extend_sites_from_sitedata(site_storage.clone())
            .await;
        let con = ConnectionController::new(controller).await?;
        let _ = con.run().await;
    } else {
        info!("Loaded : {} Plugins.", plugins.len());

        // The UI controller is authoritative for the currently served sites.
        // The peer listener uses its own loaded-site map, but both read this
        // shared registry so pause/resume/add/delete are reflected live.
        let site_registry = SiteRegistry::default();
        let tracker_peer_cache = discovery::tracker::full::TrackerPeerCache::default();
        let user_controller = users::run().unwrap();
        let sites_controller = sites::run_with_registry(site_registry.clone())
            .await
            .unwrap();

        let mut peer_sites_controller = SitesController::new(DbManager::new());
        peer_sites_controller
            .extend_sites_from_sitedata(site_storage.clone())
            .await;

        match ConnectionController::new_with_registry(
            peer_sites_controller,
            site_registry.clone(),
            tracker_peer_cache.clone(),
        )
        .await
        {
            Ok(peer_listener) => {
                info!(
                    "Peer TCP listener started on {}:{}",
                    ENV.fileserver_ip, ENV.fileserver_port
                );

                // Full announces are ZeroNet-specific and run independently
                // of the HTTP/UDP BitTorrent tracker discovery path.
                let reachability = peer_listener.reachability_evidence();
                tokio::spawn(discovery::tracker::full::run(
                    site_registry,
                    reachability,
                    tracker_peer_cache,
                ));

                tokio::spawn(async move {
                    if let Err(error) = peer_listener.run().await {
                        error!("Peer TCP listener stopped: {error}");
                    }
                });
            }
            Err(error) => {
                warn!("Peer TCP listener could not start: {error}");
            }
        }

        use plugins::site_server::server;
        let _ = server::run(sites_controller, user_controller).await;
    }
    Ok(())
}
