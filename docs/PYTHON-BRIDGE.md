# 🌈 Python ↔ Rust bridge

## 🌈 Purpose

The `bridge` command starts a long-lived, local child process that exposes selected native `zeronet-rs` operations to a Python client.

The transport is newline-delimited JSON (JSON Lines) over the child process' standard input and output. It deliberately avoids a Python ABI, PyO3 wheels, a TCP control port, and spawning a new Rust process for each operation.

This is an IPC boundary, not a Python extension module: each request is dispatched to native Rust functions inside one persistent `zeronet-rs` process.

## 🌈 Running the bridge

Build the normal `zeronet` executable, then start it in bridge mode:

```sh
./target/release/zeronet bridge
```

On Windows:

```powershell
.\target\release\zeronet.exe bridge
```

A Python client should launch this process once, keep its pipes open, and reuse it for multiple operations. Closing the child's standard input ends the bridge process.

Bridge mode is entered before the normal client startup path. It does not start the Web UI, the site server, or the default LAN discovery service with a generated identity. The Python host supplies the identity and fileserver port that the LAN service must advertise.

## 🌈 Wire protocol

Each request is one UTF-8 JSON object followed by a newline. Each request receives exactly one JSON response line.

Request envelope:

```json
{
  "protocol": "zeronet-bridge",
  "version": 1,
  "id": "request-001",
  "method": "system.ping",
  "params": {}
}
```

Successful response:

```json
{
  "protocol": "zeronet-bridge",
  "version": 1,
  "id": "request-001",
  "ok": true,
  "result": {
    "name": "zeronet-rs",
    "bridge_protocol": 1
  }
}
```

Error response:

```json
{
  "protocol": "zeronet-bridge",
  "version": 1,
  "id": "request-002",
  "ok": false,
  "error": {
    "code": "unknown_method",
    "message": "unsupported bridge method: example.nope"
  }
}
```

The `id` must be a string or number and is copied into the response unchanged. The `protocol` marker and `version` are mandatory. Version negotiation is strict: this implementation accepts version `1` only. `params` must be an object.

The child process reserves **stdout** for protocol responses. Diagnostics must go to **stderr**. A parent should read one response line for every request before sending another request, or otherwise serialize access to the pipes and correlate responses by `id`.

## 🌈 Version 1 methods

### 🌈 `system.ping`

No parameters. Confirms that the bridge is alive and reports its protocol version.

### 🌈 `discovery.lan.configure`

Configure native ZeroNet LAN discovery. This must be called once after starting the process.

Parameters:

| Field | Type | Meaning |
|---|---|---|
| `peer_id` | string | The Python host's existing ZeroNet peer ID |
| `fileserver_port` | integer | The actual TCP fileserver port advertised to LAN peers; must be greater than 1 |
| `revision` | integer | The host's ZeroNet protocol revision |
| `local_sites` | string array | Site addresses currently served by the Python host |

Example:

```json
{
  "protocol": "zeronet-bridge",
  "version": 1,
  "id": "configure-1",
  "method": "discovery.lan.configure",
  "params": {
    "peer_id": "-UT3530-existing-peer-id",
    "fileserver_port": 15441,
    "revision": 4800,
    "local_sites": ["1HeLLo4uzjaLetFx6NH3PMwFP3qbRbTf3D"]
  }
}
```

The native service uses the ZeroNet LAN discovery UDP port **1544**. The peer ID, fileserver port, and revision are immutable for the lifetime of that bridge process. Restart the child process to change them.

### 🌈 `discovery.lan.set_local_sites`

Update the site addresses advertised by the already-configured LAN service.

Parameters:

- `local_sites`: array of site-address strings.

Call this when sites are added or removed. It does not change the peer ID, revision, or fileserver port.

### 🌈 `discovery.lan.discover`

Discover peers advertising one site on the local network.

Parameters:

- `site`: site address string.

The result contains the requested site, a peer count, and a sorted array of `ip:port` peer addresses. These are candidates only. The host must still connect and perform the normal ZeroNet handshake before trusting a peer.

## 🌈 Python process example

This example illustrates the lifecycle and request/response framing. Production code should add application-specific shutdown handling and serialize access to the two pipes.

```python
import json
import subprocess
from itertools import count

class ZeroNetRustBridge:
    def __init__(self, binary):
        self.process = subprocess.Popen(
            [binary, "bridge"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            # Keep diagnostics away from the JSONL response stream.
            stderr=None,
            text=True,
            encoding="utf-8",
            bufsize=1,
        )
        self.request_ids = count(1)

    def call(self, method, params=None):
        request_id = str(next(self.request_ids))
        request = {
            "protocol": "zeronet-bridge",
            "version": 1,
            "id": request_id,
            "method": method,
            "params": params or {},
        }
        self.process.stdin.write(json.dumps(request, separators=(",", ":")) + "\n")
        self.process.stdin.flush()

        line = self.process.stdout.readline()
        if not line:
            raise RuntimeError("zeronet-rs bridge exited without a response")

        response = json.loads(line)
        if response.get("id") != request_id:
            raise RuntimeError("zeronet-rs bridge response ID mismatch")
        if not response.get("ok"):
            error = response.get("error", {})
            raise RuntimeError(
                f"{error.get('code', 'bridge_error')}: "
                f"{error.get('message', 'unknown bridge error')}"
            )
        return response["result"]

    def close(self):
        if self.process.stdin and not self.process.stdin.closed:
            self.process.stdin.close()
        self.process.wait()
```

## 🌈 Migration notes for ZirohNeth

The first intended consumer is `plugins/AnnounceLocal`. The Python host should:

1. Start one bridge child for the lifetime of its fileserver.
2. Call `discovery.lan.configure` with its existing peer ID, actual fileserver port, revision, and currently served site addresses.
3. Replace Python LAN discovery queries with `discovery.lan.discover`.
4. Forward returned addresses through the existing Python peer-add / worker-notification path.
5. Call `discovery.lan.set_local_sites` after site additions or removals.
6. Close stdin and wait for the child during shutdown.

**Do not run the Python `AnnounceLocal` listener and the Rust LAN listener at the same time.** Both use UDP port 1544, and the current Rust socket does not implement shared-port socket semantics. Disable the Python listener when the native bridge owns LAN discovery; keep a deliberate fallback path for startup failure instead of silently starting both listeners.

A discovered address is not an authenticated peer. Reuse ZirohNeth's normal connection and handshake validation before accepting it for content exchange.

## 🌈 Scope and non-goals

This first bridge version exposes only process health and LAN discovery. It does not yet expose tracker announcements, outgoing peer connections, PEX, downloads, content verification, the WebSocket API, or plugin management.

The bridge is local stdio IPC. Do not expose it as an unauthenticated network service or pass private keys through its request stream. New methods must be added to this document and covered by versioned request/response tests before ZirohNeth depends on them.
