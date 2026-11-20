# 🌈 ZeroNet dependency provenance inventory

> Snapshot: 2026-11-20. This maps the path dependencies in canewsin/zerunet
> branch dev to public repositories and compares them with the Rust crates already
> used by Floflis/zeronet-rs.

## 🌈 Findings

GitHub repository and code searches scoped to the ZeroNetX organization did not
find repositories named zerucrypt, upnp-brunch, zerucontent, sorted_json_keys,
or decentnet_protocol. The zeronet_protocol search result within ZeroNetX was the
client repository, not a separate protocol library.

The relevant Rust libraries are split across decentnetwork and Anshorei. The
ZeroNetX organization does contain ZeroNet-Plugins/AnnounceLocal, which provides
the authoritative compatibility reference for the local discovery message exchange.
It is not a runtime dependency: LAN discovery belongs in the Rust client core.

## 🌈 Path dependency inventory

| Dependency in zerunet | Original manifest path | Public source found | Current target status | Decision |
|---|---|---|---|---|
| zerucrypt | ../zerucrypt; lockfile package 0.1.8; also referenced by nested zerucontent | No exact public repository found under ZeroNetX, decentnetwork, canewsin or Anshorei. Related zeronet_cryptography repositories exist. | Already uses decentnetwork/zeronet_cryptography, branch zeronetx. | Do not transplant blindly. Compare WIF, key generation, message hashing and signing fixtures. |
| upnp_brunch | ../upnp-brunch; lockfile package 0.1.1 | No exact public repository found in the searched namespaces. | Not used by zeronet-rs. | Omit for now. The only call site in zerunet/src/main.rs is inside if false and attempts TCP port 15443 mapping. |
| zeronet_protocol | ../zeronet-protocol | decentnetwork/zeronet_protocol branch decentnet (package 0.2.0); older Anshorei/zeronet_protocol master (0.1.10). | Already uses decentnetwork/zeronet_protocol branch decentnet. | Keep current source. Add protocol and serialization fixtures before changes. |
| zerucontent | ./zerucontent, nested crate version 0.1.0 | decentnetwork/zerucontent branch decentnet. | Already uses the same repository and branch. | No wholesale transplant. Compare content format compatibility. |
| json_filter_sorted | ./json_filter_sorted, nested crate | decentnetwork/sorted_json_keys, package json_filter_sorted 0.1.1, commit c1d87b8841f83001cc8d6bede036e14a040d6b2c, MIT. | Pulled transitively by current zerucontent. | Already present transitively. Preserve deterministic JSON behavior with tests. |
| content_manager | ./content_manager, nested crate | Not an external sibling library; its manifest pulls Diesel from Git. | Target has its own SQLite DbManager. | Reference only. Compare schema/rebuild behavior; do not copy the older incomplete implementation. |

## 🌈 Candidate repositories and revisions

### 🌈 Cryptography

- decentnetwork/zeronet_cryptography: branch zeronetx, commit 905836d019f44ea2d7c5167b7a2fd4f354c6be02, package zeronet_cryptography 0.2.0, WTFPL.
- Anshorei/zeronet_cryptography: branch master, commit a0800465f86c51a8d9b90ff802efcb62e3b63bfd, package zeronet_cryptography 0.1.9, WTFPL.

The Anshorei package is named zeronet_cryptography, not zerucrypt; it is related
code, not proof that it is the exact missing path crate. The current target already
uses the later decentnetwork branch. Its create() API returns a SecretKey and
provides WIF conversion helpers, while zerunet call sites expected a WIF string.
Compare behavior using fixed test vectors rather than function names alone.

### 🌈 Protocol and content

- decentnetwork/zeronet_protocol: branch decentnet, commit c84e2c20552ea3234301325f161df5b5bc8374bf, package zeronet_protocol 0.2.0.
- decentnetwork/decentnet_protocol: lower-level protocol crate used by zeronet_protocol.
- decentnetwork/zerucontent: branch decentnet, commit 18b3c66e74654330023189148ee0c593c9e16589.
- decentnetwork/sorted_json_keys: branch master, commit c1d87b8841f83001cc8d6bede036e14a040d6b2c, MIT.

These are already in the target dependency graph. The next useful work is a set
of cross-version fixtures for content JSON ordering, content signing, address
hashes, MessagePack messages and peer handshakes.

### 🌈 UPnP

- decentnetwork/rust-igd is a generic IGD/UPnP client (igd 0.12.0, MIT), not a drop-in replacement for UPnPBrunch.
- The only upnp_brunch call in zerunet/src/main.rs is disabled by if false and attempts to open/close TCP port 15443.

Do not add an UPnP dependency just to satisfy the old manifest. Revisit only if
automatic NAT port mapping becomes an explicit product requirement.

## 🌈 ZeroNetX AnnounceLocal compatibility reference

- ZeroNetX/ZeroNet-Plugins/AnnounceLocal/AnnounceLocalPlugin.py
- ZeroNetX/ZeroNet-Plugins/AnnounceLocal/BroadcastServer.py
- ZeroNetX/ZeroNet-Plugins/AnnounceLocal/Test/TestAnnounce.py

The observed exchange is:

1. Broadcast discoverRequest to UDP port 1544.
2. Respond with discoverResponse containing sites_changed.
3. Request siteListRequest when the remote site-list revision differs or a new site query needs its current list.
4. Respond with one or more siteListResponse messages, with at most 100 site hashes per datagram.
5. Match SHA-256 address hashes; use returned peers only as candidates and retain the normal ZeroNet handshake.

The Rust implementation is a native core service. It starts with the client,
periodically announces, responds to inbound discovery requests, exchanges site
lists, and maintains a peer cache by site. The Python plugin is a protocol reference
only and is not needed at runtime.

Remaining interoperability work:

- Broadcast on each active IPv4 interface, not only the IPv4 limited-broadcast address.
- Test two Rust peers and a Python-compatible peer on the same LAN.
- Validate UDP reuse and shutdown behavior on Linux, Windows and Android.
- Keep sites_changed synchronized with all site additions and removals.

## 🌈 I2P exclusion

The old zerunet root manifest includes a direct Git dependency on I2P. It is not
needed for this mapping and must not be carried into the target. The current
decentnet_protocol full feature includes Tor but does not enable its optional I2P
feature. No I2P dependency or code is proposed here.

## 🌈 Recommended next sequence

1. Keep the current cryptography, protocol, content and sorted-JSON dependencies; add compatibility fixtures rather than replacing them.
2. Keep upnp_brunch out unless automatic port mapping becomes a requirement.
3. Improve native LAN interoperability with per-interface broadcast and multi-process tests.
4. Revisit the old secp256k1 dependency in the cryptography crate as a separate security/compatibility task.