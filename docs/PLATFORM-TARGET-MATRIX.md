# OS and architecture support matrix

**Status:** portability plan; target availability is not a claim that the application already builds or runs there.

This matrix separates four different facts:

1. **OS port:** the operating system itself supports the CPU architecture.
2. **Rust target:** rustc defines a target triple for that OS/architecture.
3. **Build evidence:** this repository's dependency graph successfully compiles for that target.
4. **Runtime/release evidence:** the resulting executable has been run and packaged on that platform.

Do not promote a target from planned to supported merely because `rustup target list` knows its name.

## Target inventory

| Platform | Rust target(s) to evaluate | Current CI lane | Notes |
|---|---|---|---|
| Linux x86-64 | `x86_64-unknown-linux-gnu` | Native hosted runner | Baseline build/test lane |
| Linux ARM64 | `aarch64-unknown-linux-gnu` | Cross-build candidate | Prefer native ARM64 runner for runtime tests |
| Linux ARMv7 hard-float | `armv7-unknown-linux-gnueabihf` | Cross-build candidate | Verify libc, linker and bundled SQLite |
| Linux RISC-V 64 | `riscv64gc-unknown-linux-gnu` | Cross-build candidate | QEMU or native RISC-V required for runtime evidence |
| Android ARM64 / Termux | `aarch64-linux-android` | NDK cross-build candidate | Termux is a userspace environment, not a distinct Rust OS target; validate on-device |
| Android ARMv7 | `armv7-linux-androideabi` | NDK cross-build candidate | Validate API level and ABI |
| macOS Intel / Apple Silicon | `x86_64-apple-darwin`, `aarch64-apple-darwin` | Native macOS runners | Keep architecture-specific artifacts |
| Windows x64 | `x86_64-pc-windows-msvc` | Native Windows runner | MSVC toolchain |
| Windows ARM64 | `aarch64-pc-windows-msvc` | Native ARM64 runner or cross-check | Runtime test on ARM64 Windows |
| FreeBSD amd64 / ARM64 | `x86_64-unknown-freebsd`, `aarch64-unknown-freebsd` | BSD VM/self-hosted | Validate target tier and native libraries before release |
| FreeBSD ARMv7 / RISC-V | `armv7-unknown-freebsd`, `riscv64gc-unknown-freebsd` | Experimental | OS port and Rust target maturity differ; no release promise |
| OpenBSD amd64 / ARM64 / RISC-V | `x86_64-unknown-openbsd`, `aarch64-unknown-openbsd`, `riscv64gc-unknown-openbsd` | Experimental BSD VM | Tier 3 targets may require a custom toolchain |
| NetBSD amd64 / ARM64 / RISC-V | `x86_64-unknown-netbsd`, `aarch64-unknown-netbsd`, `riscv64gc-unknown-netbsd` | BSD VM / experimental | Verify current Rust target support independently for each triple |
| DragonFly BSD x86-64 | `x86_64-unknown-dragonfly` | Experimental BSD VM | Do not infer support for other DragonFly architectures |
| Redox x86-64 | `x86_64-unknown-redox` | Redox VM / redoxer | Tier 2 target; runtime evidence still required |
| Redox AArch64 / i586 / RISC-V 64 | `aarch64-unknown-redox`, `i586-unknown-redox`, `riscv64gc-unknown-redox` | Experimental Redox VM/QEMU | Tier 3; prebuilt std availability is not guaranteed |

## Dependency-specific risks

The current manifest includes `rusqlite` with bundled SQLite, `ping`, `socket2`, Actix, Tokio, Wasmer and Git dependencies. The target matrix must therefore be treated as a set of hypotheses until the complete dependency graph passes.

- Bundled SQLite needs a working C compiler and target toolchain.
- Socket and ICMP behavior varies by OS, privileges and available system APIs. A successful compile does not prove ping behavior.
- Wasmer's compiler/runtime backends may have a narrower architecture/OS matrix than Rust itself.
- Git dependencies must be pinned/reproducible before release builds can be considered reproducible.
- BSD and Redox support must be checked against each dependency's own platform support, not inferred from rustc.

## CI evidence contract

Use these labels in CI summaries and release notes:

- **T0 — target identified:** a valid target triple and OS/CPU port are documented.
- **T1 — toolchain available:** the selected Rust toolchain can install/build the target standard library.
- **T2 — crate cross-check:** this repository's relevant Cargo targets pass `cargo check`.
- **T3 — native/emulated test:** tests execute on the target OS/architecture, with the runner/VM recorded.
- **T4 — release artifact:** a packaged binary is produced and smoke-tested on a clean target environment.

A skipped or unavailable tier is **not tested**, never PASS. Keep experimental targets visible without making them release blockers until the dependency graph is ready.

## Implementation sequence

1. Keep native Linux, macOS and Windows builds as the release baseline.
2. Add Android NDK builds and test the ARM64 binary in Termux on real hardware.
3. Add Linux ARM64, ARMv7 and RISC-V cross-checks, then QEMU/native runtime jobs.
4. Add BSD VM lanes one OS at a time.
5. Add Redox x86-64 through redoxer/QEMU, then investigate AArch64, i586 and RISC-V 64 with the matching toolchains.
6. Publish only artifacts with T4 evidence; list lower-tier targets as experimental.

## Required follow-up

- [ ] Record `cargo check` results for each target that has a usable toolchain.
- [ ] Identify Wasmer and bundled SQLite blockers per target.
- [ ] Add reproducible BSD VM and Redox VM instructions.
- [ ] Run Android ARM64 smoke tests on a real Termux device.
- [ ] Add target-specific release artifacts only after T4 evidence exists.
