# Evolution — Human-readable `[resources]`: `cpu` and `memory` as typed strings

**Feature:** `workload-resources-human-readable` ([GH #372](https://github.com/overdrive-sh/overdrive/issues/372)). **Finalized:** 2026-11-21.
**Scope:** the workload spec surface and the types behind it. `[resources]` takes `cpu = "500m"` and `memory = "128MiB"` instead of `cpu_milli` and `memory_bytes`, end to end from the TOML spec to the driver. Three implementation steps plus a review-fix commit.
**Waves run:** DISCUSS (2026-11-20), DESIGN (2026-11-20, user approval the same day), DISTILL (2026-11-20), DELIVER (2026-11-20 to 2026-11-21). No DIVERGE.
The [feature contract and wave history](../feature/workload-resources-human-readable/feature-delta.md), [roadmap](../feature/workload-resources-human-readable/deliver/roadmap.json) and [execution log](../feature/workload-resources-human-readable/deliver/execution-log.json) remain the delivery records. The [spike findings](../feature/workload-resources-human-readable/spike/findings-v1-newtype-layout.md), the [design reviews](../feature/workload-resources-human-readable/design/) and the [DISTILL scenarios and RED classification](../feature/workload-resources-human-readable/distill/) sit beside them.

## Summary

An operator sizing a workload no longer computes bytes or milli-units. `[resources]` takes two TOML strings, `cpu` and `memory`. `memory = "128MiB"` and `cpu = "500m"` (or decimal cores, `"0.5"`, `"2"`) are parsed by two hand-built `overdrive-core` newtypes, `ByteSize` and `CpuQuantity`. Each has one canonical spelling: `"1024MiB"` is stored and shown as `1GiB`, `"0.5"` as `500m`, and equal values spelled differently produce the same spec digest. `overdrive workload describe` prints `Memory:` and a new `CPU:` line in the canonical form. The old keys `cpu_milli` and `memory_bytes` are refused by the existing `deny_unknown_fields` check, with no alias and no fallback. A unit-less bare number for `memory` is refused. A workload `memory` that is not a whole number of MiB is refused at `deploy`.

The typed values run from the TOML spec through the intent aggregate, the wire bodies, node capacity, the scheduler and the `NoCapacity` prose, to `vcpus_for`, `cpu_weight_for` and `MemoryPlan::derive` at the driver. The persisted workload intent's archived layout is unchanged, so no envelope version was added and the existing golden fixtures decode unchanged. `VolumeSize` (shipped by #348) is rebuilt as a `ByteSize` refined to whole GiB.

## Business context

[#372](https://github.com/overdrive-sh/overdrive/issues/372) traces to `J-OPS-002` (trust what the CLI tells me) and secondarily `J-OPS-003` (VM workload sizing). Before this feature one dropped digit (`memory_bytes = 13421772` instead of `134217728`) was a valid but wrong 12.8 MiB guest, and `describe` printed a raw byte count, so the operator could not see the mistake without dividing. `cpu_milli = 500` against `cpu_milli = 0.5` had the same shape. The state was real on the production path: `overdrive deploy` parsed `[resources]` as two unit-in-the-name integers. The persona is Ana Ortiz, the platform engineer who authors Job, Service and VM specs.

## Key decisions

Every DESIGN decision was put to the user and approved on 2026-11-20. The ADRs read Accepted.

**User rulings (DISCUSS).**

| Ruling | Outcome and reason |
|---|---|
| R-1 | `memory` and the volume size share one byte-size grammar, hand-built in `overdrive-core` with no new crate dependency. `bytesize` and `byte-unit` were rejected |
| R-2 | Node capacity (`NodeSpecInput`) moves to the same newtypes and unit-free names in this feature |
| R-3 | `cpu` is a Kubernetes-style quantity (`500m`, decimal cores), hand-built. `k8s-openapi` and `kube_quantity` were rejected |
| R-4 | Rebase ruling, option a: keep R-1 after #348 DELIVER landed on main. ADR-0310 became an operative contract, so this feature changes the shipped `VolumeSize` parser and the #348 test expectations it reverses, by an explicit dated amendment. No hand-off to a #348 owner |

**DESIGN decisions.**

| Decision | Outcome | ADR |
|---|---|---|
| DD-1 | `memory` and the volume size share one binary `ByteSize` grammar | [0339](../product/architecture/adr-0339-memory-and-volume-sizes-share-one-binary-byte-size-grammar.md) |
| DD-2 | The volume size is a `ByteSize` refined to whole GiB, at least 1 GiB. `1024MiB` is accepted and reads back `1GiB`, `512MiB` is refused. Accepted set grows, every previously accepted input keeps its value and `Display`, and the archived `u64` layout is unchanged | [0310](../product/architecture/adr-0310-vm-volume-is-declared-by-a-vm-volume-table-with-a-size.md) § Amendment 2026-11-20 |
| DD-3 | `cpu` is a `CpuQuantity`: millicores or decimal cores, at most three fractional digits, `u32` milli ceiling, lowercase `m` suffix | [0340](../product/architecture/adr-0340-cpu-is-declared-as-a-millicore-or-decimal-core-quantity.md) |
| DD-4 (decision 1a) | `Resources`, both `ResourcesInput` twins and `ResourcesBody` carry `cpu` and `memory`, renamed together in one struct. Node capacity follows (R-2) | [0341](../product/architecture/adr-0341-workload-resources-carry-the-typed-quantities-from-spec-to-driver.md) |
| DD-5 (decision 2) | No envelope version is added: the archived layout is unchanged. Validation item V-1 was run as a spike during DESIGN and held | 0341 |
| DD-6 (decision 3a) | The API carries the canonical string, not a structured object | [0342](../product/architecture/adr-0342-the-api-carries-resource-quantities-as-their-canonical-string.md) |
| DD-7 (decision 4a) | `describe` prints `CPU:` beside `Memory:`, under the same presence guard. Zero CPU renders as `0`. No ADR: it is an operator output line | feature delta |
| DD-8 (decision 5a) | A workload `memory` is a whole number of MiB, refused at the aggregate admission site. Cloud Hypervisor rejects a guest size that is not a multiple of the host page size, so `memory_bytes = 1000` passed `deploy` and failed at VM launch | [0343](../product/architecture/adr-0343-a-workload-memory-is-a-whole-number-of-mib.md) |

Decisions DD-3 and DD-8 on the cut: `Schedule::from_submit` is still a `todo!()` scaffold, and when it lands it routes through the same admission, so DD-8 applies to a Schedule too. Rejected: unifying `ByteCount` (free space, `<n>B`) into `ByteSize`, which would rewrite #348's shipped type for no requirement here. The Cloud Hypervisor `--memory size=` argument stays a raw byte count from `MemoryPlan::guest_bytes()`: a `ByteSize` `Display` is never passed to a foreign protocol.

Design and DISTILL reviews: adversarial, subtractive and verification rounds (A1 to A10, S1 to S17, R2-1 to R2-5 in the feature delta) and DISTILL round 1 (S-H1 to S-L12). Dispositions are recorded there. The DISTILL round cut the test set (merged matrices, deleted a guest-placed round-trip file and a docs-name check) and narrowed O08 to the two claims no assertion holds.

## Work completed per step

Source: `deliver/execution-log.json` and `git log`. All three steps reached COMMIT with status PASS.

| Step | What landed | Commit |
|---|---|---|
| 01-01 | `ByteSize` and `CpuQuantity` grammars; `VolumeSize` refines `ByteSize`. 28 DISTILL tests activated | `caed426be` |
| 01-02 | Typed `cpu` and `memory` from spec to driver: aggregate, wire bodies, node capacity, scheduler, `describe`, `vcpus_for`, `cpu_weight_for`, `MemoryPlan::derive`, `api/openapi.yaml` regenerated. 27 DISTILL tests activated | `182273b25` |
| 01-03 | Examples, scripts and docs spelled as `cpu` and `memory` strings: 33 example specs, `run-example.sh`, `verification/**/runner.sh`, `benchmarks/catalogue.md`, `brief.md` example | `034a30a2c` |
| Review fixes | Adversarial F1, F2, F5 and subtractive S1, S3, S4, S5, S7, S9, S12 | `54223f20f` |
| Execution log | DES integrity verification passed | `10627a0bb` |

Wave and finalize commits: `7492182e9` (DISCUSS and DESIGN), `a8f8f6871` and `b2e6ece1b` (DISTILL tests and the frozen V1 fixtures decoded after main's V2 envelope), `856638f0b` (ADR-0310 amendment), `e0c5025e6` (Cloud Hypervisor cited at the commit main pins), `a3116b92a` (volume size refusals re-drawn under the shared grammar), `4f019df7b` (roadmap), `e9b6f909d` (DELIVER results), `f1fd3e907` and `c45016871` (website), `23cc4feb6` (O08 capture and review).

About 300 files changed, mostly mechanical: about 242 Rust sources, the hydration golden and trajectory fixtures, raw TOML/JSON literals that a compile does not find (they fail at run time under `deny_unknown_fields`), and four sim fixtures that submitted 256 KiB memory and were raised to at least 1 MiB because DD-8 refuses them.

## Verification

- **DISTILL tests:** 55 pre-authored acceptance tests, each `#[should_panic(expected = ...)]` and classified MISSING_FUNCTIONALITY, all activated and passing: 28 in step 01-01 and 27 in step 01-02. Nine of them stop at the string-wire decode until the rename step, as `red-classification.md` records.
- **Full workspace suite on Metal:** 5162 of 5164 passed. The two failures are unrelated to this feature. `cluster_status_against_in_process_server…` is flaky under load and passes alone. `public_ingress_gateway_answers_503_until_the_hydrator_reprograms_after_a_serve_restart` fails on main before this feature (it stays 502 after a serve restart).
- **Mutation, per step as recorded in the feature delta:** 01-01, 29 of 29. 01-02: core 13 of 13, worker 5 of 5, cli 4 of 4, reconcilers 7 of 7; control-plane had one diff-scoped mutant, unviable, so it gives no signal. Review-fix commit, value-type mutation 30 of 32 (93.8%); the two survivors replace `ByteSizeParseError::input()`, and no test asserts the input text. 01-03 had no source to mutate; its gate was the grep gate (`cpu_milli|memory_bytes`, zero matches outside the named exceptions).
- **Reviews:** adversarial CHANGES_REQUESTED with 0 blockers (F1: a density-test comment corrected after tracing that the memory check counts only the placed workload's own allocations; F2: `Hash` removed; F5: volume error text). Subtractive: S1, S3, S4, S5, S7, S9, S12 applied, S2 declined.
- **Spike V-1:** a struct holding `CpuQuantity(u32)` and `ByteSize(u64)` archives to the same bytes as one holding the primitives, and old bytes cross-decode (Metal run `2290cbebcd6604db77be3067`, kernel 7.0.0-38). This is what let DD-5 add no envelope version.
- **EDD expectation O08** (`verification/expectations/O08-workload-resources-human-readable-refusal-and-describe/`): **satisfied**, captured 2026-11-21 on native Metal at SHA `c450168716b01ab34d7c93fce56ba95c9aadca70`, reviewed by an independent agent. Five contrasting refusals (`memory = 134217728`, `"128MB"`, `cpu = "500M"`, `"1000B"`, `memory_bytes`) each exit 2 with text naming the key or the form; `describe` reads `128MiB`/`500m` and, for `"1024MiB"`/`"0.5"`, `1GiB`/`500m`. Caveat recorded in the review: both accepted workloads ended `Failed (backoff exhausted)` because the guest's transport setup failed with `creating the transport dummy device failed: Not supported`, an environment matter unrelated to resources. Satisfied for that SHA and substrate only; only one refused case was checked for "no allocation exists" (`describe` returned 404).

## Documentation changed

Website: `website/content/docs/reference/workload-spec.mdx` (new, 66 lines: the `[resources]` grammar, canonical forms, refusals and the whole-MiB rule; `reference/meta.json` updated), `how-to/deploy-a-workload.mdx`, `reference/cli.mdx`, `concepts/microvms.mdx` and `website/app/(home)/page.tsx`. `docs/product/architecture/brief.md` carries the new spelling. **No blog post, by the user's decision.** The retained evidence under `verification/**/evidence/`, `benchmarks/results/` and earlier evolution records keep the old names as immutable record.

## Issues encountered and lessons

- **The issue's reuse premise was false, caught in DISCUSS.** #372 said `memory` reuses `VolumeSize` and `ByteCount`. Neither parses `128MiB`: `VolumeSize` is `<uint>` followed by `GiB` or `TiB`, and `ByteCount` is `<n>B` only. Checking the shipped code before accepting the premise turned a "reuse" into a new shared grammar with the volume refining it. Read the code a ticket says to reuse.
- **The Cloud Hypervisor page-alignment constraint was found only by reading the vendored source.** Guest RAM must be a multiple of the host page size, or the VMM refuses it with `MisalignedMemorySize` (`vmm/src/memory_manager.rs:800-802` at the fork commit main pins). Without that reading, the grammar would have accepted `memory_bytes = 1000`-class values that fail at VM launch. It became DD-8 and ADR-0343, with the evidence section in the feature delta.
- **"The Metal box cannot boot nested guests" was a misdiagnosis.** The branch was simply behind main's `nested=on` guest-kit shim. The fix was a rebase. Two DISTILL commits were made with `--no-verify` on that wrong premise. Check whether the branch is behind main before concluding the environment is broken; and a hook failure is a signal to find the cause, not a reason to bypass it.
- **#348 shipped mid-feature.** What DESIGN planned as an in-place ADR-0310 correction became, once the volume grammar was operative, an explicit dated amendment that keeps the accepted text and states compatibility (R-4). The `design.md` rule on manufacturing amendment history only before implementation drew the line. The #348 artifacts were left unedited as history, and the amendment is the pointer.
- **A compile does not find old keys in raw string literals.** Under `deny_unknown_fields` they fail at run time. The decomposition listed the four raw-literal files, the live runner script and the non-`.rs` fixtures up front, which is why the grep gate, not the compiler, closed the sweep.
- **Sub-MiB fixtures are not mechanical.** Four sim tests and the proptest strategies that draw arbitrary `u64` memory had to be re-drawn to whole MiB, and a stale density-test comment survived until the adversarial review.

## What was not done

- Mutation on the control-plane crate gave no signal (one unviable mutant). The two surviving value-type mutants (`ByteSizeParseError::input()`) are unasserted.
- `describe` for four of the five O08 refusals was not individually checked for "no allocation exists".
- No blog post, by decision.
- The full suite's two failures were not investigated here; one is flaky under load and one predates this feature.

## Scope limits and deferrals

None introduced. Binary units only, integers only for memory, and the `u32` millicore bound are properties of the grammar, not withheld capability (feature delta § Scope limitations). No GitHub issue was proposed.

## Permanent artifacts

- ADRs 0339 to 0343 and the ADR-0310 amendment in `docs/product/architecture/`.
- Code: `crates/overdrive-core/src/byte_size.rs`, `cpu_quantity.rs`, `volume.rs`, `traits/driver.rs`, `aggregate/`; `api/openapi.yaml`; `crates/overdrive-cli/src/render.rs`.
- [`verification/expectations/O08-workload-resources-human-readable-refusal-and-describe/`](../../verification/expectations/O08-workload-resources-human-readable-refusal-and-describe/README.md) with its evidence, a candidate to stabilize into a test.
- Website pages listed above.

## Migration of lasting artifacts

Phase B is not yet run. Expected result: the feature has no `design/architecture-design.md` and no `distill/walking-skeleton.md`; its ADRs already sit in `docs/product/architecture/`. The design reviews, DISTILL scenarios, RED classification, spike findings, roadmap and execution log stay as delivery history.

## Finalization

- **Issues closed:** #372, closed 2026-11-21 after checking it with `gh issue view 372 --comments`, with a comment linking this record.
- **Metal cleanup:** _to be filled by the orchestrator (remote checkout and build output removed after any needed evidence is pulled; record the run id)_.
- **Website validation and documentation review:** `bun run typecheck` and `bun run build` pass. An independent reviewer ran two rounds over `reference/workload-spec.mdx`, `reference/cli.mdx`, `reference/meta.json`, `how-to/deploy-a-workload.mdx` and `concepts/microvms.mdx`; both rounds' findings are fixed (`f1fd3e907`, `c45016871`). No blog post, by the user's decision.
- **Session files removed:** _to be filled by the orchestrator (`deliver/.develop-progress.json`, after approval)_.
