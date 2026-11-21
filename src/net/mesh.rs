//! IPv6 overlay address classification for CJDNS and Yggdrasil.
//!
//! Both overlays expose ordinary IPv6 sockets. ZeroNet can therefore reuse
//! its existing IPv6 peer transport and peer-exchange wire format.

use std::net::Ipv6Addr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshNetwork {
    Cjdns,
    Yggdrasil,
}

/// Identify CJDNS (fc00::/8) and Yggdrasil (0200::/7) source addresses.
pub fn mesh_network(address: Ipv6Addr) -> Option<MeshNetwork> {
    let first = address.octets()[0];
    if first == 0xFC {
        Some(MeshNetwork::Cjdns)
    } else if (first & 0xFE) == 0x02 {
        // 0200::/7 includes the 0200::/8 and 0300::/8 routed ranges.
        Some(MeshNetwork::Yggdrasil)
    } else {
        None
    }
}

pub fn is_mesh_ipv6(address: &Ipv6Addr) -> bool {
    mesh_network(*address).is_some()
}

#[cfg(test)]
mod tests {
    use super::{is_mesh_ipv6, mesh_network, MeshNetwork};
    use std::net::Ipv6Addr;

    #[test]
    fn identifies_cjdns_addresses() {
        assert_eq!(
            mesh_network("fcab:1234::1".parse::<Ipv6Addr>().unwrap()),
            Some(MeshNetwork::Cjdns)
        );
    }

    #[test]
    fn identifies_yggdrasil_addresses_and_routed_prefixes() {
        assert_eq!(
            mesh_network("200:1234::1".parse::<Ipv6Addr>().unwrap()),
            Some(MeshNetwork::Yggdrasil)
        );
        assert_eq!(
            mesh_network("300:1234::1".parse::<Ipv6Addr>().unwrap()),
            Some(MeshNetwork::Yggdrasil)
        );
    }

    #[test]
    fn excludes_regular_global_and_unique_local_ipv6() {
        assert!(!is_mesh_ipv6(&"2001:db8::1".parse().unwrap()));
        assert!(!is_mesh_ipv6(&"fd12:3456::1".parse().unwrap()));
        assert!(!is_mesh_ipv6(&"fe80::1".parse().unwrap()));
    }
}
