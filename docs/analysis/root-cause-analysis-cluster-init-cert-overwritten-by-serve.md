# RCA — `cluster init` cert is overwritten by `serve` on the production-default config dir

**Status:** Analysis (no code changes proposed; recommendation only)
**Author:** Rex (Toyota 5 Whys, multi-causal)
**Date:** 2026-04-25
**Configuration:** `investigation_depth=5`, `multi_causal=true`, `evidence_required=true`
**Branch:** `marcus-sa/phase-1-control-plane-core`

---

## 1. Problem statement

`overdrive cluster init` and `overdrive serve` both unconditionally call `mint_ephemeral_ca()` and `tls_bootstrap::write_trust_triple(...)` against `<config_dir>/.overdrive/config`. When both commands target the **same** config directory — which is the production default `$HOME/.overdrive/` — `serve` runs second and **overwrites** the trust triple `cluster init` just produced. The operator's CLI was using the now-stale CA, so handshakes fail. The walking-skeleton acceptance test masks the defect by giving each command its own `TempDir`.

User confirmation: *"having used the cli locally, i can confirm that this does not work as expected. cluster init will generate a certificate and operator config, but then serve will regenerate a new one overwriting it."*

### Scope (what is in this analysis)

- The init-vs-serve double-mint on a shared `operator_config_dir`.
- Test design that hides it (`spawn_server` uses an isolated tempdir).
- Whether the fix belongs in Phase 1 or Phase 5; explicit weighing of "remove `cluster init` from Phase 1 entirely."
- Whether to adopt a Talos-shape `talosconfig` / `machineconfig` split.

### Out of scope

- The endpoint-with-resolved-port problem (already fixed in commit `980009f`).
- The `data_dir` vs `operator_config_dir` conflation (already fixed in commit `0d61cc1`).
- Phase 5 operator mTLS / SPIFFE / Corrosion-gossiped revocation (a separate, scheduled body of work).

---

## 2. Initial evidence (verified, file:line + commit)

| # | Evidence | Source |
|---|---|---|
| E1 | `cluster::init` calls `mint_ephemeral_ca()` then `write_trust_triple(&config_dir, endpoint_str, &material)` | `crates/overdrive-cli/src/commands/cluster.rs:91-94` |
| E2 | `cluster::init` hard-codes the recorded endpoint to `https://127.0.0.1:7001` (ADR-0008 default) | `crates/overdrive-cli/src/commands/cluster.rs:81` |
| E3 | `serve::run` constructs `ServerConfig { bind, data_dir, operator_config_dir: args.config_dir }` and calls `run_server(config)` | `crates/overdrive-cli/src/commands/serve.rs:97-105` |
| E4 | `run_server_with_obs` calls `tls_bootstrap::mint_ephemeral_ca()` at line 188 then **unconditionally** writes `tls_bootstrap::write_trust_triple(&config.operator_config_dir, &endpoint, &material)` at line 252, AFTER `TcpListener::bind`. Comment reads: *"Deferred until after bind: a failure before this point leaves no stale config on disk."* | `crates/overdrive-control-plane/src/lib.rs:188, 252` |
| E5 | `walking_skeleton_e2e_round_trips_byte_identical_spec_digest...` uses TempDir A for `cluster::init` (line 98–104) and TempDir B inside `spawn_server()` (line 47) for `serve::run`. The test reads `init_output.config_path` only at lines 105–109 (existence check) and 171–174 (post-shutdown existence check); the live HTTP traffic exclusively uses `server_cfg = config_path(server_tmp.path())` (line 115). | `crates/overdrive-cli/tests/integration/walking_skeleton.rs:46-56, 95-176` |
| E6 | ADR-0010 R1: *"On first `overdrive cluster init` (or its Phase 1 equivalent entry point — the server binary's startup path), the binary generates in-memory: A self-signed CA […] A server leaf certificate […] A client leaf certificate"* | `docs/product/architecture/adr-0010-phase-1-tls-bootstrap.md:43-57` |
| E7 | ADR-0010 R5: *"No cert persistence on disk in the server process (re-init re-mints)."* | `docs/product/architecture/adr-0010-phase-1-tls-bootstrap.md:96-105` |
| E8 | ADR-0010 R4 / recovery clause: *"Recovery on lost client cert is `overdrive cluster init --force`, not a verification-skip."* | `docs/product/architecture/adr-0010-phase-1-tls-bootstrap.md:89-94` |
| E9 | Commit `0d61cc1` ("decouple operator-config dir from data dir") split `data_dir` from `operator_config_dir` to fix the CLI-cannot-reach-control-plane bug, but addressed the *path conflation*, not the double-mint. | `git show 0d61cc1` |
| E10 | Commit `980009f` ("write trust triple with resolved bind address") moved `write_trust_triple` to *after* `TcpListener::bind` precisely so the recorded endpoint reflects the resolved port. This is **why** `serve` writes the triple at all — it needs the resolved port baked in. | `git show 980009f` |
| E11 | `crates/overdrive-cli/CLAUDE.md`: *"`overdrive serve` writes the trust triple after binding the listener, so the recorded endpoint names the resolved port (not the requested bind — which may be `:0` under tests and dev flows)."* — explicit project rule. | `crates/overdrive-cli/CLAUDE.md` (Mechanics §) |
| E12 | The shared default in `commands::cluster::default_operator_config_dir()` resolves to `$HOME/.overdrive` for both commands in production. | Implied by 0d61cc1 commit message; binary wrapper uses the same default for both `init` and `serve`. |

---

## 3. Multi-causal Toyota 5 Whys

Five branches survive the WHY-2 cut. Each is followed independently to WHY 5; cross-validation is at §4.

### Branch B1 — Both commands assume sole ownership of the trust triple

```
WHY 1B1: serve overwrites the file init just wrote on a shared config dir.
  [Evidence E1, E4: both code paths call write_trust_triple unconditionally]

WHY 2B1: Neither command checks for an existing triple before writing.
  [Evidence E1: no read-before-write in cluster::init;
   Evidence E4: no read-before-write in run_server_with_obs;
   the *only* prerequisite gating write_trust_triple in serve is that bind() succeeded]

WHY 3B1: There is no shared write-ownership contract between the two commands.
  Each was authored as if it were the sole author of `~/.overdrive/config`.
  [Evidence E2: cluster::init records `https://127.0.0.1:7001` — the ADR-0008
   default, which would conflict with serve's resolved-port endpoint anyway;
   Evidence E11: CLAUDE.md states "serve writes the trust triple after binding"
   without acknowledging that init also writes it]

WHY 4B1: ADR-0010 R1 names "overdrive cluster init OR its Phase 1 equivalent
  entry point — the server binary's startup path" as the trigger for minting.
  The disjunction was treated by both authors as "either path mints" —
  giving each path a license to mint without coordinating. The ADR never
  reconciled the disjunction into a single owner.
  [Evidence E6: literal ADR-0010 R1 text uses "OR"]

WHY 5B1: ROOT CAUSE B1 — The bootstrap-writer contract is implicit, not
  designed. The whitepaper §8 + ADR-0010 specify what the trust triple
  contains and where it lives, but never specify which subsystem WRITES
  it on which transition. With two equally-plausible owners and no
  arbitration rule, each becomes the sole owner from its own perspective,
  and last-write-wins corrupts the slot whenever both run.
```

**Root Cause B1.** No ownership contract for `<config_dir>/.overdrive/config` writes. The codebase treats it as a *file*; the architecture should treat it as a *resource* with a single writer per transition.

---

### Branch B2 — `run_server` couples three concerns into one bootstrap path

```
WHY 1B2: run_server_with_obs mints a CA and writes a trust triple every
  time, even if a perfectly good triple already exists at config.operator_config_dir.
  [Evidence E4: lib.rs:188 mint, lib.rs:252 write — both unconditional]

WHY 2B2: The reason serve writes the triple at all is that it needs to
  embed the RESOLVED port in the endpoint field — bind(0) yields an
  ephemeral port that init could not have predicted.
  [Evidence E10: commit 980009f's stated intent — "use std_listener.local_addr()
   so the recorded endpoint names the actual bound port"]

WHY 3B2: The "I need to record the resolved port" concern is bundled
  with the "I need to mint a CA" concern in a single atomic bootstrap
  step (mint → bind → write the WHOLE triple). The five fields of the
  triple (endpoint, ca, crt, key, current-context) are written together,
  but only ONE field (endpoint) genuinely depends on bind succeeding.
  The other four (CA + leaf material) could have been minted earlier
  or read from disk — the code does not distinguish.
  [Evidence E4: write_trust_triple takes &CaMaterial as a single bundle;
   Evidence E11: CLAUDE.md explicitly notes "the recorded endpoint names
   the resolved port" — the post-bind concern is endpoint, not material]

WHY 4B2: write_trust_triple's signature accepts an entire CaMaterial as
  a single argument, conflating "what the operator trusts" (CA pin +
  client SVID) with "where the operator connects" (endpoint URL). The
  function has no field-level granularity — it is all-or-nothing.
  [Evidence: tls_bootstrap.rs `pub fn write_trust_triple(dir, endpoint, &material)`
   signature; no "update endpoint only" surface exists]

WHY 5B2: ROOT CAUSE B2 — The write_trust_triple API does not separate
  "produce CA + leaf material" from "record where to reach the server."
  Once those two concerns share an API surface and are written
  atomically, the only way `serve` can update the endpoint after a
  successful bind is to ALSO re-mint everything else. The conflation
  forces re-mint as a *side effect* of needing to record the bound
  port.
```

**Root Cause B2.** Field conflation in `write_trust_triple`. Endpoint-update is impossible without re-minting the whole triple, so `serve` re-mints to satisfy a record-keeping requirement.

---

### Branch B3 — The trust triple format does not split operator identity from server identity

```
WHY 1B3: One file (`<config_dir>/.overdrive/config`) holds: endpoint, CA pin,
  client cert, client key, current-context. Both the OPERATOR (CLI client)
  and the SERVER (control-plane) read/write it.
  [Evidence E6: ADR-0010 R2 — single YAML file, all five fields together]

WHY 2B3: The user's referenced model — Talos — splits this into TWO files:
  - `talosconfig` — the OPERATOR's identity + cluster CA pin (laptop)
  - `machineconfig` — the SERVER's identity, with the operator's CA
    pin embedded so the server knows which clients to trust
  In Talos, these are produced by separate commands at separate times,
  with the operator config flowing into the machine config explicitly.
  [Evidence: user's problem statement]

WHY 3B3: Phase 1 collapses the two roles because the same machine runs
  both `serve` and the CLI in the walking-skeleton single-node model.
  The ADR-0010 design was driven by the localhost-single-node case;
  the multi-machine case is Phase 5+.
  [Evidence E6: ADR-0010 R1 — "process stop discards the CA";
   Evidence E7: ADR-0010 R5 — "Defer rotation / revocation / roles /
   persistence to Phase 5"]

WHY 4B3: Even at single-node the operator and the server have two
  *temporally distinct* concerns: the operator needs to know "which CA
  do I trust?" once (and persistently); the server needs to know "what
  port did I bind on?" every time it starts. Putting both in one file
  makes the second concern (which changes per-boot) corrupt the first
  concern (which the operator wants stable).
  [Evidence E10: 980009f commit — server REWRITES the file post-bind
   to update the endpoint]

WHY 5B3: ROOT CAUSE B3 — The trust-triple file format is single-role
  but the system has two roles (operator vs server) with two write
  cadences (once-at-init vs every-boot). A single-file format with two
  writers and two cadences is a race condition by construction.
  Talos's two-file model is not just operational sugar — it
  architecturally separates the once-written operator artefact from the
  per-boot server artefact, so neither writer can clobber the other.
```

**Root Cause B3.** The single-file trust triple combines artefacts that have different write cadences and different owners. Phase 1 inherited a Talos-shape file name without the Talos-shape file split.

---

### Branch B4 — The walking-skeleton acceptance test uses separate tempdirs

```
WHY 1B4: The walking-skeleton test cannot detect the production-path
  defect because Phase 0 uses TempDir A for cluster::init and Phase 1
  uses TempDir B for serve::run.
  [Evidence E5: walking_skeleton.rs:98 (tmp = TempDir A) vs spawn_server
   line 47 (tmp = TempDir B)]

WHY 2B4: The test was written to validate that `cluster::init` writes
  a file (existence check at lines 105-109) and that `serve` writes a
  file the CLI can use (lines 114-167) — but never that they coexist
  on the same path.
  [Evidence E5: line 171's post-shutdown assertion is on init_output.config_path
   inside TempDir A, never crossed-checked against server_tmp's path]

WHY 3B4: The test uses `spawn_server()` as a shared helper across three
  test functions (lines 95, 182, 217). spawn_server unconditionally
  creates its own TempDir and isolates the server's config dir from
  any caller-supplied dir. There is no overload that lets a caller
  pass init's TempDir into spawn_server.
  [Evidence E5: spawn_server signature `async fn spawn_server() -> (ServeHandle, TempDir)`,
   no parameters]

WHY 4B4: When 0d61cc1 split data_dir from operator_config_dir, the
  walking-skeleton test was updated to use SEPARATE subdirectories
  (`data` and `conf`) under spawn_server's tempdir — but the test was
  NOT updated to share `conf` with the cluster::init invocation that
  ran in Phase 0. The fix made the test SAFER per-step but did not
  exercise the production assumption that both commands target the
  same `conf`.
  [Evidence E5: spawn_server lines 49-50 — `let data_dir = tmp.path().join("data");
   let config_dir = tmp.path().join("conf");`;
   Evidence E9: 0d61cc1 commit message describes the fix]

WHY 5B4: ROOT CAUSE B4 — The test author's mental model was per-test
  isolation (each test gets a clean tempdir), which is correct for
  unit-test discipline but precisely the wrong frame for a
  walking-skeleton end-to-end test that must mirror the production
  flow. The "shared-config invariant" — that init's output IS serve's
  input — was never hoisted into a test assertion. The CI passes
  GREEN on a path that is impossible in production.
```

**Root Cause B4.** The walking-skeleton test asserts on per-tempdir liveness, never on the shared-config invariant the production default actually requires. The test gives a false-GREEN on the integration that matters most.

---

### Branch B5 — Phase 1 explicitly accepts ephemeral CA, yet ships `cluster init` as if the CA persists

```
WHY 1B5: cluster::init exists, mints a CA, writes a triple — yet the
  next command (serve) discards it and mints again.
  [Evidence E1, E4: both mint paths]

WHY 2B5: ADR-0010 R5 explicitly accepts that the CA is ephemeral —
  process stop discards it, re-init re-mints — and that the operator's
  ~/.overdrive/config is the *only* durable artefact. Yet ADR-0010 R1
  still names cluster init AND the server startup path as triggers for
  minting.
  [Evidence E6: R1 disjunction; Evidence E7: R5 ephemeral acceptance]

WHY 3B5: The disjunction in R1 made sense when the design assumed a
  SINGLE process: the same binary either bootstraps fresh (init) or
  resumes (serve), but never both in sequence in the same trust-store.
  The walking-skeleton split into two CLI verbs (`cluster init` vs
  `serve`) created a sequence of two minting events that the ADR
  did not anticipate.
  [Evidence E6: R1 was authored before the verb split was finalised;
   the "OR its Phase 1 equivalent entry point" hedging hints at this]

WHY 4B5: There is NO design pressure in Phase 1 to KEEP a CA across
  process boundaries — the whole point of "ephemeral" is that the
  CA dies with the server process. So `cluster init` minting a CA
  that `serve` will then discard is not just a duplication: it is
  semantically correct from `serve`'s perspective and semantically
  wasted from `cluster init`'s perspective. The tension between the
  two is unforced.
  [Evidence E7: R5 — "no cert persistence on disk in the server process";
   the SERVER side genuinely re-mints by design]

WHY 5B5: ROOT CAUSE B5 — `cluster init` is a Phase 5 / Talos-shape
  ceremony shipped in Phase 1 without the Phase 5 invariants that
  give it meaning. In Phase 5, `cluster init` produces a durable
  operator identity that the server commits to honour
  (Corrosion-gossiped revocation, persistent CA, role-bound SVIDs).
  In Phase 1, none of those invariants exist — the server re-mints
  on every boot — so `cluster init` is performing a ceremony that
  the rest of the system cannot fulfil. The defect is not in the
  code; it is that the verb exists at all in the current phase.
```

**Root Cause B5.** Premature shipping of a Phase 5 ceremony. `cluster init` was added because the operator UX *should* eventually have it, but the Phase 1 server is incapable of honouring its output.

---

## 4. Cross-validation

| Pair | Consistent? | Notes |
|---|---|---|
| B1 ↔ B2 | Yes | B1 says "no ownership contract"; B2 says "API conflation forces re-mint." Both are true; the API conflation (B2) is the *mechanism* through which the missing contract (B1) becomes a destructive write. |
| B1 ↔ B3 | Yes | B1 is "no contract on a single file"; B3 is "single file is the wrong shape." B3 implies B1 — if you split the file, you no longer need a contract at the same granularity. |
| B2 ↔ B3 | Yes | B2 (API conflation) and B3 (file conflation) are isomorphic at different layers. B3 is the architectural framing; B2 is the API surface that crystallises it. |
| B4 ↔ B1, B2, B3 | Yes | B4 (test gap) is *why the bug shipped*. B1/B2/B3 are *why the bug exists*. The test gap is independent of the design defect; both are real. |
| B5 ↔ B1, B2, B3 | Yes — and dominant | B5 reframes B1/B2/B3 as *consequences of a phase mis-scoping*. If `cluster init` did not exist in Phase 1, B1/B2/B3 do not surface. B5 does not contradict the others; it subsumes them at a different level (architectural-scope rather than code-detail). |
| B5 ↔ B4 | Yes | B5 explains why the test gap was tolerable: the team was treating `cluster init` as a Phase 5 placeholder; the test exercises the Phase-1-real path (server-only mint). B4's test gap is the natural consequence — nobody believed the init flow had to *integrate* with serve. |

**All five branches are consistent. The dominant root cause is B5 (premature Phase 5 ceremony).** B1/B2/B3 are mechanism-level; B4 is the test-debt that made the bug invisible to CI; B5 is the framing-level cause that makes the others surfaceable.

### Symptom-coverage check

The observed symptoms are: (a) `cluster init`'s cert is overwritten on the production-default path; (b) the walking-skeleton test passes; (c) the user's local CLI fails after `init` + `serve` against the same `~/.overdrive/`. Forward-tracing:

| Root cause | Produces (a)? | Produces (b)? | Produces (c)? |
|---|---|---|---|
| B1 (no ownership contract) | Yes — both write, last wins | No (test-blind) | Yes |
| B2 (API conflation) | Yes — endpoint update forces re-mint | No (test-blind) | Yes |
| B3 (single-file format) | Yes — one file, two writers, two cadences | No (test-blind) | Yes |
| B4 (test gap) | No | Yes — separate tempdirs hide the conflict | No |
| B5 (premature ceremony) | Yes — `cluster init` runs at all | Yes — Phase 5 framing makes the integration test seem unnecessary | Yes — ceremony issues a CA the server contractually cannot honour |

Every observed symptom is produced by at least one root cause. B5 produces every symptom; B4 explains symptom (b) cleanly that B1/B2/B3 cannot.

---

## 5. Solutions — explicit weighing

The brief mandates explicit evaluation of three named candidates plus the provocative removal candidate. Each is scored against:

- **Closes which root causes?** (B1–B5)
- **Phase-1 fit** vs **Phase 5 reach**
- **Single-cut migration** (per `.claude/rules` — greenfield, no deprecation paths)
- **Cost** (LoC + design surface)
- **Test footprint** (what new tests are required)

### S1. Make `serve` consume an existing trust triple if present; only mint when missing

**Mechanism.** `run_server_with_obs` reads `<config.operator_config_dir>/.overdrive/config` before minting. If a valid triple is found, load CA material from it; if not, mint a new one. Either way, after `bind()`, write the triple back with the resolved-port endpoint (existing material if loaded; new material if minted).

**Closes B1?** Partially — formalises *one* arbitration rule ("disk wins"), but each command still believes it owns the file.
**Closes B2?** No — the API conflation remains; serve still writes the whole triple post-bind.
**Closes B3?** No — single-file format unchanged.
**Closes B4?** No — test must be updated to share a config dir, otherwise the new branch (read-existing) is never exercised.
**Closes B5?** No — `cluster init` still ships and still mints a CA the server may or may not accept.

**Phase-1 fit.** Strong. Smallest change. Matches existing ADR-0010 R5 ("re-init re-mints") because `cluster init` still writes; only `serve` becomes idempotent across reuse.
**Single-cut.** Yes — replace the unconditional mint in `run_server_with_obs` with a load-or-mint helper. Update walking-skeleton test to use a shared config dir. No deprecation needed.
**Cost.** ~30–60 LoC in `run_server_with_obs` + a `read_trust_triple_or_none` helper in `tls_bootstrap` + walking-skeleton test rewrite.
**Test footprint.** New: shared-config invariant test (`init` then `serve` against the same dir; assert client CA pin matches server's leaf chain).

**Risk.** A subtle precedence bug: if `cluster init` wrote at `https://127.0.0.1:7001` (the ADR-0008 default) and `serve` binds on port 0 (tests/dev), the resolved endpoint must overwrite the recorded one. The "load CA, update endpoint, rewrite triple" path is non-atomic and crash-window vulnerable.

**Net.** Surface-level mitigation. Closes the immediate user-visible symptom; leaves B2, B3, B5 unresolved.

---

### S2. Split mint-bootstrap from endpoint-recording

**Mechanism.** Two new APIs in `tls_bootstrap`:
- `write_ca_material(&dir, &material)` — writes ca + crt + key
- `update_endpoint(&dir, &endpoint)` — writes endpoint + current-context

`cluster init` calls `write_ca_material`; `serve` calls `update_endpoint` post-bind. `serve` no longer mints unless it cannot find existing material.

**Closes B1?** Yes — the contract becomes "init owns CA fields; serve owns endpoint field."
**Closes B2?** Yes — the API conflation that *forced* re-mint is removed. Endpoint-update no longer requires the CA.
**Closes B3?** Partially — the file is still single, but the two writers now write disjoint slices. The race becomes a file-locking / atomic-rename concern, not a content-corruption concern.
**Closes B4?** No — test must still be updated to share a dir.
**Closes B5?** No — `cluster init` still ships.

**Phase-1 fit.** Moderate. Requires a new file-format discipline (partial-write atomicity), or full read-modify-write under an advisory lock. Atomic rename works but raises crash-window questions on Phase 1 (what if `serve` crashes between writing CA and writing endpoint? Now you have a file with stale endpoint and fresh CA).
**Single-cut.** Yes — both APIs land at once; old `write_trust_triple` is deleted.
**Cost.** ~80–120 LoC + careful atomicity work + walking-skeleton test rewrite + at least two new acceptance tests (split-write composability, crash-window invariant).
**Test footprint.** Significant: every existing test that mints-and-writes via `mint_ephemeral_ca` + `write_trust_triple` must be split or kept on a `mint_and_write_full` convenience that matches the old shape.

**Net.** Cleaner architecture; clearly the right mid-term shape for B1/B2. Adds Phase-1 surface for atomicity that Phase 5 will overhaul anyway (operator mTLS rotation has its own atomicity story).

---

### S3. Adopt the Talos two-file split (`operatorconfig` + `machineconfig`)

**Mechanism.**
- `cluster init` produces `<config_dir>/.overdrive/operatorconfig` containing CA pin + client cert + client key only. This is the *operator artefact*.
- `serve` reads or generates a `machineconfig` containing endpoint + server leaf material, embedding the operator CA pin from `operatorconfig`. Server can re-mint server-leaf on every boot without touching `operatorconfig`.
- The CLI reads only `operatorconfig` for the trust pin and `<machineconfig>.endpoint` (or just embeds endpoint-discovery via DNS / static config) for the connection target.

**Closes B1?** Yes — separate files have separate single-writer ownership.
**Closes B2?** Yes — endpoint-recording lives in `machineconfig`; CA material in `operatorconfig`. No conflation.
**Closes B3?** Yes — by construction.
**Closes B4?** No — test design still must change.
**Closes B5?** No — but it gives `cluster init` a meaningful Phase 1 output: the operator artefact is now durable across server reboots, which is the Phase 5 promise.

**Phase-1 fit.** Weak. This is a Phase 5 architecture: it implies operator-cert validity beyond a single server-boot, which contradicts ADR-0010 R5 *"No cert persistence on disk in the server process (re-init re-mints)"*. To make it work in Phase 1 you must either:
(a) keep ephemeral semantics — the operator cert is reissued every server boot anyway, and the two-file split has no semantic content (it is a syntactic dressing-up of the same race), OR
(b) shift to a persistent CA in Phase 1, which lifts the Phase 5 work (key rotation, filesystem-permissions discipline, on-disk format) into Phase 1.

The user's own message acknowledges this: *"i am not entirely sure if this is too early for us to do that."*
**Single-cut.** Yes per project rules — but the cut is *much larger*. This is a reshape of the trust-triple format, every test that loads it, every consumer (CLI, server, future Phase 5 RBAC/SPIFFE wiring).
**Cost.** ~300–500 LoC + ADR amendment to ADR-0010 + ADR-0019 (TOML format) + Phase 5 implications need to be re-validated against the new shape.
**Test footprint.** Large: every TLS-bootstrap and trust-triple-loading test rewrites; the walking-skeleton flow is restructured.

**Net.** This is the *right* end-state. It is the wrong work to do *now*. Phase 5 lands operator mTLS, persistent CA, revocation, SPIFFE roles — all of which the two-file split needs to be coherent. Adopting it before those land is shipping a syntactic stub that does not embody the semantic split.

---

### S4 (provocative). Remove `cluster init` from Phase 1 entirely

**Mechanism.** `serve` is the sole minter. On boot it mints a CA + leaves and writes the trust triple to `<operator_config_dir>/.overdrive/config`. The operator runs `serve`, sees the printed config path, and uses the CLI. There is no `cluster init` in Phase 1 — it returns in Phase 5 with the operator-identity ceremony it actually requires.

**Closes B1?** Yes — single writer; no contract needed.
**Closes B2?** Yes — only one path needs the API; conflation is harmless when there is one writer.
**Closes B3?** Partially — file is still single-role-mixed, but with one writer there is no race.
**Closes B4?** Yes — there is no `cluster init` to integrate with `serve`; the walking-skeleton test naturally reduces to the `serve`-only path it already exercises.
**Closes B5?** Yes — directly. The premature ceremony is removed.

**Phase-1 fit.** Excellent. Matches ADR-0010 R5 exactly: *"the operator's `~/.overdrive/config` is the only durable artefact. Losing it is a re-init event, not a recovery event."* In Phase 1, "re-init" simply means "rerun `serve`" — and the operator-recovery story (ADR-0010 R4: *"Recovery on lost client cert is `overdrive cluster init --force`"*) collapses to "stop and restart `serve`."
**Single-cut.** Yes per project rules — delete the `cluster` module from `overdrive-cli`, drop the `cluster init` subcommand from `clap`, delete the related integration test files, simplify `walking_skeleton.rs` to start at Phase 1 (`serve` boot).
**Cost.** Negative — net LoC removal. Estimated −200 to −400 LoC across `crates/overdrive-cli/src/commands/cluster.rs`, the binary wrapper, and integration tests.
**Test footprint.** Reductive. The `cluster_init_serve.rs`, `cluster_and_node_commands.rs` cluster-init paths, and the Phase 0 chunk of `walking_skeleton.rs` either delete or simplify.

**Risks.**
- **Operator UX regression?** Talos users expect `talosctl cluster create`. But Phase 1 has no multi-machine story anyway — the operator who runs `serve` IS the operator who runs the CLI on the same host. There is nothing to ceremonially establish.
- **ADR-0010 R1 amendment.** R1's disjunction (`cluster init` OR server-startup) collapses to just the latter. ADR-0010 needs an amendment removing the `cluster init` arm of the OR. This is a paperwork cost, not a semantic regression.
- **Phase 5 framing.** When `cluster init` returns in Phase 5, it returns with persistent CA, revocation, SPIFFE roles, operator-cert ceremony — i.e. the actual concept the verb names. Removing it in Phase 1 *clarifies* the Phase 5 reintroduction.
- **Loss of the user's mental model.** The user (per problem statement) expects Talos-shape: "you generate a certificate and operator config, and then you'll generate a machine config for the server." Removing `cluster init` does not *contradict* that model — it defers the model's introduction to when it can be honoured.

**Net.** Aligns scope to capability. Removes a feature whose contract Phase 1 cannot satisfy. Frees Phase 5 to introduce the verb properly with persistent CA + machineconfig, exactly as the user described.

---

### Comparison matrix

| Solution | B1 | B2 | B3 | B4 | B5 | Phase-1 fit | Cost | Single-cut |
|---|---|---|---|---|---|---|---|---|
| S1 — load-or-mint in `serve` | Partial | No | No | No (test fix needed) | No | Strong | ~50 LoC | Yes |
| S2 — split mint vs endpoint API | Yes | Yes | Partial | No (test fix needed) | No | Moderate | ~100 LoC + atomicity | Yes |
| S3 — Talos two-file split | Yes | Yes | Yes | No (test fix needed) | No | Weak (Phase 5 work) | ~400 LoC + ADR rewrites | Yes (large cut) |
| **S4 — remove `cluster init` in Phase 1** | **Yes** | **Yes** | **Yes** | **Yes** | **Yes** | **Excellent** | **−300 LoC** | **Yes** |

S4 is the only solution that closes all five root causes, has negative cost, and aligns scope to capability. S1 is the smallest patch but leaves B2/B3/B5 unaddressed and is structurally fragile. S2 is good architecture for the wrong phase. S3 is the right end-state but Phase 5 work.

---

## 6. Backwards-chain validation

For each candidate, trace forward: would the proposed solution have prevented the observed defect, and *every* root cause it claims to address?

### S1 (load-or-mint in `serve`)

Trace:
- Operator runs `cluster init` → triple T1 written to `~/.overdrive/config`.
- Operator runs `serve` → load T1 (succeeds), bind succeeds at resolved port, write back T1 with updated endpoint.
- CLI reads T1 → CA pin and SVID match the server's leaf chain → handshake succeeds.

Result: defect symptom prevented. **But:** if the operator runs `serve` twice in succession (e.g. restart), the second `serve` boot now reuses the existing CA from T1 — but the server-side runtime does not retain the CA private key from the previous boot (it was minted in-process and discarded). The reused operator-cert in T1 is no longer signable by anything the new server can present. In other words, S1 introduces a *new* failure mode: stale operator material on consecutive `serve` boots. Either S1 must re-mint when the in-process CA-private-key-state is missing (which collapses S1 back to the current defect at restart), or S1 must persist the CA private key on disk (which is Phase 5 work per ADR-0010 R5).

S1 closes the symptom on the *first* `init`+`serve` sequence but breaks on the *second* `serve` boot. **S1 fails backwards-chain validation.**

### S2 (split mint vs endpoint API)

Trace:
- `cluster init` writes CA fields (T1.ca, T1.crt, T1.key) only.
- `serve` reads CA fields, mints server-leaf in-process (private key memory-only), bind succeeds, writes endpoint field (T1.endpoint) only.
- CLI reads T1 → trusts T1.ca → handshake against server's freshly-minted server-leaf → succeeds because server-leaf is signed by T1.ca's CA *which was generated by cluster init* and whose private key is **on disk in T1.key** (the operator client key, not the CA key).

Wait — for the server to mint a server-leaf signed by the same CA across boots, the CA private key must persist. ADR-0010 R5 forbids that. So S2 has the same defect as S1: it formally separates the writes, but the underlying CA-private-key-discard semantics make the operator's CA pin (T1.ca) refer to a CA the next-boot server cannot speak for.

S2 only works if the CA private key is persisted, which is exactly the Phase 5 boundary. **S2 fails backwards-chain validation in Phase 1.**

### S3 (Talos two-file split)

Trace:
- `cluster init` generates `operatorconfig` containing operator key + a CA-pin reference.
- `serve` generates `machineconfig` (server identity), embeds `operatorconfig.ca_pin` as the trust anchor.
- Operator CLI reads `operatorconfig` → CA pin matches server's machineconfig → handshake succeeds.

Same problem: for `operatorconfig.ca_pin` to remain valid across `serve` boots, the CA private key *must persist*. Without persistence, every `serve` boot mints a new CA, and the operator's `operatorconfig` carries a now-stale pin. The split fixes the *file-write race* but not the *underlying ephemeral-CA semantic*.

**S3 fails backwards-chain validation in Phase 1** — it presupposes Phase 5's persistent-CA discipline.

### S4 (remove `cluster init` in Phase 1)

Trace:
- Operator runs `serve` → mints CA + server leaf + client leaf in-process; writes full trust triple T1 to `~/.overdrive/config`.
- Operator runs CLI → reads T1; T1.endpoint matches server, T1.ca trusts server-leaf, T1.key is the freshly-minted operator key paired with T1.crt → handshake succeeds.
- Operator restarts `serve` → mints fresh CA + leaves; writes T1' over T1. Operator's CLI reads the new T1' → handshake succeeds.

No race. No stale material. The operator's *expectation* of "I configure the cluster, then it stays configured" is replaced by the Phase-1-honest contract: "the cluster is ephemeral; (re)start the server and the CLI will pick up the fresh config from `~/.overdrive/`." This matches ADR-0010 R5 word for word: *"the operator's `~/.overdrive/config` is the only durable artefact. Losing it is a re-init event, not a recovery event."*

Forward trace covers every observed symptom (a/b/c) and every root cause (B1–B5). **S4 passes backwards-chain validation cleanly.**

### Test design (independent of S1–S4)

Regardless of which solution is chosen, B4 must be addressed: the walking-skeleton test must include an assertion that the server-leaf chain validates against the operator CA pin from the *same* config file the CLI is reading. The current test asserts only path-existence and spec-digest round-trip. A handshake-validity assertion against a **shared** `<operator_config_dir>` is the missing test invariant. This is a separate cut from the solution choice; it is a test fix, not an architecture fix.

---

## 7. Recommendation

**Adopt S4 (remove `cluster init` from Phase 1) as the structural fix.** Pair it with a tightening of the walking-skeleton test (B4) so the shared-config invariant is asserted in CI on the `serve`-only path.

Concretely the structural change is:

1. Delete `crates/overdrive-cli/src/commands/cluster.rs::init` and the `cluster init` subcommand wiring (clap definition, binary entrypoint).
2. Amend ADR-0010 to remove the disjunction in R1 — `serve` is the sole minter in Phase 1; `cluster init` returns in Phase 5 with the persistent-CA + operatorconfig/machineconfig split the verb requires.
3. Reduce `walking_skeleton.rs` to its Phase-1 honest shape: skip the Phase 0 `cluster init` step; assert the trust triple `serve` writes is the one the CLI reads (single config path).
4. Add a new acceptance test asserting that the operator CA pin in the on-disk trust triple validates against the server-leaf chain on the live socket — this defends against a *future* regression where serve writes one CA and presents a different leaf.

When Phase 5 lands (operator mTLS, persistent CA, Corrosion-gossiped revocation, SPIFFE roles), `cluster init` returns with the Talos two-file split (S3) and the API granularity (S2) — but those are Phase 5 work, with the invariants that make them coherent.

If Phase 1 must keep `cluster init` for *operator UX continuity reasons external to this analysis* (the brief does not indicate any, and ADR-0010 R5 implies the opposite), the smallest viable fallback is **S1 plus a hard precondition**: `cluster init` may only run *before* `serve` has ever started, and re-running `cluster init` requires `--force` (already reserved per ADR-0010 §R4). This makes the contract "init writes once; serve writes thereafter" — and the dominant root cause B5 is downgraded to a documented constraint rather than removed.

---

## 8. Brief for the user (2–3 sentences)

The dominant root cause is **B5**: `cluster init` is a Phase 5 / Talos-shape ceremony shipped in Phase 1, but Phase 1's ephemeral-CA contract (ADR-0010 R5) makes it impossible for the server to honour the operator artefact `cluster init` produces. The proximate mechanisms — no shared-write contract (B1), endpoint-and-CA conflation in `write_trust_triple` (B2), single-file format (B3), test that uses isolated tempdirs (B4) — all dissolve when `cluster init` is removed in Phase 1.

**Recommended next step:** open an architect-led ticket to remove `cluster init` from Phase 1 (delete the verb; amend ADR-0010 R1 to make `serve` the sole Phase 1 minter; reduce the walking-skeleton test to start at `serve`). Defer the Talos two-file split to Phase 5 where persistent CA + revocation + SPIFFE operator roles make it semantically real. This is a single-cut migration consistent with greenfield discipline; estimated net code change is negative (deletion).
