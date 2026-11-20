//! Route-based discovery for IPv6 overlay networks.
//!
//! CJDNS and Yggdrasil expose ordinary IPv6 sockets to applications. We probe
//! their prefixes with UDP connect(), which asks the kernel for a route and
//! source address without transmitting an application datagram.

use std::net::{IpAddr, Ipv6Addr, SocketAddr, UdpSocket};

const CJDNS_ROUTE_PROBE: &str = "fc00::1";
const YGGDRASIL_ROUTE_PROBE: &str = "200::1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshNetwork {
    Cjdns,
    Yggdrasil,
}

/// Identify the overlay from its IPv6 allocation.
///
/// CJDNS uses fc00::/8. Yggdrasil uses 0200::/7, which also includes the
/// 0300::/8 routed subnet prefix documented by the Yggdrasil project.
pub fn mesh_network(address: Ipv6Addr) -> Option<MeshNetwork> {
    let first = address.octets()[0];
    if first == 0xFC {
        Some(MeshNetwork::Cjdns)
    } else if (first & 0xFE) == 0x02 {
        Some(MeshNetwork::Yggdrasil)
    } else {
        None
    }
}

pub fn is_mesh_ipv6(address: &Ipv6Addr) -> bool {
    mesh_network(*address).is_some()
}

/// Return local IPv6 source addresses for mesh routes installed by the OS.
///
/// A successful UDP connect does not send a packet. It only asks the local
/// routing table which source address would be used for the destination.
pub fn discover_mesh_ipv6_addresses() -> Vec<Ipv6Addr> {
    let probes = [CJDNS_ROUTE_PROBE, YGGDRASIL_ROUTE_PROBE];
    let mut addresses = Vec::new();

    for probe in probes {
        let Ok(destination) = probe.parse::<Ipv6Addr>() else {
            continue;
        };
        let Ok(socket) = UdpSocket::bind("[::]:0") else {
            continue;
        };
        if socket
            .connect(SocketAddr::new(IpAddr::V6(destination), 9))
            .is_err()
        {
            continue;
        }

        let Ok(SocketAddr::V6(source)) = socket.local_addr() else {
            continue;
        };
        let address = *source.ip();
        if is_mesh_ipv6(&address) && !addresses.contains(&address) {
            addresses.push(address);
        }
    }

    addresses
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
    fn does_not_classify_ordinary_ula_or_global_ipv6_as_mesh() {
        assert!(!is_mesh_ipv6(&"fd12:3456::1".parse().unwrap()));
        assert!(!is_mesh_ipv6(&"2001:db8::1".parse().unwrap()));
    }
}
