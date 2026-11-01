# Quint website content review

## Scope and verdict

**APPROVED.** Independent review of the uncommitted Quint blog, concept, procedural guide, related DST edits, and navigation. No blocking factual, editorial, command, link, or discovery defect was found.

Reviewed on 2026-11-01 under `AGENTS.md`, `website/CLAUDE.md`, `.claude/rules/finalize.md`, `.claude/rules/domain-naming.md`, and `.claude/skills/technical-writer/SKILL.md`. The technical-writer principles, decision-story completeness requirement, and prohibition on semicolons apply to the new and revised prose.

The complete authored scope is:

- `website/content/blog/what-quint-can-prove-about-a-flow-owner.mdx`.
- `website/content/docs/concepts/quint-modeling.mdx`.
- `website/content/docs/how-to/check-quint-models.mdx`.
- The existing deterministic-simulation concept and DST harness guide.
- The concept and how-to navigation metadata.

The editorial ledger is `.context/website-quint-modeling.md`. This reviewer wrote only this artifact and preserved the user's dirty `AGENTS.md` and the writer's changes. No model, source, test, policy, dependency, lockfile, DES log, or recorded model evidence was edited. No commit, push, publication, deployment, model checker, Rust test, Lima, Metal, workload VM, native kernel, or Cloudflare operation was run.

## Decision narrative and document roles

The blog has a concrete thesis and motivating example. Coalesced reads of a latest-row observation store can hide a `Fail`, `Pass`, `Fail` boundary. A consumer that reconstructs the streak from skipped sequence numbers can terminate early. The post then explains the actual producer-owned alternative, the check that distinguishes the two rules, and the limits of that model evidence.

This is a documented design comparison from ADR-0191, not an invented history of choosing Quint over another language or tool. The later watch-before-stream-handoff example explains why conformance can correct a model premise. Native backpressure and TCP Fast Open counterexamples explain why a consistent abstract model cannot settle kernel behavior. The final evidence discussion preserves the known CID replacement exclusion. These are consequential decisions and discoveries rather than a list of delivery tasks.

The concept page explains current abstractions, backends, hazards, witnesses, conformance, and evidence boundaries. The how-to provides the real repository workflow for editing a protocol/check pair, selecting checks, recording a complete subsystem, and checking freshness. Its procedures remain distinct from the concept explanation. The DST edits improve the sampled-trajectory qualification and connect the two workflows without presenting either as exhaustive proof of the Rust implementation. No unrelated rewrite is required.

## Factual and interface assessment

| Content claim | Authoritative evidence and result |
|---|---|
| Latest-row coalescing makes consumer-side streak reconstruction invalid. The producer owns streak counting and the consumer applies thresholds and records the acted-on row. | ADR-0191's context, decisions, and alternatives explicitly establish this ownership choice. `specs/quint/probe-count/README.md` and `probe_count.qnt` encode one producer, one consumer, latest-row merge by sequence, missing-sample inertia, and retry on a newer crossed row while still running. The blog's three-outcome example matches the reconstruction hazard. |
| `StreakExact` and `FlipBetweenLooksNoEarlyTerminate` are actual model excerpts. | Independently compared both fenced excerpts with contiguous `probe_count.qnt` source after removing only the source block's common indentation. Both match exactly. The excerpts are identified as excerpts, not a standalone executable module. |
| The selected rule passes the named run while the reconstruction hazard fails it as expected. | The normal/hazard manifest entries select the same named run against different mains. The retained `evidence/traces/probe-count-run-hazard-reconstruct.txt` contains QNT508 assertion failure for that exact run. The post accurately calls this expected negative evidence rather than an unresolved production failure or a new reproduction of a production defect. |
| Seven producer runs and threshold three are model bounds, and written-outcome history is model-only evidence. | Probe-count README abstraction and projection sections and `probe_count.qnt` constants/ghost state support those claims. The post does not mistake model history for production persistence. |
| TLC, Apalache, named evaluator runs, and randomized traces have distinct result boundaries. | `xtask/src/quint.rs:231`, `:367` distinguish the backends and reject temporal Apalache checks. Local resource code distinguishes finite explicit-state enumeration from bounded symbolic checking. Official [model-checker documentation](https://quint.sh/docs/model-checkers) and [protocol lesson](https://quint.sh/docs/lessons/anatomy) independently support the language and search distinctions. The text confines guarantees to configured instances, assumptions, bounds, and completed searches. |
| Liveness permits a reader to stop reading and conditions delivery on stall/cutoff behavior. | `specs/quint/guest-flow-owner/fwd_pair.qnt:1281` defines the finite-instance fairness treatment, `stalled`, and `cutOff`. The blog does not assume every reader eventually cooperates. |
| Rust conformance drives production owners and compares observations, with named and randomized modes. | Guest-flow-owner README documents conformance mains and action recording. `crates/overdrive-init/src/guest_flow/quint_udp_slots.rs` contains real action handlers, `#[quint_test]`, `#[quint_run]`, the qualified `Config::nondet` path, and projection from `guest_observation()`. The host flow-pair driver similarly reads `flow_observation()`. The prose correctly rejects driver-owned expected bookkeeping as independent implementation evidence. |
| A live watch and a caller-held stream are different model facts. | The feature delta's stream-handoff correction separates `strmHeld` from watch liveness, including abort before handoff. The post describes a wrong model premise corrected to match the existing ownership contract. It does not invent production remediation to satisfy that premise. |
| `conf_step` is write-only recording instrumentation and stays constant on checking instances. | Probe-count and guest-flow-owner README contracts and recording-main definitions support the REC distinction and qualified action-recording paths. The prose does not introduce a public API or new instrumentation mechanism. |
| Pending TCP capture has two abstract places and four children, without an added quint-connect driver. `fwd_safe` is also outside Rust conformance coverage. | The TCP capture pending-place README explicitly pins these bounds, exclusions, and its paired existing native cell. The guest-flow-owner module table identifies the lock/RCU model's different boundary. The content does not turn small abstract counts into evidence about 4,096 live kernel objects or claim automatic refinement. |
| The queued-end audit correction is seeded Rust evidence, while kernel backpressure and hook ordering require native evidence. | The approved networking review and retained implementation records distinguish the seeded owner/supervisor regression from abstract inventory modeling and native kernel effects. The blog preserves these distinctions and the disposable-guest boundary. It makes no throughput, density, memory, or cold-start measurement claim. |
| Ordinary CID lease conformance excludes a known generation-replacement crash window. | `specs/quint/guest-flow-owner/cid_lease.qnt:514` and its README describe the guard, separate pending configuration, and issue 325. The blog and concept explicitly preserve the exclusion rather than claiming a universal green implementation. |

## Commands, toolchain, and recording

The guide's command shapes match `xtask/src/main.rs:303` and the implemented runner:

- `typecheck`, `check`, and `verify-evidence` are existing subcommands.
- `--subsystem`, exact `--name`, `--ci`, `--jobs`, and `--record` have the documented meanings. The named `probe-count-safety` entry exists.
- The example `[[check]]` table matches an actual entry in the probe-count manifest exactly after TOML parsing.
- The runner requires evaluator backend for named runs and TLC for temporal checks.
- Resource admission reserves CPU and memory. `--jobs 3` is a concurrency ceiling, not a promise of three simultaneous jobs.
- `--record` requires one full subsystem and refuses `--name` and `--ci`. `xtask/src/quint/evidence.rs` stages and replaces the evidence directory even when results miss expectations. The guide explicitly states that recording does not turn failure into acceptance evidence.
- `verify-evidence` checks recorded inputs, expected outcomes, and required traces. It does not rerun checkers or validate a Rust binary. Earlier evidence belongs in Git history rather than mixed run directories.

The local pin Quint 0.32.0 comes from `infra/provision/install-quint.sh:37`. Its Node/JVM provisioning and Apalache 0.56.1 pin establish the toolchain described by the guide. Exact local behavior was checked against source rather than inferred from upstream HEAD. Commands consistently use the repository's provisioned Lima wrapper. The guide supplies no direct shared-host kernel test command and accurately distinguishes abstract model execution from workload-VM and networking validation. None of these documented commands was executed during review.

## Retained model evidence freshness

Independent read-only SHA-256 comparisons recomputed every recorded input hash from the current files. Every recorded check has `pass = true` at its stated expectation.

| Subsystem | Retained run | Expected outcomes | Current input hashes |
|---|---|---|---|
| guest-flow-owner | `2228824-1793511799.752` | 279 total, 209 holds, 70 violations | 28/28 match |
| probe-count | `128344-1792609895.793` | 31 total, 19 holds, 12 violations | 3/3 match |
| tcp-capture-pending | `166979-1792763128.074` | 8 total, 6 holds, 2 violations | 3/3 match |

This is a freshness inspection of retained evidence, not a fresh `verify-evidence` invocation or model run. The public prose gives these counts their correct meaning and does not combine them into a count of universal system proofs.

## Website validation

The ledger records passing typecheck, lint, production Next build, local OpenNext build, and one-index validation. Those full builds were not redundantly rerun by the reviewer. No package or lockfile change is part of the content diff.

Independent targeted checks:

| Check | Result |
|---|---|
| `git diff --check` | PASS |
| One-index verifier against retained current build | PASS, 28 published pages including six blogs, one draft excluded from all three consumers |
| Exact two blog source excerpts and guide manifest excerpt | PASS |
| Site-local content links | PASS, all 15 targets exist |
| Navigation metadata | New concept and guide included in their existing navigation lists |
| Semicolon scan | PASS, zero across all five authored/revised MDX files, including metadata and examples |
| New blog, concept, and guide HTML and Markdown exports | PASS, all six responses returned 200 and expected Quint content |
| Blog listing and llms export | PASS, listing includes the new blog and llms includes all three new pages |
| Search discovery | PASS, `/api/search?query=Quint` reaches all three new pages |

HTTP smoke used the retained production Next build on local port 8996. The reviewer stopped the server after the checks. This verifies website rendering and discovery, not model or system correctness.

The existing deterministic-simulation search-ranking failure remains qualified by the preceding current/pristine-baseline reproduction in `review-website-content.md`. The writer still observed the same first hit in this pass. Full blog/MCP/analytics suites were not rerun, and no pass is claimed for analytics branches beyond the existing failed assertion. The new page discovery and one-index results show no additional observed indexing failure. They are not a universal search-ranking guarantee and do not justify an unrelated search fix.

Initial direct fetches of official Quint documentation timed out. Official indexed documentation extracts were subsequently available and support the stable engine and language semantics cited above. Repository pins and exact runner commands were verified locally. The new repository-source links name existing local paths. No claim about external repository publication or branch synchronization is made.

## Findings and disposition

No material finding requires remediation. The blog supplies an evidenced design comparison and concrete failure schedule, the docs describe current mechanisms and useful commands, and the evidence boundaries remain explicit. No new test scenario, model execution, kernel experiment, or architecture change is required by this content review.

## Final verdict

**APPROVED — Quint modeling blog and documentation.** This is approval of the authored website content and its bounded validation. It is not a new model proof, implementation conformance result, native-kernel receipt, feature finalization, or publication authorization.

## Review iteration 2 — define Quint before the example

### Scope and assessment

Reviewed the actual opening-only blog diff and the editorial ledger's correction entry. The user requires a brief definition first. The earlier placement of the definition after the probe example did not meet that reading order, and the prior review did not catch the missing introduction for an unfamiliar reader.

The first body paragraph now identifies Quint as a language and toolset for executable system models. It immediately explains state, actions, rules, exploration, and counterexamples in plain language. The next sentence states why Overdrive uses it, with service restart decisions as the concrete connection. The existing probe-count example follows naturally. The reader can understand what Quint does before encountering that example.

The definition agrees with the official protocol and model-checker sources verified in iteration 1. It confines exploration and counterexamples to the model and does not claim exhaustive exploration by every tool, automatic Rust conformance, native-kernel correctness, or a whole-system proof. The unchanged article supplies the detailed backend and evidence distinctions. No new semantic claim requires a model run or a fresh toolchain investigation.

### Validation and disposition

Independent comparison against HEAD confirms that removing the six added opening lines reproduces the prior article exactly. Metadata, links, examples, and the remainder are unchanged. The complete revised article contains zero semicolons. `git diff --check` passes. The writer records passing MDX generation and targeted opening/link checks, with no claim of a repeated full website suite. No model, Rust, native, or website server run was needed for this bounded addition.

| Finding | Disposition |
|---|---|
| Quint must be briefly defined before the probe-count example | **CLOSED.** The definition is the first body paragraph, followed by Overdrive's purpose and then the example. |

Only this review artifact was edited by the reviewer. The user's dirty `AGENTS.md` and the writer's blog revision remain preserved. Earlier evidence and runtime-validation limitations remain unchanged.

### Iteration 2 final verdict

**APPROVED — opening clarification.** The revision directly satisfies the requested reading order, accurately defines Quint, and preserves the previously reviewed model, conformance, and native-evidence boundaries.
