# Agent instructions: zeronet-rs

## Scope and source of truth

The CJDNS/Yggdrasil implementation backlog is tracked in `docs/CJDNS-YGGDRASIL-SUPPORT-TASKLIST.md`.
Its stable task identifiers are the authoritative references for planning, implementation,
review, testing, and close-out:

- ZirohNeth: `ZN-001` through `ZN-044`.
- zeronet-rs: `ZR-001` through `ZR-047`.

Do not renumber, reuse, silently rename, or remove an identifier. Refer to the exact
identifier in branch names, commit messages, pull-request descriptions, review notes,
test names where practical, and evidence records. If a task is split, retain the original
ID for the original scope and add clearly documented child IDs without changing existing IDs.

## Before changing code

1. Read this file and the relevant task entry in `docs/CJDNS-YGGDRASIL-SUPPORT-TASKLIST.md`.
2. Inspect the current implementation, tests, CI workflows, and related history before
   proposing a change. Do not assume a task is still unresolved without checking its code
   and evidence.
3. Keep the change within the task's scope. Preserve existing ZeroNet protocol behavior,
   IPv4 support, and ordinary public IPv6 unless the task explicitly requires otherwise.
4. Do not claim CJDNS/Yggdrasil reachability from prefix classification, a local route,
   an outbound connection, or a successful socket bind alone.
5. Do not introduce placeholder implementations, blanket lint suppressions, or tests that
   merely compile without exercising the behavior being claimed.

## Required evidence for every completed task

A checkbox in the task list may be changed to `[x]` only when its evidence record
immediately below it is complete in the same change. Use this record:

```text
Task: ZR-NNN
Implementation PR: https://github.com/Floflis/zeronet-rs/pull/<number>
Commit: <full 40-character commit SHA>
Tests: <exact command(s)> — PASS/FAIL; <CI run URL or reproducible output>
Runtime/topology: <OS and version; overlay and version; node count; relevant
  interfaces/routes and setup; or N/A with a concrete reason>
```

Replace the example task ID with the actual stable ID. For cross-repository work,
include every relevant implementation PR and full commit SHA, not just the PR in
this repository.

Evidence rules:

- The documentation/tasklist PR is not an implementation PR and cannot, by itself,
  satisfy an implementation task.
- Record the commit that actually contains the implementation and/or test. Do not use
  an abbreviated SHA.
- Record commands exactly as executed, including workspace/package filters and feature
  flags. Report failures honestly; a failed or skipped test is not a pass.
- Link the CI run when available. If CI is unavailable, include reproducible output and
  enough environment detail to repeat the check.
- Use `N/A` only for genuinely non-runtime tasks, and explain why runtime topology does
  not apply.
- Unit tests, integration tests, cross-platform tests, and real multi-node overlay tests
  prove different things. Never present one category as evidence for another.
- For interoperability claims, record both nodes and the actual overlay/network
  topology. A mock or simulated harness must be labelled as such.
- Keep a task unchecked while any required evidence is pending, ambiguous, or unverifiable.

## Review and change discipline

- Make changes traceable to one or more task IDs. Include the IDs in the PR description.
- Prefer focused PRs. If a task spans both repositories, coordinate the contracts and
  link the companion PR.
- Do not mark unrelated tasks complete merely because nearby code changed.
- Do not rewrite historical commits or alter unrelated open PRs as part of task work.
- Never publish private keys, credentials, tokens, or sensitive node configuration in
  source, logs, test fixtures, PRs, or evidence.
- Update documentation when behavior, platform support, ownership boundaries, or test
  procedures change.
- If work is incomplete, blocked, or not yet tested, leave the checkbox open and document
  the blocker or next step.

## Repository-specific coordination

- Keep this repository's responsibilities explicit and avoid duplicating networking
  ownership across Python and Rust.
- When a change affects the shared networking contract, link the corresponding work in
  the companion repository and document which side owns route monitoring, listeners,
  reachability evidence, peer eligibility, and tracker announcements.
- Preserve the existing historical PR/commit record. Do not amend or force-push unrelated
  history.

Current related documentation PR: https://github.com/Floflis/zeronet-rs/pull/11
Companion repository PR (context only): https://github.com/Floflis/ZirohNeth/pull/17
