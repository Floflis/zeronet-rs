# ZeroNet-RS Roadmap

> Status: planning document. The existing commands and Python bridge are documented capabilities; unchecked items below are future work until backed by implementation and tests.

## Direction

Develop the Rust implementation as a reliable, auditable ZeroNet protocol foundation. Keep its command-line interface and versioned JSONL bridge stable so higher-level clients can integrate without depending on internal Rust details.

## Workstreams

### 1. Protocol correctness and compatibility
- [ ] Maintain a protocol compatibility matrix against the relevant ZeroNet/ZeroNetX behavior.
- [ ] Add fixtures and interoperability tests for site creation, download, file requests, signing, verification, and peer discovery.
- [ ] Document unsupported protocol features and intentional deviations.

### 2. Core reliability
- [ ] Expand negative-path tests for malformed data, unavailable peers, timeouts, interrupted transfers, and corrupted content.
- [ ] Verify atomicity and recovery behavior for filesystem and database updates.
- [ ] Add structured diagnostics for network, storage, database, and bridge operations.
- [ ] Define resource limits and cancellation behavior for long-running operations.

### 3. Peer discovery and transport
- [ ] Validate tracker interoperability for each supported tracker scheme using reproducible integration tests.
- [ ] Test peer exchange and peer liveness behavior under churn and partial network failure.
- [ ] Document tracker configuration, trust assumptions, and fallback behavior.

### 4. Python bridge
- [ ] Treat the JSONL bridge as a versioned public interface.
- [ ] Add schema/contract tests for every supported method and error response.
- [ ] Test long-lived process behavior, multiple requests, malformed input, and graceful shutdown.
- [ ] Keep examples synchronized with the bridge protocol documentation.

### 5. Packaging and portability
- [ ] Define a supported-target matrix based on actual CI/build/test evidence.
- [ ] Add reproducible builds and artifact verification for each supported target.
- [ ] Track architecture and operating-system support independently; do not infer RISC-V or alternative-OS support from x86-64/ARM results.

### 6. Security and release process
- [ ] Document key handling, signature verification, trust boundaries, and dependency-update policy.
- [ ] Add security-focused regression tests for parsing, path handling, and content verification.
- [ ] Publish release notes that distinguish implemented features from experimental or unsupported behavior.

## Completion policy

Every completed item must link to its implementation PR/commit and test evidence. Keep the roadmap synchronized with the CLI and bridge documentation; do not claim parity without interoperability results.
