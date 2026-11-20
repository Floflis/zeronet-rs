//! Shared, deterministic view of sites currently served by this node.
use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};

use super::site::models::SiteStorage;

/// One site advertised by a complete ZeroNet tracker announce.
#[derive(Clone)]
pub struct ServedSite {
    pub address: String,
    pub hash: Vec<u8>,
    pub storage: SiteStorage,
}

/// Shared between the UI site controller, peer listener and tracker announcer.
///
/// The UI controller is the source of truth. A BTreeMap keeps full-announce
/// hash order deterministic even though the controllers store sites in a
/// HashMap. Keeping SiteStorage here lets the peer listener load sites added
/// after daemon startup without depending on a startup-only static snapshot.
#[derive(Clone, Default)]
pub struct SiteRegistry {
    sites: Arc<RwLock<BTreeMap<String, ServedSite>>>,
}

impl SiteRegistry {
    /// Replace the active serving set atomically.
    pub fn replace(&self, sites: impl IntoIterator<Item = ServedSite>) {
        let next = sites
            .into_iter()
            .map(|site| (site.address.clone(), site))
            .collect::<BTreeMap<_, _>>();
        *self
            .sites
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = next;
    }

    /// Check whether a site is currently allowed to be served.
    pub fn is_serving(&self, address: &str) -> bool {
        self.sites
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains_key(address)
    }

    /// Return the current serving set in deterministic address order.
    pub fn snapshot(&self) -> Vec<ServedSite> {
        self.sites
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .cloned()
            .collect()
    }

    /// Get persisted site settings for a site that became active after startup.
    pub fn storage_for(&self, address: &str) -> Option<SiteStorage> {
        self.sites
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(address)
            .map(|site| site.storage.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(address: &str, marker: u8) -> ServedSite {
        let mut storage = SiteStorage::default();
        storage.settings.serving = true;
        ServedSite {
            address: address.to_string(),
            hash: vec![marker; 32],
            storage,
        }
    }

    #[test]
    fn replaces_sites_and_returns_a_deterministic_snapshot() {
        let registry = SiteRegistry::default();
        registry.replace([site("1B", 2), site("1A", 1)]);

        assert!(registry.is_serving("1A"));
        assert!(!registry.is_serving("1C"));
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].address, "1A");
        assert_eq!(snapshot[0].hash, vec![1; 32]);
        assert_eq!(snapshot[1].address, "1B");
        assert_eq!(snapshot[1].hash, vec![2; 32]);
        assert!(registry.storage_for("1A").unwrap().settings.serving);

        registry.replace([site("1C", 3)]);
        assert!(!registry.is_serving("1A"));
        assert_eq!(registry.snapshot().len(), 1);
        assert_eq!(registry.snapshot()[0].address, "1C");
    }
}
