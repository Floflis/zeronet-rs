# ZeroNet tracker full-announce: implementation record

## Status

**Implemented and merged.** The work originally captured as a hand-off is implemented in PR #6, **feat: implement ZeroNet full tracker announce**, merged into `main` on 2026-11-20.

- PR #5 was the documentation-only hand-off: [#5](https://github.com/Floflis/zeronet-rs/pull/5).
- PR #6 implemented the hand-off: [#6](https://github.com/Floflis/zeronet-rs/pull/6).
- PR #6 merge commit: `d39329c2b037b53be91d0e817cc0434dae652274`.
- PR #6 CI run: [Continuous Integration #45](https://github.com/Floflis/zeronet-rs/actions/runs/44819172004), successful for rustfmt, Clippy and `cargo test --workspace --all-features`.

This file is now a record of the delivered behavior and its boundaries. The old hand-off's implementation checklist is not a list of pending work.

## Delivered behavior

### 1. Evidence-backed external reachability

- IPv4 and IPv6 reachability evidence is tracked independently.
- Evidence is recorded only after a valid inbound ZeroNet v2 peer handshake from a globally routable source address. A TCP accept, successful local bind, configured port, outbound tracker connection or LAN discovery exchange alone is not proof.
- Evidence expires after 15 minutes.
- Private, shared, loopback, link-local, documentation, benchmarking, multicast and reserved address ranges are excluded. IPv4-mapped IPv6 addresses are handled consistently.
- The handshake `port_opened` flag and tracker announce `add` service types use the same positive evidence model.
- The IPv4/IPv6 service types are advertised only when supported by fresh evidence. Onion is not advertised because local onion-service hosting and the required identity/signing support are not implemented.

A successful inbound handshake is evidence of observed reachability from that source/network at that time. It is not a universal guarantee that every ISP, NAT or remote peer can reach the service.

### 2. Full multi-site announce and deletion reconciliation

- Native `zero://` trackers receive the complete, deterministic set of site hashes currently served by this peer.
- The full announce uses `delete=true`, including an empty site snapshot when no sites remain, so stale announcements can be reconciled.
- Site hashes are ordered deterministically before the request is built.
- Responses preserve one peer group per requested site. Peers are deduplicated within each site's group, while the same peer may remain present in multiple groups when it serves multiple sites.
- Compatible all-empty response shapes are accepted; partial or mismatched response group sets are rejected instead of being silently truncated or zipped.
- ZeroNet-specific `delete` behavior is confined to native `zero://` trackers. HTTP and UDP BitTorrent tracker discovery paths do not receive it.

### 3. Live serving-site registry and PEX

- The peer listener shares the authoritative serving-site registry.
- Pause, resume and delete operations update the registry immediately; newly served sites can also be loaded after startup.
- Peers learned from full announces are fed into the listener's PEX peer set.

## Tests and validation delivered in PR #6

The implementation added coverage for:

- IPv4/IPv6 reachability classification, mapped IPv4, special-purpose ranges, address-family separation and evidence expiry.
- Valid and invalid inbound peer-handshake validation.
- Multi-site response grouping, per-site deduplication, and empty/partial response handling.
- A mock TCP tracker exercising ordered hashes, service-type `add`, `delete=true` and empty-site full sync.
- Shared live site-registry ordering and replacement.

The PR #6 CI run completed successfully:
- `rustfmt`: passed.
- `cargo clippy`: passed.
- `cargo test --workspace --all-features`: passed.

No live public-tracker interoperability session is recorded here. Such a test may be run as an opt-in external check; ordinary CI does not depend on a public tracker being available.

## Historical hand-off context

The work was initially interrupted after PR #3, and PR #5 preserved the investigation and acceptance criteria without claiming implementation. The exploratory blobs from that interrupted session were not a committed implementation. PR #6 superseded that draft and delivered the behavior described above.

The original design concerns remain useful as constraints when modifying this code:

- Never infer public reachability from a local bind or outbound connection.
- Do not claim universal reachability based on one successful inbound peer.
- Do not advertise Onion without a real local onion service and the required identity/signing capability.
- Do not send ZeroNet-specific `delete` semantics to HTTP or UDP BitTorrent trackers.
- Keep the live serving-site registry synchronized with the peer listener.

## Non-goals and remaining boundaries

- Local Tor onion-service hosting, onion identity and RSA signing.
- A guarantee of reachability from every ISP, NAT or remote peer.
- A mandatory public-tracker test in offline CI.

These are explicit boundaries, not unfinished acceptance criteria for the full-announce implementation.
