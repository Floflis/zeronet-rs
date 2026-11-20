//! Evidence-based external reachability for the peer TCP listener.
//!
//! A bind address, wildcard listener or outbound connection is not proof that
//! peers can reach this node. Evidence is recorded only after a valid ZeroNet
//! v2 handshake from a globally routable remote source and expires after 15m.
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const EVIDENCE_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AddressFamily {
    Ipv4,
    Ipv6,
}

#[derive(Clone)]
pub struct ReachabilityEvidence {
    // The evidence object is created per bound listener, so proofs do not
    // accidentally transfer to a different bind address or port.
    listener: SocketAddr,
    observed: Arc<Mutex<HashMap<AddressFamily, Instant>>>,
}

impl ReachabilityEvidence {
    pub fn new(listener: SocketAddr) -> Self {
        Self {
            listener,
            observed: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Record a valid inbound handshake only when its source is globally
    /// routable and the accepted socket family matches this listener.
    pub fn record_inbound(&self, remote: IpAddr) -> Option<AddressFamily> {
        self.record_inbound_at(remote, Instant::now())
    }

    fn record_inbound_at(&self, remote: IpAddr, now: Instant) -> Option<AddressFamily> {
        let family = globally_routable_family(remote)?;
        if !self.listener_accepts(family) {
            return None;
        }
        self.observed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(family, now);
        Some(family)
    }

    pub fn is_reachable_for(&self, remote: IpAddr) -> bool {
        self.is_reachable_for_at(address_family(remote), Instant::now())
    }

    fn is_reachable_for_at(&self, family: AddressFamily, now: Instant) -> bool {
        self.observed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&family)
            .is_some_and(|observed| now.saturating_duration_since(*observed) <= EVIDENCE_TTL)
    }

    /// ZeroNet service type names derived only from fresh inbound proof.
    pub fn service_types(&self) -> Vec<String> {
        let now = Instant::now();
        let mut types = Vec::new();
        if self.is_reachable_for_at(AddressFamily::Ipv4, now) {
            types.push("ipv4".to_string());
        }
        if self.is_reachable_for_at(AddressFamily::Ipv6, now) {
            types.push("ipv6".to_string());
        }
        types
    }

    fn listener_accepts(&self, family: AddressFamily) -> bool {
        match self.listener.ip() {
            // A private interface may be reached through a port-forward. Its
            // address is not evidence; the completed public-source handshake is.
            IpAddr::V4(_) => family == AddressFamily::Ipv4,
            IpAddr::V6(address) if address.to_ipv4_mapped().is_some() => {
                family == AddressFamily::Ipv4
            }
            IpAddr::V6(address) if address.is_unspecified() => true,
            IpAddr::V6(_) => family == AddressFamily::Ipv6,
        }
    }
}

fn address_family(address: IpAddr) -> AddressFamily {
    match address {
        IpAddr::V4(_) => AddressFamily::Ipv4,
        IpAddr::V6(address) if address.to_ipv4_mapped().is_some() => AddressFamily::Ipv4,
        IpAddr::V6(_) => AddressFamily::Ipv6,
    }
}

/// Return the public address family represented by an IP address.
pub fn globally_routable_family(address: IpAddr) -> Option<AddressFamily> {
    match address {
        IpAddr::V4(address) if is_globally_routable_ipv4(address) => Some(AddressFamily::Ipv4),
        IpAddr::V6(address) => {
            if let Some(mapped) = address.to_ipv4_mapped() {
                return is_globally_routable_ipv4(mapped).then_some(AddressFamily::Ipv4);
            }
            is_globally_routable_ipv6(address).then_some(AddressFamily::Ipv6)
        }
        _ => None,
    }
}

fn in_ipv4_range(address: Ipv4Addr, network: [u8; 4], prefix: u8) -> bool {
    let mask = u32::MAX << (32 - prefix);
    (u32::from(address) & mask) == (u32::from(Ipv4Addr::from(network)) & mask)
}

fn is_globally_routable_ipv4(address: Ipv4Addr) -> bool {
    // Conservative exclusions for IANA special-purpose, private, shared,
    // benchmarking, documentation, multicast and reserved address space.
    let excluded = [
        ([0, 0, 0, 0], 8),
        ([10, 0, 0, 0], 8),
        ([100, 64, 0, 0], 10),
        ([127, 0, 0, 0], 8),
        ([169, 254, 0, 0], 16),
        ([172, 16, 0, 0], 12),
        ([192, 0, 0, 0], 24),
        ([192, 0, 2, 0], 24),
        ([192, 88, 99, 0], 24),
        ([192, 168, 0, 0], 16),
        ([198, 18, 0, 0], 15),
        ([198, 51, 100, 0], 24),
        ([203, 0, 113, 0], 24),
        ([224, 0, 0, 0], 4),
        ([240, 0, 0, 0], 4),
    ];
    !excluded
        .iter()
        .any(|(network, prefix)| in_ipv4_range(address, *network, *prefix))
}

fn in_ipv6_prefix(address: Ipv6Addr, network: u128, prefix: u32) -> bool {
    let mask = u128::MAX << (128 - prefix);
    (u128::from(address) & mask) == (network & mask)
}

fn is_globally_routable_ipv6(address: Ipv6Addr) -> bool {
    let raw = u128::from(address);
    // Global unicast is 2000::/3. Exclude IETF protocol assignments,
    // documentation and deprecated transition address ranges.
    let global_unicast = (raw & (0xe000_u128 << 112)) == (0x2000_u128 << 112);
    global_unicast
        && !in_ipv6_prefix(address, 0x2000_0000_0000_0000_0000_0000_0000_0000, 23)
        && !in_ipv6_prefix(address, 0x2001_0000_0000_0000_0000_0000_0000_0000, 23)
        && !in_ipv6_prefix(address, 0x2001_0db8_0000_0000_0000_0000_0000_0000, 32)
        && !in_ipv6_prefix(address, 0x2002_0000_0000_0000_0000_0000_0000_0000, 16)
        && !in_ipv6_prefix(address, 0x3fff_0000_0000_0000_0000_0000_0000_0000, 20)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(address: &str) -> IpAddr {
        address.parse().unwrap()
    }

    #[test]
    fn rejects_private_shared_reserved_and_documentation_ipv4() {
        for address in [
            "0.1.2.3",
            "10.1.2.3",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.1.1",
            "172.16.0.1",
            "192.0.0.8",
            "192.0.2.1",
            "192.88.99.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
        ] {
            assert_eq!(globally_routable_family(ip(address)), None, "{address}");
        }
        assert_eq!(
            globally_routable_family(ip("8.8.8.8")),
            Some(AddressFamily::Ipv4)
        );
    }

    #[test]
    fn accepts_global_ipv6_and_rejects_special_ranges() {
        assert_eq!(
            globally_routable_family(ip("2606:4700:4700::1111")),
            Some(AddressFamily::Ipv6)
        );
        for address in [
            "::",
            "::1",
            "fc00::1",
            "fe80::1",
            "2000::1",
            "2001:db8::1",
            "2002::1",
            "3fff::1",
        ] {
            assert_eq!(globally_routable_family(ip(address)), None, "{address}");
        }
    }

    #[test]
    fn treats_ipv4_mapped_ipv6_as_ipv4() {
        assert_eq!(
            globally_routable_family(ip("::ffff:8.8.8.8")),
            Some(AddressFamily::Ipv4)
        );
        assert_eq!(globally_routable_family(ip("::ffff:192.168.1.1")), None);
    }

    #[test]
    fn evidence_is_listener_scoped_family_specific_and_expires() {
        let now = Instant::now();
        let evidence = ReachabilityEvidence::new("0.0.0.0:15441".parse().unwrap());
        assert_eq!(
            evidence.record_inbound_at(ip("8.8.8.8"), now),
            Some(AddressFamily::Ipv4)
        );
        assert!(evidence.is_reachable_for_at(AddressFamily::Ipv4, now));
        assert!(!evidence.is_reachable_for_at(AddressFamily::Ipv6, now));
        assert!(evidence.is_reachable_for_at(AddressFamily::Ipv4, now + EVIDENCE_TTL));
        assert!(!evidence.is_reachable_for_at(
            AddressFamily::Ipv4,
            now + EVIDENCE_TTL + Duration::from_secs(1)
        ));
    }

    #[test]
    fn public_inbound_proof_counts_on_a_private_nat_bound_interface() {
        // A successful connection from a public source can reach a private
        // interface through an explicit port-forward. The bind address alone
        // is not used as evidence either way.
        let evidence = ReachabilityEvidence::new("192.168.1.20:15441".parse().unwrap());
        assert_eq!(
            evidence.record_inbound(ip("8.8.8.8")),
            Some(AddressFamily::Ipv4)
        );
        assert_eq!(evidence.service_types(), vec!["ipv4"]);
    }

    #[test]
    fn records_ipv4_and_ipv6_independently_on_wildcard_listener() {
        let evidence = ReachabilityEvidence::new("[::]:15441".parse().unwrap());
        assert_eq!(
            evidence.record_inbound(ip("8.8.8.8")),
            Some(AddressFamily::Ipv4)
        );
        assert_eq!(
            evidence.record_inbound(ip("2606:4700:4700::1111")),
            Some(AddressFamily::Ipv6)
        );
        assert_eq!(evidence.service_types(), vec!["ipv4", "ipv6"]);
    }
}
