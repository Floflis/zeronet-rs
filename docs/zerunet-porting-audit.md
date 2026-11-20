# 🌈 ZeroNet Rust consolidation audit

> Audit snapshot: 2026-11-19. This document records the source state inspected before
> the first integration changes. Re-check upstream branches before future syncs.

## 🌈 Repositories and selected baseline

| Repository | Branch / revision inspected | Role |
|---|---|---|
| [canewsin/zerunet](https://github.com/canewsin/zerunet) | **dev**, **5ef483f7a022571fc8fdfd9996da50769c63b3c2** | Feature reference; old Rust prototype |
| [ZeroNetX/zeronet-rs](https://github.com/ZeroNetX/zeronet-rs) | **main**, **5504af6b34c2d6a5e162d8b7cf8823c5a733a11d** | Main implementation baseline |
| [Floflis/zeronet-rs](https://github.com/Floflis/zeronet-rs) | **main**, initially based on the same **0.3.1** revision | Integration destination |

The fork is being developed independently. It is not being added to ZirohNeth as a
submodule in this phase.

## 🌈 Build assessment

### 🌈 zerunet

The repository describes itself as a Rust ZeroNet client, but a clean checkout does
not contain all of the local crates referenced by its manifests.

Unresolved path dependencies in the inspected Cargo.toml include:

- **../zerucrypt**
- **../upnp-brunch**
- **../zeronet-protocol**

The nested **zerucontent/Cargo.toml** also references **../../zerucrypt**. A fresh clone
of **canewsin/zerunet** does not provide those sibling directories. The repository's
**content_manager** also pulls Diesel from a Git dependency. Consequently, these
commands are the intended Cargo entry points, but they are **not expected to build
from the repository alone** until the missing sibling crates and their revisions are
resolved:

~~~sh
git clone https://github.com/canewsin/zerunet.git
cd zerunet
git checkout dev
cargo build
cargo test
~~~

Other reproducibility concerns include wildcard dependency versions, an old
Rust 2018 codebase, and a direct I2P Git dependency. I2P is explicitly out of scope
for this integration and must not be brought into the target.

The inspected **zerunet** repository did not expose a root license file or a declared
license in its repository metadata. Its README's informal code-of-conduct wording
is not a software license. For that reason, this integration does not copy its
source files verbatim. New target-side code is independently written to implement
the compatible LAN discovery exchange. Revisit source reuse only if an explicit
license or permission is established.

### 🌈 zeronet-rs

The target README and CI agree on a nightly compiler dated 2025-04-26. The source
uses nightly language features (**trait_alias** and **let_chains**). CI checks out
submodules and runs the workspace test suite with all features.

~~~sh
git clone https://github.com/Floflis/zeronet-rs.git
cd zeronet-rs
git submodule update --init --recursive
rustup toolchain install nightly-2025-04-26
rustup override set nightly-2025-04-26
cargo test --workspace --all-features
cargo build --release
~~~

The repository's **tools.yml** CI workflow runs tests, Clippy and rustfmt with
**nightly-2025-04-26**. The previous stable-rustfmt job could not parse the project's
**let_chains** syntax under edition 2021, so the formatter is aligned with the same
nightly toolchain required by the source. A PR against **main** exercises this CI path.

The target currently enables **decentnet_protocol**'s **full** feature, which includes
its Tor support. It does not enable the protocol crate's optional **i2p** feature.
No I2P dependency or I2P-specific code is being added by this work.

**Execution note:** the current editing environment has GitHub repository access but
does not have **cargo** or **rustc** installed. Local compilation cannot be truthfully
claimed; the PR's GitHub Actions checks are the compilation gate.

## 🌈 Feature comparison and transplant decisions

| Area | zerunet findings | zeronet-rs findings | Decision |
|---|---|---|---|
| LAN peer discovery | MessagePack broadcast on UDP 1544; discovery request/response followed by a site-list exchange | No equivalent LAN discovery module found | **Port first**, reimplementing the wire exchange and retaining normal peer handshake validation |
| BitTorrent trackers | Tracker manager and announcer skeletons | HTTP and UDP tracker announce implementations | Keep target implementation; compare edge cases later |
| Peer connections | Actix actors and an older direct TCP connection abstraction | Async protocol crate, peer connect, handshake, ping, get-file and peer exchange | Keep target transport; do not transplant the older socket layer |
| Content model | Older zerucontent model and deterministic JSON formatting | Newer shared zerucontent with cert/meta support | Keep target model; add compatibility tests rather than replacing it |
| Signing and verification | zerucrypt API and SHA-512 content logic | zeronet_cryptography and current zerucontent signing APIs | Keep target crypto path |
| SQLite / site index | Content manager has schema groundwork, but insert_content is empty and optional-file insertion is a stub | Existing DB manager, schema rebuild and query paths | Keep target DB implementation |
| Site manager / UI / WebSocket | Several TODOs and incomplete API handlers | More complete controllers, site server and WebSocket handlers | Keep target implementation |
| Telemetry | Optional InfluxDB logger, disabled in the main routine | No equivalent integrated path identified | Defer; not a protocol requirement |

### 🌈 LAN discovery wire exchange

The target-side LAN discovery implementation follows the observed ZeroNet message
sequence:

1. Send a MessagePack **discoverRequest** to IPv4 broadcast port **1544**.
2. Receive **discoverResponse** messages and retain the sender's source IP, peer ID,
   file-server port and advertised reply port.
3. Send **siteListRequest** directly to each responder's advertised reply port.
4. Accept a peer only when its **siteListResponse** includes the SHA-256 hash of the
   requested site's address.
5. Return the peer as a candidate. The existing **Site::find_peers** flow still
   connects and performs the normal ZeroNet handshake before treating it as valid.

The discovery socket binds to an ephemeral local UDP port and advertises that port
in the message. This avoids colliding with a second client already listening on the
legacy fixed port. LAN discovery is best-effort: UDP broadcast failure must not
break tracker discovery.

## 🌈 Follow-up work, not included in this first integration

- Add cross-implementation fixtures for the LAN discovery MessagePack messages.
- Add a loopback integration test with two local discovery sockets.
- Validate peer-list behavior on Linux, Windows and Android/Termux.
- Compare tracker announce behavior against the Python ZeroNet client.
- Add a stable CLI/IPC contract before ZirohNeth starts invoking this binary.
- Consider a library crate once the public API is stable. The future ZirohNeth
  submodule should consume a versioned CLI or library interface, not reach into
  private implementation modules.
