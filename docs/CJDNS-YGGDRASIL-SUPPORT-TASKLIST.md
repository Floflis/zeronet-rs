# CJDNS and Yggdrasil support: implementation task list

Status: planning and verification backlog  
Scope: native Rust networking in zeronet-rs and interoperability with ZirohNeth  
Related work: PR #8 (dual-stack listeners, IPv6 mesh route discovery and reachability evidence); ZirohNeth PR #15 (IPv6 route detection).

This is a task list, not a claim that these items are implemented. Keep each item open until code, tests and runtime evidence support closing it.

## Goals and invariants

- Treat CJDNS and Yggdrasil as IPv6 routing overlays; retain existing ZeroNet TCP, handshake, tracker and PEX protocols.
- Preserve IPv4 and ordinary public IPv6.
- Separate route availability, outbound connectivity, active inbound listener and verified inbound reachability.
- Never advertise inbound reachability based solely on address classification, local route or successful bind.
- Do not let peers from an unavailable overlay cause unbounded retries.
- Keep native Rust behavior aligned with ZirohNeth during migration.

## P0 - CI and implementation hygiene

- [ ] Run rustfmt and commit formatting corrections. PR #8 CI reported cargo fmt -- --check failures in connections.rs, environment.rs and net/mod.rs. Format and verify the complete workspace.
- [ ] Triage Clippy failures. The PR #8 job reports warnings denied in files outside the PR diff, including site server, websocket and sites controller code. Separate pre-existing warnings from regressions; fix or explicitly scope them without blanket suppression.
- [ ] Triage dependency security audit. Review advisories affecting memmap2 and secp256k1 plus unmaintained crates. Upgrade, replace, mitigate or document each finding with reason and owner.
- [ ] Keep mesh tests in CI: address classification, listener setup, handshake-derived evidence, expiry and tracker/PEX behavior.
- [ ] Test listener bind failure. An unsuccessful IPv6 bind must not leave the node advertising an IPv6 endpoint.

## P1 - Dynamic overlay route and interface lifecycle

- [ ] Add runtime route/interface discovery. The mesh module classifies CJDNS/Yggdrasil prefixes but does not provide a complete runtime route inventory/reconciliation loop.
- [ ] Track route identity and interface scope where exposed by the OS. Do not hard-code a scope ID for global or overlay IPv6 addresses.
- [ ] Detect route additions/removals and metric changes. Prefer native OS notifications, with bounded polling fallback where required.
- [ ] Handle overlay daemon start, stop, restart and interface replacement without process restart.
- [ ] Reconcile outbound peer eligibility. Dial CJDNS peers only when a usable CJDNS route exists, and likewise for Yggdrasil. Preserve ordinary IPv6.
- [ ] Handle dual-overlay nodes with independent route state.
- [ ] Invalidate stale route eligibility immediately after route loss. Keep route eligibility distinct from inbound reachability evidence.
- [ ] Test route flapping and concurrency: rapid transitions, simultaneous IPv4/IPv6 listeners, active transfers, cancellation and shutdown.

## P1 - Reachability evidence and tracker semantics

- [ ] Preserve handshake-derived reachability: advertise inbound IPv6 only after a valid ZeroNet v2 handshake on an active IPv6 listener.
- [ ] Review evidence binding. Ensure evidence is associated with the correct address family/listener and cannot be created by an IPv4-mapped connection or unrelated socket.
- [ ] Review evidence expiry/refresh: TTL, monotonic time, stale-entry cleanup and revalidation after listener/route changes.
- [ ] Define route-loss behavior. Decide whether overlay route loss invalidates inbound evidence immediately or only outbound selection; document and test.
- [ ] Verify tracker full-announce fields represent actual active/reachable endpoints and remain compatible with existing ZeroNet trackers.
- [ ] Test false-positive cases: route present/listener absent; listener bound/no handshake evidence; stale evidence; listener restart; overlay route loss.
- [ ] Verify independent family operation. IPv4 must continue if IPv6 bind/overlay setup fails, and vice versa.

## P1 - Overlay-aware peer discovery and selection

- [ ] Add route-aware peer filtering using mesh classification plus runtime route state.
- [ ] Keep overlay identity separate from generic IPv6. Do not assume every IPv6 address is CJDNS/Yggdrasil or that every node can reach every mesh prefix.
- [ ] Preserve protocol compatibility. Continue using peers_ipv6 unless a separately reviewed protocol change is justified; do not invent incompatible tracker fields just to label overlays.
- [ ] Test PEX with mixed peers: CJDNS-only, Yggdrasil-only, dual-overlay, public IPv6 and IPv4.
- [ ] Bound retries. Defer/filter peers whose overlay route is unavailable, with backoff.
- [ ] Verify IPv6 serialization/parsing through tracker responses, PEX, persistence, logging and socket conversion.

## P2 - DHT and local discovery

- [ ] Audit native DHT address-family support end-to-end: IPv6 UDP endpoints, bootstrap, serialization, response handling and peer storage.
- [ ] Implement IPv6 DHT only with end-to-end coverage. If any layer is IPv4-only, document it and stage the implementation rather than claiming mesh DHT support.
- [ ] Preserve IPv4 DHT bootstrap and lookup behavior.
- [ ] Evaluate IPv6 local discovery with multicast, interface scoping, hop limits, duplicate suppression and privacy controls.
- [ ] Do not assume multicast crosses overlays. Use discovery mechanisms actually routed by CJDNS/Yggdrasil.

## P2 - Cross-platform and interoperability testing

- [ ] Linux: dual-stack/IPv6-only sockets, IPV6_V6ONLY, route notifications, CJDNS/Yggdrasil lifecycle and multiple interfaces.
- [ ] Windows: socket binding, interface/route notifications, IPv6 scope handling and overlay client compatibility.
- [ ] macOS: listener behavior, route updates, interface indexes and overlay lifecycle.
- [ ] Android where supported: document deployment and test VPN/overlay routing, process lifecycle and background restrictions.
- [ ] CJDNS two-node integration: inbound/outbound TCP, ZeroNet handshake, tracker announce, PEX and file transfer.
- [ ] Yggdrasil two-node integration: same complete path.
- [ ] Mixed-overlay integration: test CJDNS-to-Yggdrasil only where an actual inter-overlay route exists; otherwise verify filtering/defer behavior.
- [ ] Mixed address-family integration: IPv4, public IPv6 and mesh IPv6 concurrently.
- [ ] Network-transition integration: add/remove routes during active sessions and verify reconnect, backoff, evidence expiry and tracker reannounce.
- [ ] Record reproducible topology: overlay versions, OS/kernel versions, interface/route setup, node configuration and commands. Never publish private keys or sensitive node data.

## P2 - ZirohNeth bridge and migration parity

- [ ] Define a shared networking capability contract: route availability, outbound capability, listener state, reachability evidence, overlay classification and peer eligibility.
- [ ] Expose dynamic network-state events across the bridge, or define Rust as the sole owner of this state consistently.
- [ ] Prevent double ownership: Python and Rust must not both bind the same listener, duplicate endpoint announcements or maintain conflicting peer eligibility.
- [ ] Keep fallback explicit. Python fallback must work when Rust is unavailable, without duplicate announce/connect loops.
- [ ] Add cross-language parity fixtures for CJDNS, Yggdrasil, public IPv6 and IPv4 addresses/routes.
- [ ] Update migration findings/handoff docs for listener, tracker announce, PEX, outgoing peer connections, DHT and downloads.

## Acceptance criteria

CJDNS/Yggdrasil support in zeronet-rs is not complete until:
1. Runtime route/interface changes update outbound peer eligibility.
2. IPv4/IPv6 listeners remain independent and recover from bind/overlay failures.
3. Inbound reachability remains handshake-derived, correctly scoped and time-bounded.
4. Tracker announcements and PEX expose only endpoints consistent with listener/reachability policy.
5. Peer selection respects CJDNS/Yggdrasil routes without breaking generic IPv6.
6. DHT/local-discovery gaps are implemented end-to-end or explicitly excluded from the peer-transport completion claim.
7. CI passes tests, formatting and quality gates; security advisories have explicit disposition.
8. Two-node overlay integration tests pass on supported platforms.
9. ZirohNeth and Rust share a documented, tested capability contract.

## Evidence and close-out rules

For each completed item, record implementation PR/commit, exact test command/result, OS, overlay and topology. A successful bind or prefix-classification unit test is not proof of interoperability. Do not close integration tasks without a two-node test or clearly labelled equivalent harness.
