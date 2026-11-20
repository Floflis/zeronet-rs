# ZeroNet tracker full-announce hand-off

## Purpose

Resume the interrupted work after PR #3. The next implementation slice is:

1. Detect externally reachable peer services.
2. Populate the ZeroNet announce `add` field only from positive reachability evidence.
3. Announce the complete set of locally served sites in one native ZeroNet tracker request, using `delete=true` to reconcile stale site entries.

This document records the investigation and the unfinished design. It is **not** a claim that these features are implemented.

## Repository state at hand-off

- Repository: `Floflis/zeronet-rs`
- PR #3, **Implement native zero:// tracker interoperability**, is closed/merged.
- PR #4 is already closed/merged: **feat: add versioned JSONL bridge for Python clients**.
- Therefore this work cannot be PR #4 in the current GitHub history. Open the implementation as the next available PR (currently expected to be PR #5).
- The working branch observed during the interrupted session was `feat/zero-tracker-full-announce`, at commit `7cc7f894f27859ef66200797694cdc164c3f8e0c` (the PR #3 merge commit). The exploratory Git blobs created during the interrupted session were not assembled into a tree/commit and are not part of that branch.

## Existing baseline from PR #3

- The native ZeroNet `zero://` tracker handshake and single-site announce path exist.
- The normal daemon startup starts the TCP peer listener.
- The listener accepts sockets concurrently.
- `--fileserver_ip=*` binds to `0.0.0.0`; an explicit loopback address remains local-only.
- The outgoing handshake still advertises `port_opened=false`.
- The single-site announce still sends an empty `add` list.
- Tracker replies are parsed into peer addresses and covered by fixtures/tests.
- Onion tracker transport through Tor SOCKS5 exists, but onion-service identity/signing and local onion-service advertisement are not implemented.

## Findings and design decisions

### 1. A local bind is not external reachability

Do not infer reachability from any of the following alone:

- `TcpListener::bind()` succeeding;
- the configured bind address being `0.0.0.0`;
- an outbound tracker connection succeeding;
- a locally configured port number;
- a successful LAN discovery exchange.

A defensible positive signal is a completed inbound ZeroNet peer handshake on the listener, with the remote TCP source address globally routable. The successful TCP connection plus a valid protocol handshake demonstrates that an unsolicited connection reached this listener from outside the local/private address ranges.

Track evidence independently for IPv4 and IPv6. Expire evidence after a bounded TTL (15 minutes was proposed) so a stale success does not become a permanent claim. Only advertise the address family for which fresh evidence exists.

Important limitations to resolve:
- Check the standard-library IP classification against IANA special-purpose ranges, including shared address space, documentation, benchmarking, protocol-assignment and reserved ranges. Do not rely on `is_global` unless the project's Rust toolchain actually provides it.
- Treat IPv4-mapped IPv6 addresses consistently.
- Decide whether the proof should be keyed by listener/bind configuration and port, not merely process-global address family. If multiple listeners or ports are introduced, a process-global bit is insufficient.
- A successful inbound connection proves reachability from that source/network at that time, not universal reachability from every ISP or NAT. Document this as observed reachability, not a guarantee.
- Do not count a bare TCP accept. Count only after decoding and validating the ZeroNet handshake.

### 2. The announce `add` field

The implementation must not advertise `onion` unless this process actually owns and operates a reachable onion service with the required identity/signing support.

For IPv4/IPv6:
- Derive candidate service types only from fresh inbound handshake evidence.
- Confirm the exact service-type strings expected by the Python ZeroNet/AnnounceZero implementation before sending them. The interrupted draft considered `ipv4` and `ipv6`, but this must be checked against upstream protocol code and fixtures; do not assume these names are correct.
- Keep `add=[]` when there is no positive evidence.
- Do not set `port_opened=true` merely because a service type is included in an outgoing tracker announce. The handshake flag and tracker `add` field must use the same evidence model, while respecting their exact protocol semantics.

### 3. Multi-site announce and `delete`

The intended native API is conceptually:

```rust
announce_many(
    tracker,
    site_hashes,
    local_fileserver_port,
    proven_service_types,
    delete: true,
).await
```

The request should contain the complete, deterministic set of site hashes currently served by this node. Sort the site list before building the request so response groups can be mapped back to sites consistently.

Protocol checks required before implementation:
- Verify the exact meaning of `delete=true` in the upstream AnnounceZero implementation. Confirm that it means removing this peer's previous announcements for sites omitted from the supplied complete list, rather than deleting the newly announced entries.
- Confirm whether the tracker expects one peer group per input hash, including an empty group when a site has no peers.
- Confirm the exact packed-peer formats and whether groups are ordered identically to request hashes.
- Verify behavior for an empty site list. Do not send an empty destructive full-sync request until upstream semantics are confirmed.
- Preserve duplicate peers across different site groups when a peer serves multiple sites; deduplicate only within each site's group.
- Reject malformed or mismatched response group counts rather than silently zipping truncated lists.

Only native `zero://` trackers should receive this ZeroNet-specific multi-site/delete request. Do not route it through HTTP or BitTorrent UDP tracker code.

### 4. Site registry lifecycle

The normal startup listener currently receives a `SitesController` populated from persisted site data. It is a separate controller from the Web UI's actor/controller.

Before relying on periodic full announces, ensure the peer listener's served-site registry reflects site additions, removals, and serving-state changes made after startup. A periodic snapshot of a stale in-memory map is not a correct full announce.

Choose and test one explicit synchronization strategy:
- share a single authoritative site registry between the UI/site server and peer listener; or
- send add/remove/update events to the peer listener; or
- reload the authoritative persisted registry before each full announce, if that is safe and inexpensive.

Do not silently treat startup-only state as the complete current site set.

## Interrupted implementation notes

The previous session began exploratory edits in GitHub blobs, but did not create a commit. The following ideas were drafted and must be re-evaluated rather than copied blindly:

- A new `src/discovery/reachability.rs` module with IPv4/IPv6 proof timestamps and a 15-minute TTL.
- Record a proof only after a valid inbound ZeroNet handshake from a globally routable source.
- Read current site hashes from the peer listener's site controller, sort them, and send a periodic full announce.
- Add an optional `delete` field to the native ZeroNet announce request, omitted for existing single-site calls and set to true for a verified full announce.
- Return peer groups as `Vec<Vec<PeerAddr>>` for multi-site calls, retaining a compatibility wrapper for the existing single-site API.
- Correct an empty-site fixture to represent one empty peer group for one requested site, if upstream protocol behavior confirms that shape.

No compile, test, or tracker interoperability result was obtained for those exploratory edits. They are not validated implementation.

## Suggested implementation order

1. Inspect upstream Python ZeroNet tracker/AnnounceZero code and pin down `add`, `delete`, empty-list, and response-group semantics. Record source references in tests/comments.
2. Implement and unit-test IP classification and expiring reachability evidence.
3. Hook proof recording into the accepted peer connection path, after successful handshake validation.
4. Add focused tests proving private, loopback, link-local, documentation and reserved source addresses never create service evidence.
5. Extend the ZeroNet announce request with backward-compatible optional `delete`; preserve existing single-site behavior.
6. Implement multi-site request/response grouping and fixtures generated from or verified against Python ZeroNet.
7. Synchronize the live served-site registry with the peer listener.
8. Add a local mock-tracker integration test that verifies the exact handshake, announce payload, hash ordering, add list, delete flag, and per-site response mapping.
9. Add an opt-in real Python ZeroNet tracker interoperability test. Never make ordinary CI depend on a public tracker being online.
10. Run formatting, all-feature tests, Clippy, and inspect CI on the new PR head.

## Acceptance criteria

- [ ] A successful local bind alone never creates reachability evidence.
- [ ] Only a valid inbound ZeroNet handshake from a globally routable source records evidence.
- [ ] IPv4 and IPv6 evidence are independent and expire.
- [ ] `add` contains only upstream-confirmed service names backed by fresh evidence.
- [ ] Onion is never advertised without a real local onion service and signing capability.
- [ ] The full announce contains exactly the currently served site hashes.
- [ ] `delete=true` semantics are confirmed against upstream before use.
- [ ] Response groups are validated and mapped to the corresponding site hashes.
- [ ] Existing single-site tracker behavior remains compatible.
- [ ] Site additions/removals after startup are reflected in the next full announce.
- [ ] Unit, fixture, mock integration, formatting, Clippy and CI checks pass.
- [ ] A real tracker test is documented separately and is not required for offline CI.

## Non-goals for this slice

- Implementing local Tor onion-service hosting or RSA onion signing.
- Claiming universal public reachability from a single successful inbound peer.
- Advertising a port merely because it is configured or bound.
- Sending ZeroNet-specific `delete` semantics to HTTP/UDP BitTorrent trackers.
