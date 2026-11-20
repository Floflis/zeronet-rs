# ZeroNetX
[![FOSSA Status](https://app.fossa.com/api/projects/git%2Bgithub.com%2Fcanewsin%2Fzeronet-rs.svg?type=shield)](https://app.fossa.com/projects/git%2Bgithub.com%2Fcanewsin%2Fzeronet-rs?ref=badge_shield)

Rust Implementation of ZeroNet Protocol

Currently This Repo is open to Code Reviews/Security Audit/Best Code Practice Suggestions. You can freely review and suggest your opinions to us.

## Features:
 - Site Create => Create New Site
 - Site Download => Download Site from internet peers
 - Site Need File => Download Site Single(Inner) File from peers
 - Find Peers via Trackers => Discover Peers using torrent tracker network
 - Site Sign => sign changes in site files
 - Site Verify => verify content file hashes with files
 - PeerExchange => Get more peers from connected peers
 - PeerPing => Get peer alive status
 - dbRebuild => Build db from data files using dbschema.json
 - dbQuery => Sql Query on built db to fetch data
 - getConfig => Client Config data
 
## Available Commands:
 - siteCreate
 - siteDownload
 - siteNeedFile
 - siteFindPeers
 - sitePeerExchange
 - siteFetchChanges
 - siteSign
 - siteVerify
 - peerPing
 - dbRebuild
 - dbQuery
 - getConfig

pass -s "Your Site Address" for above commands

## Download :
Latest Packages Available on [Github Releases](https://github.com/canewsin/zeronet-rs/releases/latest).

## Usage :
### Window :
> zeronet.exe siteDownload -s "1HELLoE3sFD9569CLCbHEAVqvqV7U2Ri9d"
### Linux/Mac :
> ./zeronet siteDownload -s "1HELLoE3sFD9569CLCbHEAVqvqV7U2Ri9d"

You may need to give exec permission on Linux/Mac OS
> chmod +x zeronet

and rerun the cmd

## Building ZeroNetX

### Repository

Clone the [ZNX repository](https://github.com/ZeroNetX/zeronet-rs).

### Dependencies

- Install [rustup](https://www.rust-lang.org/tools/install)

-  > cd zeronet-rs

- Windows : Install nightly rust toolchain using
  > rustup override set nightly-2025-04-26-x86_64-pc-windows-msvc

- `rustc --version` info for other platforms

  > rustc 1.88.0-nightly (b4c8b0c3f 2025-04-25)

Once you have the dependencies installed, you can build ZNX using [Cargo](https://doc.rust-lang.org/cargo/).

For a debug build:

```sh
cargo run
```

For a release build:

```sh
cargo run --release
```

And to run the tests:

```sh
cargo test
```

### Troubleshooting


#### Cargo errors claiming that a dependency is using unstable features

Try `cargo clean` and `cargo build`.

## License
[![FOSSA Status](https://app.fossa.com/api/projects/git%2Bgithub.com%2Fcanewsin%2Fzeronet-rs.svg?type=large)](https://app.fossa.com/projects/git%2Bgithub.com%2Fcanewsin%2Fzeronet-rs?ref=badge_large)

## Tracker configuration

Tracker endpoints are loaded from `assets/trackers.txt`, one endpoint per
line. The file is intentionally empty by default: no historical tracker is
treated as live or trusted without an interoperability check.

Supported entries are:

- `zero://host:port` for the native ZeroNet MessagePack tracker protocol.
- `http://host:port/announce` for BitTorrent HTTP trackers.
- `udp://host:port/announce` for BitTorrent UDP trackers.
- Bare `host:port` entries remain HTTP for legacy compatibility.

Blank lines and lines beginning with `#` are ignored. Duplicate entries are
removed, malformed addresses and unsupported schemes are skipped with a
warning, and the original order is preserved. Configuration parsing does not
perform DNS lookups.

The current native `zero://` transport supports clear-TCP clearnet endpoints
and Tor-routed onion tracker endpoints. Certificate-pinned TLS and RSA
onion-signing challenges are not implemented yet. Pinned endpoints are
rejected explicitly instead of being downgraded to clear TCP.

Native `zero://*.onion:port` endpoints are routed through a local Tor SOCKS5
proxy using the hostname as supplied, so the system resolver is not asked to
resolve the onion address. The default proxy is `127.0.0.1:9050`; set
`ZERONET_TOR_SOCKS=127.0.0.1:9150` (or another SOCKS5 socket address) to use
a different local listener. Tor must already be running. This does not yet
create local onion services or answer RSA onion-signing challenges.

## Peer TCP listener

The normal daemon startup path starts the native peer TCP listener using
`--fileserver_ip` and `--fileserver_port`. The default `--fileserver_ip=*`
means `0.0.0.0`, binding all IPv4 interfaces. To keep peer serving local-only,
set `--fileserver_ip=127.0.0.1`.

A successful local bind does **not** prove that the port is reachable from the
public internet. Until external reachability and supported service types can
be verified, peer handshakes continue to report `port_opened=false` and
ZeroNet tracker announces keep `add=[]`. Router/NAT/firewall checks and
service-type advertisement remain separate work.
