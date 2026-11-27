# ZirohNeth Iroh bridge

This crate builds `zirohneth-iroh-bridge`, a supervised Iroh blob-transfer
process for the Python ZirohNeth client.

## Build

```bash
cargo build --release --manifest-path iroh-bridge/Cargo.toml
```

The binary is expected on `PATH` as `zirohneth-iroh-bridge`, or may be
selected with the `ZIROHNETH_IROH_BRIDGE` environment variable.

## Runtime contract

The process accepts one JSON object per line on stdin and returns exactly one
JSON object per line on stdout. Diagnostics go to stderr.

Every request contains:

```json
{"protocol_version":1,"request_id":1,"op":"health","args":{}}
```

Supported operations:

- `health`: protocol and endpoint status.
- `node_addr`: current Iroh `NodeAddr` as a JSON object.
- `blob.add`: import a local file into the persistent store and return its
  BLAKE3 hash and provider address.
- `blob.fetch`: download a hash from a specified `NodeAddr`, verify it in
  the iroh-blobs protocol, and export it to a path.
- `shutdown`: gracefully stop the service.

The bridge owns a single persistent endpoint identity and filesystem-backed
blob store beneath `--data-dir`. Its endpoint key is stored in
`endpoint.key` with owner-only permissions on Unix. The key is never exposed
through IPC.

Input and output file paths are restricted to the configured data directory.
The JSONL protocol is versioned and rejects requests larger than 64 KiB.
Iroh failures are returned as errors. The caller must not fall back to the
legacy ZeroNet transport for content-v3 files.
