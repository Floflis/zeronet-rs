# Original IPv6 mesh implementation history

This document preserves the lineage of the original IPv6/CJDNS/Yggdrasil work carried by [PR #9](https://github.com/Floflis/zeronet-rs/pull/9).

## Relationship to the maintained implementation

The original implementation line ends at commit `5adcb67069c2f0c4859f78a805fc34a94a91d5e4` and contains 12 historical commits. Its work was reapplied and evolved in [PR #8](https://github.com/Floflis/zeronet-rs/pull/8), whose branch is based on the newer `main`.

The maintained implementation in PR #8 is the code-bearing integration. It retains the useful design work from the original line:

- independently configured IPv4 and IPv6 peer listeners, with explicit IPv6-only sockets;
- reuse of the existing IPv6 peer transport and `peers_ipv6` exchange;
- CJDNS (`fc00::/8`) and Yggdrasil (`0200::/7`, including `0300::/8`) address classification;
- family-aware reachability evidence and tests.

It deliberately replaces the original route-probe announcement rule. A local route or successful bind is not proof that peers can reach the service. PR #8 advertises IPv6 only from fresh evidence produced by a valid inbound ZeroNet v2 handshake on an active IPv6 listener; that evidence expires after 15 minutes.

## History and merge order

This branch keeps the original 12-commit ancestry. It is stacked on PR #8 so the maintained implementation remains the first PR to merge. Merging this archival PR into the PR #8 branch preserves the older commit history without replacing the newer implementation tree.

The historical route-probe implementation remains available in the original commits for comparison. It is not reintroduced into the maintained code.

## Validation boundary

The existence of this history record does not claim that the original overlay implementation was tested end-to-end. Real two-node CJDNS/Yggdrasil interoperability and cross-platform listener validation remain separate validation tasks.
