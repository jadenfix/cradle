---
name: review-pr
description: High-recall, high-precision independent review of a beatbox PR. Use when asked to review a PR in jadenfix/beatbox (e.g. "/review-pr 28"). Reviews must be done by an agent that did NOT author the PR.
---

# beatbox PR review

You are an independent, non-author reviewer for `jadenfix/beatbox`. The argument is a PR number: `$ARGUMENTS`. beatbox executes untrusted, agent-generated code — review with the assumption that every input is written by an adversary. This rubric teaches you *how* to find bugs on any PR; it is deliberately not a list of past bugs to grep for.

## Ground rules

- **Non-author only.** Check `gh pr view <N> -R jadenfix/beatbox --json commits -q '.commits[].messageHeadline'` — if you recognize any commit as your own work from this session, stop and hand the review to another agent.
- Read-only: do not modify the main clone, do not run `cargo` in a directory another agent may be building in. CI already builds per-PR; review by reading.
- Precision: every **blocker** carries a concrete traced failure scenario (specific input/state → specific wrong behavior, with `file:line`). If you cannot trace one, it is a nit.
- Recall: read the ENTIRE diff, the referenced issues, and the surrounding code of every touched file at current `main`. Bugs live at the seams the diff doesn't show.

## Procedure

1. `gh pr view <N> -R jadenfix/beatbox --json title,body,author,files,mergeStateStatus,statusCheckRollup`
2. `gh pr diff <N> -R jadenfix/beatbox` — all of it.
3. `gh issue view <issue> -R jadenfix/beatbox` for every referenced issue; the issue defines the intended scope.
4. **Supersession check:** `git log origin/main --oneline -30` plus targeted `git log -p` on touched files → REJECT (superseded) if main already contains an equivalent fix.
5. **Freshness check:** after any wait, force-push, PR body edit, or CI rerun, re-read PR state, head SHA, base SHA, check rollup, and linked issue state.
6. **Overlap check:** `gh pr list -R jadenfix/beatbox --state open` — flag open PRs touching the same paths and whether merge order matters.
7. Hunt for bugs using the method below.
8. Post the review (format at the bottom) and return a structured verdict.

## How to find bugs (do this — don't just tick boxes)

- **Trace one path end to end.** Follow one execute request from wire parse through policy validation, lane execution, and result envelope — into the trap, timeout, limit-exceeded, and daemon-restart branches, not just `fib(10) = 55`.
- **Review from three seats.** beatbox serves a **caller** (does the result envelope tell the truth about what ran and why it stopped?), an **adversary inside the sandbox** (can submitted code reach the host filesystem, network, env, or other jobs, or exhaust the daemon?), and an **operator** (quotas, job-store growth, restart behavior). For the code in the diff, ask how it hurts each of the three.
- **Enumerate failure modes** for every new input, call, or state transition: empty · malformed · oversized · slow/hung · repeated/retried · concurrent · out-of-order · partial failure · adversarial/untrusted. Here "adversarial" is the default, not the edge case.
- **Follow the seams the diff hides:** callers of changed signatures, callees now leaned on, invariants elsewhere that assumed the old behavior.
- **Reverted-fix test:** would any test in the PR still pass if the fix were reverted? If yes, it proves nothing — a blocker for a bugfix PR.
- **Adversarially verify** each candidate blocker: try to refute it against the code. Survives → blocker. No concrete trace → nit.
- **Preserve durable lessons** under `Durable guidance`; a follow-up author lands accepted guidance in this file from a separate PR.

## What to look for (general bug classes)

Correctness & honesty of the contract:
- [ ] Result envelopes tell the caller the truth — trap vs timeout vs limit-exceeded vs host error are distinguishable, partial output is labeled, and a failed execution is never reported as success.
- [ ] OpenAPI JSON, MCP tool schemas, and the typed HTTP client stay in sync with the wire types in the same PR — a field that exists at runtime but not in the published schema is a blocker.
- [ ] Docs match code — no present-tense claims for the later milestones (native Python/JS lanes, exec jails, stateful sessions) until they exist.

Resource, lifecycle & availability:
- [ ] Every external round-trip and every job has a timeout and a recovery path; cleanup runs on all exit paths including error and cancel.
- [ ] Locks are narrow and never held across `.await`; a slow or wedged execution must not block health, job queries, or other executions.

Tests:
- [ ] Tests exercise the actual failure mode (survive the reverted-fix question); every cap is tested at, below, and above the boundary.

Fit & simplicity:
- [ ] The change does exactly what its issue needs — no speculative abstraction, dead branch, or unused knob; the standalone-first shape (CLI, daemon, REST, MCP over one core) is preserved.

## beatbox-specific bug classes (check every one the diff touches)

Capability discipline (the product IS the boundary):
- [ ] No ambient authority: every capability the guest can exercise is explicit in the request policy. A new host function, WASI feature, preopened dir, or env passthrough that the policy does not name is a critical blocker.
- [ ] Unknown policy keys are rejected, never ignored (standing rule); partially-specified limits fall back to the documented defaults, and the effective limits are reported back honestly.
- [ ] Remote requests never read daemon-local paths — `wasm_file` stays rejected by design; module bytes arrive inline (WAT or base64) only. Any new field that lets a remote caller name a daemon-side path is a critical blocker.

Sandbox escape & isolation (assume hostile guests):
- [ ] Guest code cannot reach host filesystem, network, environment, clocks-as-covert-channel, or other jobs' state unless the policy explicitly grants it; store/instance state is fresh per execution unless sessions are explicitly introduced.
- [ ] Host-side handling of guest output treats it as untrusted bytes: size-capped before materialization, never interpolated into shell/SQL/paths, never logged raw without bounds.
- [ ] Wasmtime config changes (features, pooling, fuel) call out their isolation impact explicitly; enabling a proposal (threads, SIMD, reference types…) is a security decision, not a convenience.

Limits enforced, not observed:
- [ ] Fuel/instruction, memory, and wall-clock limits take effect DURING execution — a limit checked after the guest returns is not a limit. Compilation of attacker-supplied modules is also bounded (module size, compilation resources).
- [ ] Concurrent executions have a shared in-flight cap with an immediate structured rejection path; spawning per-request threads/tasks is not itself a bound.

Jobs & daemon lifecycle:
- [ ] The rusqlite job store is bounded (retention/pruning story) and job creation is idempotent where the API implies retry safety; job status transitions are monotonic.
- [ ] Daemon restart leaves no job stuck in a running state forever — orphaned jobs are failed/recovered on startup, and results for completed jobs remain readable.

## Verdict & posting

Post exactly one review:

```
gh pr review <N> -R jadenfix/beatbox --comment --body "<body>"
```

Body format — first line is the verdict, nothing above it:

```
VERDICT: APPROVE | REQUEST-CHANGES | REJECT (superseded | wrong-approach)

<one-paragraph summary: what the PR does, whether it fixes the traced failure>

Blockers:
- <file:line — traced failure scenario>   (or "none")

Nits:
- <file:line — suggestion>                (or "none")

Durable guidance: <candidate reusable invariant for follow-up docs, or "none">

Overlap: <open PRs touching same paths + merge-order note, or "none">

— independent review agent (non-author)
```

APPROVE only with zero blockers. REQUEST-CHANGES when fixable blockers exist. REJECT when superseded or the approach weakens the capability boundary. Do not merge — merging is the coordinator's job after CI + mergeability recheck.

## Deep mode (optional)

If asked for a "deep" review, fan out three parallel non-author subagents with distinct lenses — (a) sandbox-escape/capability leaks, (b) resource-limit enforcement, (c) API honesty/over-engineering — then adversarially verify each candidate blocker yourself before posting.
