# TODO

## LAN peer discovery and interoperability

The native LAN discovery service is a core networking feature. It must not depend on the plugin subsystem or on a Python runtime.

### Next: validate and complete LAN discovery

- [ ] Add an interoperability integration test using **two real Rust client processes** on the same host/network.
  - [ ] Verify discovery requests and responses across process boundaries.
  - [ ] Verify site-list exchange and that discovered peers proceed through the normal handshake path.
  - [ ] Keep the test deterministic and document any platform/network prerequisites.
- [ ] Implement IPv4 broadcast **per active network interface**, rather than relying only on the limited broadcast address `255.255.255.255`.
  - [ ] Handle interfaces that are down or have no broadcast-capable IPv4 address.
  - [ ] Avoid making one interface failure prevent broadcasts on the others.
- [ ] Validate interoperability with existing **Python ZeroNet peers**.
  - [ ] Check message names, MessagePack field names, payload shapes, site-hash behavior, and packet-size limits against the Python implementation.
  - [ ] Add cross-implementation fixtures or integration tests where practical.
  - [ ] Check same-host UDP port binding behavior when Python and Rust clients run together.
- [ ] Reconcile the LAN peer cache when a peer removes one or more sites.
  - [ ] Correctly handle multi-packet site-list responses (up to 100 site hashes per datagram).
  - [ ] Remove stale peer-to-site associations without dropping entries from later chunks of the same list.
  - [ ] Add regression tests for removals, empty lists, and multi-packet lists.
- [ ] Verify behavior on supported platforms, especially UDP socket binding and interface enumeration on Linux, Windows, and Android.
- [ ] Document remaining limitations and the results of real-network testing. Do not claim full Python parity until it has been demonstrated.

## Tracker interoperability

Keep tracker discovery independent from LAN discovery. Native ZeroNet `zero://` announces use the peer TCP connection and named MessagePack framing; they are distinct from BitTorrent HTTP and UDP announces.

### Native `zero://` protocol

The Python reference implementation is `HelloZeroNet/ZeroNet`'s `plugins/AnnounceZero/AnnounceZeroPlugin.py`. It does **not** use the BitTorrent UDP announce protocol or an HTTP announce endpoint: it opens a normal ZeroNet peer connection to the tracker and sends the `announce` command with a MessagePack request.

- [x] Add an explicit tracker protocol/address variant for `zero://`; never silently route it through the HTTP or BitTorrent UDP implementation.
- [x] Reuse the existing ZeroNet handshake builder and named MessagePack framing for native `announce` over the peer TCP stream. The `announce` command is sent by the Rust tracker module because the current `zeronet_protocol` command enum does not expose it.
- [x] Implement the single-site AnnounceZero request shape and required fields: `hashes`, `onions`, `port`, `need_types`, `need_num`, and `add`.
- [x] Start and supervise the peer TCP listener in the normal daemon startup path. The default `--fileserver_ip=*` now binds all IPv4 interfaces, and accepted peer sockets are handled independently so one connection cannot block new accepts.
- [x] Determine reachable service types for `add` from fresh inbound ZeroNet v2 handshake evidence, scoped by listener address family and expiring after 15 minutes. A local bind or outbound connection alone is not proof of reachability.
  - [ ] Support the optional `delete` behavior used when announcing the complete set of locally served sites (multi-site full announce is not implemented yet).
  - [x] Route native `zero://*.onion` tracker connections through the existing `tor-stream` dependency and a local Tor SOCKS5 proxy, preserving the onion hostname instead of resolving it with system DNS. Default proxy: `127.0.0.1:9050`; override with `ZERONET_TOR_SOCKS`.
- [x] Test the SOCKS5 path with a local proxy fixture that asserts the `.onion` domain is sent as a hostname and completes a handshake/announce exchange through the proxy.
- [x] Keep onion signing (`onion_sign_this` / `onion_signs`) out of scope until local onion-service keys are managed; return an explicit unsupported error if challenged. Do not add I2P.
- [x] Parse and validate response `peers` entries for `ip4`, `ipv4`, `ipv6`, and `onion` byte encodings (where the dependency feature supports them).
- [x] Add Python-generated MessagePack fixtures and tests for successful announces, empty peer sets, malformed responses, connection failures, request IDs, duplicate peers, and multiple site groups.
- [ ] Verify peer insertion, deduplication, source attribution, and normal peer-handshake validation after tracker discovery against a real Python ZeroNet tracker.
- [x] Document current limits: clearnet native trackers use clear TCP and onion trackers use local Tor SOCKS5; certificate-pinned TLS, RSA onion signing, multi-site `delete`, and service-type advertisement are not implemented. Do not claim full compatibility until real-tracker testing.

### Tracker bootstrap and configuration

- [x] Load tracker addresses from `assets/trackers.txt` independently from LAN discovery. The file remains empty by default; no unverified bootstrap endpoint is shipped.
- [x] Validate supported schemes explicitly (`zero://`, `udp://`, and `http://`); reject unsupported `https://`, unknown schemes, malformed hosts/ports, onion URLs outside native `zero://`, and pinned endpoints without TLS.
- [x] Keep the bootstrap list empty until candidate endpoints have been checked for current operation, protocol compatibility, and maintainership. Do not copy historical IPs or onion addresses from old ZeroNet configurations as presumed-live defaults.
- [x] Add bounded TCP connect/read/write timeouts and isolate tracker failures with the existing `join_all` discovery flow. Retry/backoff remain future work.
- [x] Add tests for empty configuration, comments/blank lines, duplicate entries, malformed addresses, unsupported schemes, and mixed tracker protocols.
- [x] Document how operators can supply and update tracker endpoints without recompiling the client.

## IPv6 overlay mesh peer connectivity

- [x] Add configurable IPv4, IPv6 and dual-stack TCP peer listeners.
- [x] Reuse the standard IPv6 TCP transport and `peers_ipv6` exchange for mesh peers.
- [x] Count valid inbound CJDNS (`fc00::/8`) and Yggdrasil (`0200::/7`) handshakes as IPv6 reachability evidence, only when an IPv6 listener is bound.
- [ ] Verify inbound and outbound peer handshakes between two real CJDNS clients.
- [ ] Repeat interoperability validation between two real Yggdrasil clients.
- [ ] Validate dual-stack binding and mesh reachability on Linux, Windows, macOS and Android.

## Later: compare ZeroNet implementations

After LAN interoperability is validated, compare the relevant implementations and versions for:

- [ ] Content addressing and content comparison semantics.
- [ ] Cryptographic key handling, signatures, and signature verification.
- [ ] Wire protocol messages, serialization, and compatibility behavior.
- [ ] Differences in validation rules, edge cases, and error handling.
- [ ] A compatibility matrix backed by shared fixtures and tests.

### Design constraints

- LAN connectivity remains a native core capability, not an optional plugin.
- Do not add I2P support as part of this work.
- Keep tracker discovery available; LAN discovery is an additional peer-discovery path.
- LAN-discovered peers are untrusted candidates and must still pass the normal peer handshake and validation.
