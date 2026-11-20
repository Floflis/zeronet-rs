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

### Later: compare ZeroNet implementations

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
