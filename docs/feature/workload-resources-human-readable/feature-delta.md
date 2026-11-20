# Feature delta: workload-resources-human-readable (GH #372)

Waves: DISCUSS (decisions D-1..D-8, stories, ACs) and DESIGN (approved by the user 2026-11-20). Where DESIGN refines a DISCUSS item, the DISCUSS
text below is written as the current requirement.

## Wave: DISCUSS / [REF] Persona ID

`ana-platform-engineer` (`docs/product/personas/ana-platform-engineer.yaml`): the operator who authors Job, Service and VM workload specs and applies them with `overdrive deploy <SPEC>`. Her lens here is spec authoring and reading back what the platform recorded.

## Wave: DISCUSS / [REF] JTBD one-liner

Traces to `J-OPS-002` (submit a spec and "trust what the CLI tells me": the size she declared must read back in her own words) and secondarily `J-OPS-003` (VM workload sizing: `[resources]` is the single source for guest vCPUs and RAM, `run-a-vm-workload.yaml` step 1). No new job: the existing two cover it. No `jobs.yaml` edit.

When I size a workload in its TOML spec, I want to write and read the size the way I think of it (`128MiB`, `500m`), so a mistyped number is rejected or visibly wrong instead of silently becoming a different size.

## Wave: DISCUSS / [REF] Grounded premise

The state is real on the production path: `overdrive deploy <SPEC>` parses `[resources]` today with two unit-in-the-name integers.

| Surface | Today |
|---|---|
| TOML wire input | `ResourcesInput { cpu_milli: u32, memory_bytes: u64 }`, `deny_unknown_fields` (`crates/overdrive-core/src/aggregate/workload_spec.rs`) |
| Validation | `Job::from_submit` and the service constructor reject `memory_bytes == 0` (`aggregate/mod.rs`); `cpu_milli` has no check, `vcpus_for(cpu_milli)` floors to 1 vCPU (`vm/config.rs`) |
| Intent aggregate | `Resources { cpu_milli, memory_bytes }` (`traits/driver.rs`); a second twin in `aggregate/mod.rs`; node capacity `NodeSpecInput` |
| Persisted form | workload intent rkyv envelope; golden fixtures reference the fields (`crates/overdrive-core/tests/schema_evolution/workload_intent.rs`, `service_spec.rs`) |
| Wire / OpenAPI | `ResourcesBody` (`overdrive-control-plane/src/api.rs`), three schemas in `api/openapi.yaml`; describe wire `core/src/api/describe.rs` |
| Operator rendering | `render::workload_describe` prints `Memory:        134217728` raw bytes (`overdrive-cli/src/render.rs`); CPU is not rendered at all |
| Examples / docs | 33 TOML specs plus `examples/service-kind-vm-workloads-v2/run-example.sh` under `examples/`; `website/content/docs/how-to/deploy-a-workload.mdx`; `website/app/(home)/page.tsx` |

Historical evidence under `verification/expectations/*/evidence/`, `benchmarks/results/` and `docs/evolution/` carries the old names and is immutable record: not edited.

**The issue's reuse premise is only partly true.** #372 says `memory` reuses `VolumeSize`/`ByteCount`. Neither parses `128MiB`: `VolumeSize` is
`<uint>` followed by `GiB` or `TiB` (ADR-0310 as accepted), and `ByteCount` is `<uint>B` only. Both are still the RED scaffold at HEAD
(`volume.rs`); #348 DELIVER has not landed them. R-1 below resolves this.

## Wave: DISCUSS / [REF] Locked decisions

| ID | Decision |
|---|---|
| D-1 | `[resources]` takes `cpu` and `memory`, both TOML strings. `cpu_milli` and `memory_bytes` are removed with no alias and no fallback. Their rejection is the existing `deny_unknown_fields` refusal; the operator sees which key is unknown and which are expected. |
| D-2 | Each value is a validated `overdrive-core` newtype: constructor validates and returns `Result`, `FromStr`, `Display` (canonical form), serde string matching `Display`/`FromStr` (`rust.md` Newtypes). Memory and CPU render through the same `Display` on every operator-visible and wire surface (Cloud Hypervisor and cgroup receive raw bytes). |
| D-3 | One canonical form per value. Whatever an operator writes, `describe`, deploy output, error text and the API wire show the canonical form. `parse(display(x)) == x`. |
| D-4 | A unit-less bare number for `memory` (`memory = 134217728`) is refused with a message naming the expected form. Silently interpreting a bare number is the exact error class this feature removes. |
| D-5 | The type change leaves the persisted workload intent's archived layout unchanged, so no envelope version is added: the existing golden fixtures decode unchanged through the new types, and their hex literals are untouched. |
| D-6 | Scope is the workload `[resources]` table for all three kinds (Job, Service, Schedule share `ResourcesInput`) and node capacity (R-2). Examples and `website/content/docs` change in the same slice as the field they describe. |
| D-7 | Zero memory stays refused (today's behaviour). A workload `memory` that is not a whole number of MiB is refused at `deploy` (DD-8). Zero CPU is accepted (DD-3). |
| D-8 | Greenfield single-cut: no migration of stored intent, no dual-read. |

## Wave: DISCUSS / [REF] User rulings (2026-11-20)

- **R-1 (memory grammar base): one shared byte-size grammar, hand-built in `overdrive-core`, no new crate dependency.** `memory` and the volume size (#348) parse and render through the same grammar; the volume keeps its own refinement on top. ADR-0310's unit decision is corrected directly in present tense (#348 is not yet implemented).
- **R-2 (node capacity): rename in this feature.** `NodeSpecInput { cpu_milli, memory_bytes }` moves to the same CPU / memory newtypes and unit-free names.
- **R-3 (cpu grammar): Kubernetes-style quantity.** `cpu = "500m"` or decimal cores (`"0.5"`, `"2"`); hand-built, no `k8s-openapi` / `kube_quantity` dependency.

## Wave: DISCUSS / [REF] User stories

Two operator-visible outcomes, delivered in one cut (§ Delivery cut).

### US-1: Write and read the workload's memory in human units

`job_id: J-OPS-002` (secondary `J-OPS-003`).

**Problem.** Ana Ortiz sizes a 128 MiB VM service. She writes `memory_bytes = 134217728` by hand; one dropped digit (`13421772`) is a valid but wrong 12.8 MiB guest, and `describe` prints `Memory:        134217728`, so she cannot see the mistake without dividing.

### Elevator Pitch
Before: Ana cannot declare memory without computing bytes by hand, and cannot tell from `describe` whether the recorded size is what she meant.
After: run `overdrive deploy dev-box.toml` with `[resources] memory = "128MiB"`, then `overdrive workload describe dev-box` → sees `Memory:        128MiB`.
Decision enabled: Ana confirms the declared size matches her intent, or corrects the spec, without arithmetic.

**Domain examples.**
1. Happy: `memory = "128MiB"` deploys; `describe` shows `Memory:        128MiB`; the guest gets 134217728 bytes.
2. Canonicalisation: `memory = "1024MiB"` is stored and shown as `1GiB`; `"1024MiB"` and `"1GiB"` produce the identical spec digest.
3. Rejection: `memory = "128 megs"`, `memory = "0MiB"`, `memory = "1000B"`, `memory = 134217728` and the removed `memory_bytes = 134217728` each fail at `deploy`, nothing is stored, and the message names the expected form.

**Acceptance criteria (each testable at the library entry point).**
- AC-1.1: A spec with `[resources] memory = "128MiB"` is admitted; `JobSpecInput`/`ServiceSpecInput` carry it and the intent `Resources` memory equals 134217728 bytes.
- AC-1.2: `memory_bytes` in `[resources]` is refused at parse time, naming `memory_bytes` as unknown; no record is written.
- AC-1.3: Each of `""`, `"0MiB"`, `"128"`, `"128MB"` (decimal unit), `"-1MiB"`, `"1.5MiB"`, a bare TOML integer, a value that overflows `u64` bytes, and a value that is not a whole number of MiB (`"1000B"`) is rejected with an error whose text names the expected grammar or rule.
- AC-1.4: Round trip: for every accepted value `v`, `parse(display(v)) == v` (proptest), and serde JSON round-trips to the same string.
- AC-1.5: `overdrive workload describe` prints `Memory:` with the canonical string, not raw bytes, for Job, Service and a VM-driver workload alike (one renderer, `render::workload_describe`).
- AC-1.6: The existing golden fixtures of the persisted workload intent and parser-side service spec decode unchanged through the new types and equal their canonical projection; their hex literals are untouched.
- AC-1.7: `api/openapi.yaml` regenerates with `memory` and without `memory_bytes` in the three resource schemas; the openapi check passes.
- AC-1.8: Every file under `examples/` and the website how-to use `memory = "..."`; none carries `memory_bytes`.
- AC-1.9: Identical spec content spelled two ways that canonicalise equal (`"1024MiB"`, `"1GiB"`) yields the same spec digest, so an idempotent re-deploy is not a conflict.

### US-2: Write and read the workload's CPU as a quantity

`job_id: J-OPS-002` (secondary `J-OPS-003`).

**Problem.** Ana writes `cpu_milli = 500` and has to know the field counts thousandths of a core; `cpu_milli = 0.5` or `cpu_milli = 2` (meaning two cores) is a type error or a 2-millicore workload depending on the day she misreads it. The name encodes the unit; the value does not carry it.

### Elevator Pitch
Before: Ana expresses CPU in raw milli-units and cannot tell `500` from `0.5` cores at a glance.
After: run `overdrive deploy dev-box.toml` with `[resources] cpu = "500m"` → the spec is admitted; a malformed `cpu = "five hundred"` is refused with the expected form named; `overdrive workload describe dev-box` shows `CPU:           500m`.
Decision enabled: Ana picks a core fraction without unit arithmetic and knows at deploy time whether it was understood.

**Domain examples.**
1. Happy: `cpu = "500m"` admitted; `describe` shows `CPU:           500m`; the intent carries 500 millicores, the value `cpu_milli = 500` carried.
2. Whole cores: `cpu = "2"` admitted as 2000 millicores; `describe` shows `CPU:           2`; `cpu = "0.5"` reads back `500m`.
3. Rejection: `cpu = "500mm"`, `cpu = "1.0005"` (more than three fractional digits), `cpu = ""`, the removed `cpu_milli = 500`, each fail at `deploy`, nothing stored.

**Acceptance criteria.**
- AC-2.1: A spec with `[resources] cpu = "500m"` is admitted and the intent `Resources` CPU equals 500 milli-CPU.
- AC-2.2: `cpu_milli` in `[resources]` is refused at parse time, naming `cpu_milli` as unknown.
- AC-2.3: Malformed values (empty, unknown suffix, negative, more than three fractional digits, above the `u32` milli ceiling, a bare TOML number) are rejected with an error naming the expected grammar.
- AC-2.4: A `cpu` string parses to the millicores the operator meant (`"500m"` and `"0.5"` are 500, `"2"` is 2000), pinned by the CPU grammar tests. `vcpus_for` itself is unchanged and stays covered by the existing `vm_resources_derivation` tests.
- AC-2.5: `parse(display(v)) == v` for every accepted value (proptest); equal quantities spelled differently produce the same spec digest.
- AC-2.6: Fixtures (AC-1.6), openapi regeneration (AC-1.7) and the examples/docs sweep (AC-1.8) hold for the `cpu` field.
- AC-2.7: After this feature no `cpu_milli` or `memory_bytes` identifier remains in the operator-facing spec, the wire types, `api/openapi.yaml` or the grep-gate surfaces (`cpu_milli|memory_bytes` across `crates/`, `tests/`, `examples/`, `verification/**/runner.sh`, `benchmarks/catalogue.md`, `website/`, `api/`), except the `memory_bytes` metric label in `crates/overdrive-bench-families/src/families/mtls_path.rs`.
- AC-2.8: `overdrive workload describe` prints a `CPU:` line with the canonical quantity for every workload kind, under the same presence guard as `Memory:`; zero CPU renders as its canonical zero form `0`.

### Node capacity (R-2)

- AC-1.10: Node capacity (`NodeSpecInput`) carries `cpu` and `memory` as the same typed quantities (`CpuQuantity`, `ByteSize`) with unit-free names; the field types are compiler-enforced, so a raw integer cannot be assigned. Zero memory is refused. The whole-MiB rule applies to workloads only, so a sub-MiB node capacity is admitted.

## Wave: DISCUSS / [REF] Definition of Done

1. Both AC sets green through the production composition (`overdrive deploy` handler and `workload describe` renderer called as library functions, per `overdrive-cli/CLAUDE.md`). Scenario specifications: [`distill/test-scenarios.md`](distill/test-scenarios.md).
2. Old keys refused; no alias path exists in source.
3. Existing golden fixtures decode unchanged through the new types; their hex literals are byte-identical.
4. `api/openapi.yaml` regenerated; check passes.
5. Examples and website docs updated; the grep gate is clean.
6. One renderer (`render::workload_describe`) prints the canonical forms; tests on the live path.
7. Grammar proptests (round trip, rejection) for both newtypes; mutation gate on the parsers.
8. Doc and blog gate per `.claude/rules/finalize.md` at FINALIZE.
9. No deferral language without a user-approved issue number.

## Wave: DISCUSS / [REF] Out-of-scope

- Resource limits vs requests, CPU pinning, burst, hugepages, swap: no such surface exists.
- Renaming or restructuring `[vm.volume]`.
- A migration tool or compatibility read for old specs and stored intent (D-8).

## Wave: DISCUSS / [REF] WS strategy

Brownfield. Walking skeleton: not needed. `overdrive deploy` -> parse -> intent store -> `GET /v1/allocs` -> `workload describe` already exists end to end; the slice is a change along that existing path (strategy: extend in place).

## Wave: DISCUSS / [REF] Pre-requisites

- GH #348 (node-local volume) is not a prerequisite: `ByteSize` is created here (R-1) and `VolumeSize` follows it when #348 resumes.
- Rust: `rust.md` Newtypes; no `.expect()` in library code.

## Wave: DISCUSS / [REF] Outcome KPIs

| KPI | Target | Measured by |
|---|---|---|
| Specs and docs using unit-in-name fields | 0 occurrences of `cpu_milli`/`memory_bytes` on the grep-gate surfaces (`crates/`, `tests/`, `examples/`, `verification/**/runner.sh`, `benchmarks/catalogue.md`, `website/`, `api/`), except the `memory_bytes` metric label in `crates/overdrive-bench-families/src/families/mtls_path.rs` | grep gate at DELIVER and in review |
| Declared size readable in `describe` | 100% of rendered memory values are canonical strings, 0 raw byte counts | live-path render tests |
| Grammar soundness | round trip holds for all accepted inputs; every rejection class in AC-1.3/AC-2.3 has a test | proptest + acceptance tests; mutation kill rate >= 80% on both parsers |
| Silent mis-size on mistyped value | 0 inputs of the rejected classes admitted | acceptance tests |

## Wave: DISCUSS / [REF] Wave decisions

- Feature type cross-cutting; WS = none, brownfield extend-in-place; UX depth lightweight; JTBD traced to `J-OPS-002`/`J-OPS-003`, no new job.
- SSOT: `docs/product/journeys/run-a-vm-workload.yaml` step 1 and the declared-resources shared artifact use the new field names. `jobs.yaml` unchanged.
- DISCOVER/DIVERGE: absent; the source requirement is GH #372 and the operator pain is first-hand. Noted as a risk: the cpu grammar has no operator evidence behind it.

## Wave: DESIGN / [REF] Status and scope

Application scope, PROPOSE mode, 2026-11-20; every decision below is approved by the user. No code, no GitHub issue, no new dependency. No new
container or component: C4 L1 and L2 are unchanged (a value-type and field change inside `overdrive-core`, the control-plane API, the CLI
renderer and the worker's derivations), so no C4 diagram is added. No external integration is added or changed, so no contract-test annotation
applies. Kernel claims: none. One VMM claim is read in `vendors/cloud-hypervisor` (§ Cloud Hypervisor memory constraint).

## Wave: DESIGN / [REF] Decisions

OQ-1..OQ-6 of DISCUSS resolve as: cpu grammar = DD-3; memory grammar base = DD-1/DD-2; internal `Resources` = DD-4; envelope = DD-5; wire shape = DD-6; describe CPU = DD-7.

| ID | Decision | ADR | Approval |
|---|---|---|---|
| DD-1 | `memory` and the volume size share one `ByteSize` grammar | [0339](../../product/architecture/adr-0339-memory-and-volume-sizes-share-one-binary-byte-size-grammar.md) | R-1, 2026-11-20 |
| DD-2 | The volume size is a `ByteSize` refined to whole GiB, at least 1 GiB; ADR-0310 corrected in place | [0310](../../product/architecture/adr-0310-vm-volume-is-declared-by-a-vm-volume-table-with-a-size.md) | R-1, 2026-11-20 |
| DD-3 | `cpu` is a `CpuQuantity` in the Kubernetes-style grammar pinned here | [0340](../../product/architecture/adr-0340-cpu-is-declared-as-a-millicore-or-decimal-core-quantity.md) | R-3, 2026-11-20 |
| DD-4 | The resource types carry the newtypes as `cpu` and `memory`; node capacity is R-2 | [0341](../../product/architecture/adr-0341-workload-resources-carry-the-typed-quantities-from-spec-to-driver.md) | R-2 and decision 1a, 2026-11-20 |
| DD-5 | No envelope version is added: the archived layout is unchanged (spike V-1 held) | 0341 | decision 2, 2026-11-20 |
| DD-6 | The API carries the canonical string | [0342](../../product/architecture/adr-0342-the-api-carries-resource-quantities-as-their-canonical-string.md) | decision 3a, 2026-11-20 |
| DD-7 | `describe` prints `CPU:` beside `Memory:` | none (operator output line) | decision 4a, 2026-11-20 |
| DD-8 | A workload `memory` is a whole number of MiB; `deploy` refuses any other value at the aggregate admission site | [0343](../../product/architecture/adr-0343-a-workload-memory-is-a-whole-number-of-mib.md) | decision 5a, 2026-11-20 |

## Wave: DESIGN / [REF] DDD list

No new aggregate, entity or event. Two new value objects: `ByteSize` (a byte count with one grammar and one canonical spelling; memory is a
`ByteSize`, `VolumeSize` refines it) and `CpuQuantity` (a CPU amount at millicore resolution with one grammar and one canonical spelling).
`Resources` is unchanged as a concept.

## Wave: DESIGN / [REF] Component decomposition

Paths relative to the repository root. Each row includes its consumer, so there is no separate consumer table.

| Path | Change | What |
|---|---|---|
| `crates/overdrive-core/src/byte_size.rs`, `lib.rs` | CREATE NEW | `ByteSize`, `ByteSizeParseError` |
| `crates/overdrive-core/src/cpu_quantity.rs`, `lib.rs` | CREATE NEW | `CpuQuantity`, `CpuQuantityParseError` |
| `crates/overdrive-core/src/volume.rs` | EXTEND | `VolumeSize` parses via `ByteSize` and refines; `Display` delegates; `ByteCount` untouched |
| `crates/overdrive-core/src/traits/driver.rs` | EXTEND | `Resources { cpu, memory }` |
| `crates/overdrive-core/src/aggregate/workload_spec.rs` | EXTEND | parser `ResourcesInput { cpu, memory }` (archived inside `ServiceSpecV1`) |
| `crates/overdrive-core/src/aggregate/mod.rs` | EXTEND | `from_submit` and the service constructor admit `memory`; wire `ResourcesInput`; the two reverse projections; `Node::new`; `NodeSpecInput` |
| `crates/overdrive-core/src/scheduler.rs` | EXTEND | `free_capacity`, `covers`, the `max_free` fold read through `.millis()` / `.bytes()` with saturating arithmetic; results rebuilt with `CpuQuantity::from_millis`, `ByteSize::new`; comparisons use the derived `Ord` |
| `crates/overdrive-core/src/vm/config.rs` | EXTEND | `vcpus_for(cpu: CpuQuantity)`; `MemoryPlan` and `reserve_bytes` KEEP (byte arithmetic whose outputs are foreign-protocol numbers) |
| `crates/overdrive-core/src/transition_reason.rs` | EXTEND | `ResourceEnvelope { cpu, memory }`; `NoCapacity` prose via `Display` |
| `crates/overdrive-core/src/api/describe.rs`, `api/submit.rs` | EXTEND | follow the `ResourcesInput` change (including the proptest strategy) |
| `crates/overdrive-control-plane/src/api.rs`, `handlers.rs` | EXTEND | `ResourcesBody { cpu, memory }`; projections |
| `crates/overdrive-cli/src/commands/deploy.rs` | EXTEND | projections |
| `crates/overdrive-cli/src/render.rs` | EXTEND | `Memory:` canonical, `CPU:` added (DD-7), both under the existing presence guard |
| `crates/overdrive-worker/src/cgroup_manager.rs` | EXTEND | `cpu_weight_for(cpu: CpuQuantity)`; the `memory.max` write reads `.bytes()` (a decimal byte count) |
| `crates/overdrive-worker/src/vm_driver.rs` | EXTEND | `MemoryPlan::derive(spec.resources.memory.bytes())`; `Resources { cpu, memory: ByteSize::new(plan.cgroup_max_bytes()) }`; `vcpus_for(spec.resources.cpu)`. The Cloud Hypervisor `--memory size=` argument (`overdrive-host/src/vmm.rs`) stays a raw byte count from `MemoryPlan::guest_bytes()`: a `ByteSize` `Display` is never passed to a foreign protocol (`128MiB` is not a Cloud Hypervisor spelling) |
| `crates/overdrive-reconcilers/src/workload_lifecycle.rs` | EXTEND | the baseline local node: `4000m`, `8GiB` |
| `api/openapi.yaml` | EXTEND (regenerated) | three schemas: `cpu`, `memory` as `type: string` |
| Rust sources under `crates/` (fixtures, projections, tests; about 242 files) | EXTEND | mechanical rename and literal rewrite, except the sub-MiB sim fixtures below |
| Non-`.rs` files under `crates/` | EXTEND | the hydration golden and trajectory text fixtures (`overdrive-control-plane/tests/acceptance/fixtures/hydration_golden/`, `overdrive-sim/tests/acceptance/fixtures/hydration_trajectory/`) and the `.proptest-regressions` seed files that name the old fields |
| Raw TOML/JSON string literals that carry the old keys (a compile does not find them; under `deny_unknown_fields` they fail at run time) | EXTEND | `crates/overdrive-control-plane/src/node_volume_boundaries.rs`, `src/quint_node_volume.rs`, `tests/conformance/src/gateway.rs`, `tests/conformance/tests/shared_guest_network_fail_stop_recovery.rs` |
| `crates/overdrive-bench-families/src/families/vm_lifecycle.rs`, `benchmarks/catalogue.md` | EXTEND | the generated spec TOML and its note; the catalogue line naming the field. The metric label `memory_bytes` in `mtls_path.rs` is a measurement name, not the resource field, and is not touched |
| `verification/expectations/E06-vm-job-deploy-reaches-running/runner.sh` | EXTEND | the live runner script's spec heredoc (the `evidence/` directories are immutable record) |
| 33 TOML specs under `examples/`, `website/content/docs/how-to/deploy-a-workload.mdx`, `website/app/(home)/page.tsx` | EXTEND | spec spelling |
| `examples/service-kind-vm-workloads-v2/run-example.sh` | EXTEND | the capacity log label `memory_bytes=$HOST_MEM_BYTES` (host capacity, not a spec field) is renamed to `memory=`; no parser reads it (the retained E09 evidence logs that contain the old label are outside the gate) |
| `docs/product/architecture/brief.md` workload spec example | EXTEND at DELIVER | same spelling; older ADR-derived prose naming `memory_bytes` is not rewritten |

**Sub-MiB fixtures are not mechanical.** Four sim tests submit 256 KiB memory through `ResourcesInput`, which DD-8 refuses:
`netns_density_activation_order.rs`, `netns_density_retiring_cleanup.rs`, `netns_density_reclaim.rs` (under `overdrive-sim/tests/`) and the
`DEMAND_MEMORY_BYTES` constant in `netns_density_node_admission.rs`. Raise each to at least 1 MiB, and keep the paired `Resources` literals
in the same files consistent with the submitted value. Any other `ResourcesInput` literal that is not a whole number of MiB is handled the same way,
including the proptest strategies that draw arbitrary `u64` memory (`describe_round_trip.proptest-regressions` records such seeds); they generate whole-MiB memory.

## Wave: DESIGN / [REF] Driving ports

Unchanged: `overdrive deploy <SPEC>` (`commands/deploy.rs`), the submit handler (`handlers.rs`), `overdrive workload describe <id>`
(`render::workload_describe`), and `api/openapi.yaml`. The deploy TOML and the submit JSON take `cpu` and `memory` as strings. No driving port
gains a write method; the describe path stays read-only.

## Wave: DESIGN / [REF] Driven ports

None added or changed. The parsers are pure functions (contract shape: pure-function, return-only); there is no adapter, so no probe applies.

## Wave: DESIGN / [REF] Technology choices

Hand-built grammars in `overdrive-core` (user rulings R-1, R-3); existing `thiserror`, `serde`, `rkyv`, `utoipa`. No new dependency, no licence
to record. Rejected: `bytesize`, `byte-unit`, `k8s-openapi`, `kube_quantity` (user rulings).

Architecture enforcement: the existing `dst-lint` scan keeps both parsers free of clock, entropy and network (both are `core`-class pure code);
the 80% mutation gate covers both parsers. The KPI grep gate is `cpu_milli|memory_bytes` over `crates/`, `tests/`, `examples/`,
`website/content/docs`, `website/app/(home)/page.tsx`, `api/openapi.yaml`, `benchmarks/catalogue.md` and `verification/expectations/`
(excluding every `evidence/` directory). It expects zero matches except the `memory_bytes` metric label in `mtls_path.rs`. Retained results under
`benchmarks/results/` and `docs/evolution/` are historical record and outside the gate. No new tool.

## Wave: DESIGN / [REF] Delivery cut

**One slice** (`slices/slice-01-workload-resources-human-readable.md`). `Resources`, both `ResourcesInput` twins and `ResourcesBody` each carry
`cpu` and `memory` in one struct, and DD-4 renames both fields together. Splitting by field would leave an intermediate spec form that mixes
`cpu_milli` with `memory`, rewrite the same ~242 fixture literals, the three OpenAPI schemas and the 33 example specs twice, and ship the
unit-in-the-name key this feature removes. US-1 and US-2 stay two stories with their own ACs; they are one DELIVER cut.

`Schedule::from_submit` is a `todo!()` scaffold today. When it lands it routes through the same admission as `Job::from_submit`, so the
memory refusal of DD-8 applies to a Schedule too.

## Wave: DESIGN / [REF] Reuse Analysis

| Existing | Verdict | Reason and contract shape |
|---|---|---|
| `ByteCount` (`volume.rs`, RED scaffold) | KEEP unchanged | free space, `<n>B`, any `u64`; a strict subset of the grammar. Unifying it into `ByteSize` rewrites GH #348's pinned scaffold and its S-VOL-26a scenario, which no requirement here needs |
| `MemoryPlan`, `reserve_bytes` (`vm/config.rs`) | KEEP unchanged | byte arithmetic whose outputs are foreign-protocol numbers (`memory.max`, the VMM `size=`); the caller unwraps with `.bytes()` |
| `ByteSize` | CREATE NEW | no existing type parses `128MiB`; `VolumeSize` is GiB-only and `ByteCount` is `B`-only. pure-function |
| `CpuQuantity` | CREATE NEW | no CPU value type exists; `u32` primitives today. pure-function |

Every other overlapping component is EXTEND in § Component decomposition: field types and names only, bounded-change (universe: the
resource structs listed there), with the existing proptests of `vcpus_for`, `cpu_weight_for` and the scheduler as the assertion mechanism.

## Wave: DESIGN / [REF] Interface contracts

Exact public contracts. Internal structure (the scanner, helper functions, module-private types) is the crafter's. The table rows below pin
decisions; DISTILL owns the full scenario matrix.

### `ByteSize` — `overdrive-core/src/byte_size.rs`

```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord,
         rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct ByteSize(u64);

impl ByteSize {
    pub const fn new(bytes: u64) -> Self;        // total: every u64 is a valid size, including 0
    pub const fn bytes(self) -> u64;
}
impl FromStr for ByteSize { type Err = ByteSizeParseError; }
impl Display for ByteSize;                       // the canonical form
impl Serialize for ByteSize;                     // the Display string
impl<'de> Deserialize<'de> for ByteSize;         // a string through FromStr; a non-string reports the expected grammar
```

`ByteSizeParseError` (`Debug Clone PartialEq Eq`, `thiserror::Error`), one variant per distinct operator remedy, each with its own `Display`:
`Malformed { input }` (names the grammar and the five units), `MissingUnit { input }` (names the expected form), `DecimalUnit { input }`
(names the binary units `KiB`, `MiB`, `GiB`, `TiB`), `Fractional { input }` (says to write the smaller unit, e.g. `1536MiB`),
`Overflow { input }`.

Grammar (ASCII; the unit is case-insensitive):

```
byte_size = digits , unit ;
digits    = digit , { digit } ;          (* leading zeros allowed; value = digits x unit *)
unit      = "B" | "KiB" | "MiB" | "GiB" | "TiB" ;   (* 1, 2^10, 2^20, 2^30, 2^40 bytes *)
```

Classification, in order, over the input: no leading digit (empty, sign, space, `.5`, `MiB`) is `Malformed`; digits to the end is
`MissingUnit`; digits followed by `.` is `Fractional`; a tail of `KB`, `MB`, `GB` or `TB` (any case) is `DecimalUnit`; a tail that is not one
of the five units is `Malformed` (`128Mi`, `128M`, `1PiB`, `1e3MiB`, `128 MiB`, `1_000MiB`); a valid input whose value exceeds `u64::MAX`
bytes is `Overflow`.

Canonical `Display`: the integer count in the largest of `TiB`, `GiB`, `MiB`, `KiB`, `B` that divides the value exactly; zero is `0B`.

| Input | Result |
|---|---|
| `128MiB`, `128mib` | 134217728, `128MiB` |
| `1024MiB`, `1GiB`, `1073741824B` | 1073741824, `1GiB` |
| `1536MiB` | 1610612736, `1536MiB` |
| `0B`, `0MiB` | 0, `0B` (the grammar admits zero; workload memory admission refuses it) |
| `16777215TiB` / `16777216TiB`, `18446744073709551616B` | accepted (largest whole-TiB value) / `Overflow` |
| `128` | `MissingUnit` |
| `128MB`, `1kb` | `DecimalUnit` |
| `1.5MiB`, `1.MiB` | `Fractional` |
| ` 128MiB`, `-1MiB`, `128Mi` | `Malformed` |

### `CpuQuantity` — `overdrive-core/src/cpu_quantity.rs`

```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord,
         rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct CpuQuantity(u32);                     // millicores

impl CpuQuantity {
    pub const fn from_millis(millis: u32) -> Self;   // total, including 0
    pub const fn millis(self) -> u32;
}
impl FromStr for CpuQuantity { type Err = CpuQuantityParseError; }
impl Display for CpuQuantity;  impl Serialize;  impl<'de> Deserialize<'de>;   // as ByteSize
```

`CpuQuantityParseError`: `Malformed { input }`, `UnknownSuffix { input, suffix }` (the `Display` for an uppercase `M` says the milli suffix is
the lowercase `m`), `TooManyDecimals { input }` (more than three fractional digits), `Overflow { input }`.

```
cpu        = milli_form | core_form ;
milli_form = digits , "m" ;                  (* "m" is lowercase only; integers only *)
core_form  = digits , [ "." , frac ] ;       (* a bare number is whole cores *)
frac       = digit , [ digit ] , [ digit ] ; (* at most three digits: millicore resolution *)
```

Classification, in order: no leading digit is `Malformed`; digits to the end is whole cores; digits then a purely alphabetic tail is `m`
(milli) or else `UnknownSuffix` (`500M`, `500mm`, `2c`); digits then `.` needs at least one further digit (else `Malformed`), anything after
the fractional digits is `Malformed` (`500.5m`), and more than three fractional digits is `TooManyDecimals` even when the extra digits are
zeros (`0.5000`); any other tail is `Malformed` (`5 cores`, `1e3`, ` 1`, `1 `); a value above `u32::MAX` millicores is `Overflow`.

Value in millicores: `milli_form` is `digits`; `core_form` is `whole x 1000 + frac` with `frac` right-padded to three digits.
Canonical `Display`: if the value is an exact multiple of 1000, the whole-core integer (`2`, `0`); otherwise `<n>m`. Decimal cores are an input
spelling only.

| Input | Result |
|---|---|
| `500m`, `0.5`, `0.500` | 500, `500m` |
| `2`, `2000m`, `2.0` | 2000, `2` |
| `1.5` | 1500, `1500m` |
| `0`, `0m` | 0, `0` (accepted; see ADR-0340) |
| `4294967.295` / `4294967296m` | 4294967295, `4294967295m` / `Overflow` |
| `500M`, `500mm` | `UnknownSuffix` |
| `1.0005`, `0.5000` | `TooManyDecimals` |
| `five`, `.5`, `500.5m`, `5 cores` | `Malformed` |

### Volume refinement — `overdrive-core/src/volume.rs`

`VolumeSize::from_str` parses a `ByteSize`, then refuses a value that is zero or not a multiple of 1 GiB. A refusal at either step is a
`VolumeSizeParseError` (the existing scaffold struct, unchanged in shape: `Display` text names the cause). `VolumeSize::bytes(self) -> u64` and
the derives are as pinned in GH #348. `Display` is the `ByteSize` canonical form of the same value, which for a multiple of GiB is `<n>GiB`, or
`<n>TiB` for an exact TiB multiple: the rule ADR-0310 already stated.

| Input | Volume result |
|---|---|
| `40GiB`, `2TiB`, `1024MiB` | accepted (`40GiB`, `2TiB`, `1GiB`) |
| `512MiB`, `0GiB`, `1.5GiB`, `1GB` | refused |

### Workload memory refinement — whole MiB (DD-8, ADR-0343)

`ByteSize` is unchanged; the rule is a check at the aggregate admission site that already refuses zero memory, in `Job::from_submit` and the
service constructor. It is a check, not a type: the rule has one caller pair and no value is carried further that needs the guarantee in its
type. `Node::new` keeps only the non-zero check (node capacity is not guest RAM). Whole MiB covers 4 KiB, 16 KiB and 64 KiB host pages.

Pinned contract: a workload `memory` is admitted only when `memory.bytes()` is a non-zero multiple of 1 MiB (1048576). Otherwise admission
returns the existing `AggregateError::Validation { field: "memory", message }` (no new variant). Messages (the crafter owns exact punctuation):
`memory must be non-zero`; `memory must be a whole number of MiB; got {canonical}`.

| `memory` input | Workload result |
|---|---|
| `128MiB`, `1MiB` | accepted |
| `1024KiB` | accepted; canonical `1MiB` |
| `0MiB` | refused: `memory must be non-zero` |
| `1000B`, `1536KiB` | refused: whole MiB, naming the received canonical value |

The guest RAM Cloud Hypervisor receives is unchanged: `MemoryPlan::derive` takes `spec.resources.memory.bytes()` and `--memory size=` stays a
raw byte count. The volume size is unaffected (its whole-GiB rule already implies whole MiB).

### Resources and capacity — field types

```
// overdrive-core/src/traits/driver.rs
pub struct Resources { pub cpu: CpuQuantity, pub memory: ByteSize }   // derives unchanged

// overdrive-core/src/aggregate/workload_spec.rs  (parser twin; archived inside ServiceSpecV1)
#[serde(deny_unknown_fields)]
pub struct ResourcesInput {
    #[schema(value_type = String, example = "500m")]    pub cpu: CpuQuantity,
    #[schema(value_type = String, example = "128MiB")]  pub memory: ByteSize,
}
// overdrive-core/src/aggregate/mod.rs  (wire twin): the same two fields, same attributes, no rkyv derive (as today)

// overdrive-core/src/aggregate/mod.rs
pub struct NodeSpecInput { pub id: String, pub region: String, pub cpu: CpuQuantity, pub memory: ByteSize }

// overdrive-core/src/transition_reason.rs
pub struct ResourceEnvelope { pub cpu: CpuQuantity, pub memory: ByteSize }      // derives unchanged; ToSchema value_type = String per field

// overdrive-control-plane/src/api.rs
pub struct ResourcesBody { pub cpu: CpuQuantity, pub memory: ByteSize }         // Serialize, Deserialize, ToSchema value_type = String
impl From<&Resources> for ResourcesBody;                                         // unchanged
```

Functions: `vcpus_for(cpu: CpuQuantity) -> NonZeroU8` (rule unchanged: `max(1, ceil(millis / 1000))`, saturating at `u8::MAX`);
`cpu_weight_for(cpu: CpuQuantity) -> u32` (`clamp(millis / 10, 1, 10000)`, ADR-0026 D9, unchanged). `MemoryPlan::derive(declared: u64)` and
`reserve_bytes` are unchanged. `schedule` and `free_capacity` keep their signatures.

Admission: `Job::from_submit` and the service constructor apply the whole-MiB refinement; `Node::new` refuses only `memory.bytes() == 0`. All use
`AggregateError::Validation` whose `field` is `"memory"` (was `"memory_bytes"`); CPU has no admission check.

### Wire and spec shapes

| Surface | Shape |
|---|---|
| TOML | `[resources]` with `cpu = "500m"` and `memory = "128MiB"`, both required, both strings; `deny_unknown_fields` refuses `cpu_milli` and `memory_bytes` naming the unknown key and the expected `cpu`, `memory` |
| Submit and describe JSON, `GET /v1/allocs` row `resources` | `{"cpu":"500m","memory":"128MiB"}`; the canonical form is emitted, so `1024MiB` is read back as `1GiB` |
| OpenAPI | `cpu` and `memory` are `type: string` with the example above, in `ResourcesInput`, `ResourcesBody`, `ResourceEnvelope` |
| `NoCapacity` prose | `no capacity (requested cpu=500m mem=128MiB / free cpu=0 mem=0B)` |
| `describe` | `Memory:        128MiB` and `CPU:           500m` (value column at offset 15, as today); both under the existing presence guard |

A bare TOML or JSON number for `cpu` or `memory` is a deserialization error whose text names the expected string form.

## Wave: DESIGN / [REF] Envelope plan

**No envelope version is added (DD-5).** Persisted rkyv types that carry the fields, and the effect:

| Persisted type | Carries | Effect |
|---|---|---|
| `WorkloadIntentEnvelope` V1 (`JobV1`, `ServiceV1`, `ScheduleV1` through `Resources`) | `Resources` | layout unchanged |
| `ServiceSpecEnvelope` V1 (`ServiceSpecV1.resources`) | parser `ResourcesInput` | layout unchanged |
| `AllocStatusRowEnvelope` V1 (`TransitionReason::NoCapacity` through `ResourceEnvelope`) | `ResourceEnvelope` | layout unchanged; no production emit site exists today (GH #261) |
| `Node` (derives rkyv) | `Resources` | layout unchanged; no `Node` envelope exists in tree |

Reason: `#[derive(rkyv::Archive)]` on `struct CpuQuantity(u32)` and `struct ByteSize(u64)` archives each as a one-field struct of the same size
and alignment as the primitive it replaces, so every enclosing struct keeps its field offsets (field names do not enter the bytes). Spike V-1
confirms it on this workspace's rkyv. `development.md` § "rkyv schema evolution" requires a bump when the layout changes; it does not change
here. The `spec_digest` of a stored spec is a hash of those bytes and is unchanged; two spellings of one size produce identical bytes because
the spelling is not stored.

DELIVER obligations: update the struct-literal field names and types in `crates/overdrive-core/tests/schema_evolution/workload_intent.rs`,
`service_spec.rs` and the `AllocStatusRow` fixtures; the hex literals `FIXTURE_V1` stay byte-identical and must still decode and equal the canonical
projection (the default-lane regression guard for V-1). If a fixture fails to decode, the layout claim is false for that envelope: stop and
surface it to the user. If GH #348 lands first, its V2 payloads embed the same `Resources`; the two changes do not conflict in either order.

## Wave: DESIGN / [REF] Lifecycle Gate Ownership

Not applicable: no gate is added, removed or moved. The memory admission refusal stays at the same constructors; its `field` label changes and
the whole-MiB check joins the zero check at the same site (DD-8).

## Wave: DESIGN / [REF] Formal protocol model

Not applicable: the change is value types, parsing and field renames; it adds no concurrent, ordered or crash-sensitive protocol.

## Wave: DESIGN / [REF] Validation items

| ID | Claim | Falsification changes | Class | Status |
|---|---|---|---|---|
| V-1 | A struct holding `CpuQuantity(u32)` and `ByteSize(u64)` archives to the same bytes as one holding `u32` and `u64` (rkyv 0.8, this workspace's features) | the persisted-format contract (DD-5): a bump on three envelopes | contract-shaping | **HELD.** Single-field newtypes archive byte-identically to the primitives and old bytes cross-decode (`spike/findings-v1-newtype-layout.md`, Metal run `2290cbebcd6604db77be3067`, kernel 7.0.0-38) |

No contract-shaping item is Open. The Cloud Hypervisor claim behind DD-8 is source reading, below.

## Wave: DESIGN / [REF] Corrections to other artifacts (listed for the user)

Per R-1, in present tense, no amendment narration:

1. `docs/product/architecture/adr-0310-...md`: Decision says the size is a `ByteSize` refined to whole GiB, at least 1 GiB (`1024MiB`
   accepted, `512MiB` refused); the volume-grammar alternative is reworded as "a volume-only `GiB`/`TiB` grammar".
2. `docs/feature/node-local-persistent-volume/feature-delta.md` § Spec surface: the `VolumeSize` grammar sentence points at ADR-0339 instead of
   restating `GiB|TiB`. `design/wave-decisions.md` VD-1 row follows.
3. Not edited, for the GH #348 DISTILL owner to reconcile when DELIVER resumes: `distill/test-scenarios.md` grammar scenarios for `VolumeSize`
   (a unit-restriction assertion, if any, becomes a refinement assertion) and the scaffold in `crates/overdrive-core/src/volume.rs`.

## Wave: DESIGN / [REF] Scope limitations

None introduced. The grammar choices (binary units only, integers only, `u32` millicore bound) are properties of the mechanism, not withheld
capability. No GitHub issue is proposed.

## Wave: DESIGN / [REF] Open questions for DISTILL and DELIVER

- DISTILL: the exact `Display` text of each parse-error variant is the acceptance designer's to assert at the level of "names the grammar and
  the remedy"; the variant set above is the contract.
- DISTILL: GH #348's grammar scenarios need reconciling with the shared grammar (§ Corrections, item 3).
- No open question blocks DISTILL. DISTILL covers the whole-MiB rows at the `from_submit` and service entry points.

## Wave: DESIGN / [REF] Cloud Hypervisor memory constraint (evidence for DD-8)

Read in `vendors/cloud-hypervisor` @ `f252bb20b` (Overdrive's fork, branch `overdrive/vhost-kernel-vsock`):

- Overdrive launches CH with `--memory size=<guest_bytes>` as a raw byte count (`crates/overdrive-host/src/vmm.rs:283-284`); `guest_bytes` is the operator's declared memory unchanged (`MemoryPlan::derive`, `crates/overdrive-core/src/vm/config.rs:275`).
- With no `zones`, CH builds one default zone from `config.size`, `file: None`, `hugepages: false` (`vmm/src/memory_manager.rs:1499-1513`). Its alignment is the host page size, `sysconf(_SC_PAGESIZE)` (`memory_zone_get_align_size`, `memory_manager.rs:474-481`), and a size that is not a multiple of it is refused with `Error::MisalignedMemorySize` (`memory_manager.rs:635-637`).
- So guest RAM must be a multiple of the host page size (4 KiB on x86_64). The only workload driver is the VM (`WorkloadDriverV1::Vm`, `aggregate/mod.rs:169-172`), so every workload's `memory` is guest RAM.
- **Production-reachable today:** `memory_bytes = 1000` passes `deploy` (only zero is refused) and fails at VM launch. The whole-MiB rule refuses it at `deploy`.
- vCPUs: CH takes `--cpus boot=<vcpus_for(cpu)>` (`vmm.rs:281-282`); `vcpus_for` rounds up and floors at 1. No CPU grammar constraint follows from CH.

## Review dispositions

Adversarial review (A), subtractive review (S) and verification review (R2), persisted under `design/`.

| Id | Disposition |
|---|---|
| A1 | Applied: V-1 row is HELD with the findings link and run id |
| A2 | Applied: D-5, AC-1.6, AC-2.6, DoD 3 and both slice texts now require existing golden fixtures to decode unchanged through the new types |
| A3 | Applied: DD-8 moved to ADR-0343; ADR-0339 links to it and carries no examples |
| A4 | Applied: decomposition lists the raw-literal files, the `verification` runner, `benchmarks/catalogue.md` and the non-`.rs` fixtures; grep gate extended, `mtls_path.rs` metric label excluded |
| A5 | Applied: four sub-MiB sim fixtures listed as non-mechanical (the three named plus `DEMAND_MEMORY_BYTES` in `netns_density_node_admission.rs`, which the review missed) |
| A6 | Applied: rule is "at most three fractional digits"; variant renamed `TooManyDecimals`; `0.5000` is refused under it |
| A7 | Applied: `ByteSize` classification and `CpuQuantity` classification are pinned in order; `1e3MiB`, `5 cores` are `Malformed`; rows fixed |
| A8 | Applied: ADR-0310 status carries no narration; brief.md heading and body are ACCEPTED |
| A9 | Applied: ADR-0341 Decision cut to the decision; layout fact moved to Consequences; names moved to the decomposition |
| A10 | Applied: § Delivery cut states one slice and why; `Schedule::from_submit` sentence added |
| S1 | Applied: stale DISCUSS text rewritten in place (D-5, AC-1.6, AC-2.6, DoD 3, OQ prose, Out-of-scope bullet) |
| S2 | Applied: same as A1 |
| S3 | Applied: V-3 dropped; the openapi check at DELIVER remains the gate |
| S4 | Applied: one statement of the Cloud Hypervisor evidence (the last section); V-2 row removed; ADR-0339 and the refinement point at ADR-0343 |
| S5 | Applied: one ledger (Decisions table with approval and date); the four restatements and the grammar-pin re-listing deleted |
| S6 | Applied: ADR-0339/0340/0341 trimmed to decision, alternatives and consequences; grammar rules live only in § Interface contracts |
| S7 | Applied: ADR-0310 status narration removed |
| S8 | Applied: DISCUSS driving ports section deleted; DESIGN section kept |
| S9 | Applied in part: `UnknownUnit` merged into `Malformed` (same remedy: the grammar); `Fractional`, `DecimalUnit`, `MissingUnit` kept because each `Display` gives a different fix (rust.md distinct failure modes); `CpuQuantityParseError` keeps four variants (`UnknownSuffix` carries the uppercase `M` remedy) |
| S10 | Applied: the `suggestion` field and the per-spelling mapping are dropped; `DecimalUnit` names the four binary units |
| S11 | Applied: the whole-MiB message is the rule plus the received canonical value |
| S12 | Applied: `Hash` derive dropped from both newtypes (no consumer; `Ord` kept) |
| S13 | Applied: consumer trace merged into the decomposition; Reuse Analysis cut to two CREATE NEW and two KEEP rows; rotting line anchors dropped |
| S14 | Applied: DDD list is one paragraph; brief.md and `vm_lifecycle.rs` rows folded into the sweep rows |
| S15 | Applied: grammar tables trimmed to rows that pin a decision; DISTILL owns the rest |
| S16 | Applied: the byte-granule alternative dropped from ADR-0310 |
| S17 | Applied: the crate-alternative wording in ADR-0339 reduced to the user's ruling |
| R2-1 | Applied: the `run-example.sh` capacity log label is renamed to `memory=` in scope (no parser reads it); new decomposition row |
| R2-2 | Applied: the examples count is 33 TOML specs; the script has its own row |
| R2-3 | Applied: the byte-granule alternative is dropped from `node-local-persistent-volume/design/wave-decisions.md` VD-1 |
| R2-4 | Applied: AC-2.7 and the KPI name the `mtls_path.rs` metric label exception the grep gate carries |
| R2-5 | Applied: the sub-MiB paragraph covers proptest strategies drawing arbitrary `u64` memory |

## Wave: DISTILL / [REF] Status and scope

DISTILL, 2026-11-20, revised after review round 1. No `.feature` files (`.claude/rules/testing.md`): every scenario is a Rust test with a complete executable body. A pending test is marked `#[should_panic(expected = "<semantic RED>")]` and runs on every run; DELIVER removes the marker and the test must then pass unchanged (`.claude/rules/testing.md` § "RED acceptance scaffolds and activation"). No DEVOPS wave exists (warning logged; the Metal substrate and the default lanes are the environment). Reconciliation passed: 0 contradictions between DISCUSS and DESIGN. No production field is renamed; the tests are written against surface that exists today (TOML text, JSON text, typed errors, rendered text), so they compile before the rename and fail on their own assertions. 54 tests in 9 files, all in the default lane (host-pure); 33 are error or edge scenarios (61%); no `.feature` file; no new dependency. Per-test RED classification: [`distill/red-classification.md`](distill/red-classification.md).

## Wave: DISTILL / [REF] Scenario list

Lane: **L1** = default lane unit/property, **L2** = default lane in-memory acceptance; every test is host-pure. Error = sad path or boundary. Every test carries `CONTRACT_SHAPE: pure-function`, except the submit-handler test (`unbounded-preservation`). Files are under `crates/`; the RED reason of each test is in `red-classification.md`.

| File | Tests | AC / DD | Lane | Error |
|---|---|---|---|---|
| `overdrive-core/tests/acceptance/byte_size_grammar.rs` | `the_pinned_accept_table_parses_to_the_pinned_size_and_canonical_form`, `display_is_canonical_and_round_trips`, `any_count_in_any_unit_and_case_parses_to_count_times_unit`, `serde_json_is_the_quoted_canonical_text_and_round_trips` | AC-1.1, AC-1.4, AC-1.9, DD-1 | L1 (256 cases for the three proptests) | no |
| same | `sad_decimal_units_are_decimal_unit`, `sad_fractional_counts_are_fractional`, `sad_text_outside_the_grammar_is_malformed`, `sad_a_value_above_u64_bytes_is_overflow`, `sad_structure_outranks_overflow`, `sad_every_rejection_names_its_remedy_and_the_five_messages_differ`, `sad_serde_refuses_a_bare_number_and_names_the_expected_form`, `a_digit_run_without_a_unit_is_always_missing_unit`, `text_without_a_leading_digit_is_always_malformed`, `the_u64_boundary_is_exact_in_every_unit` | AC-1.3, D-4, DD-1 classification order | L1 | yes |
| same | `the_volume_size_is_the_shared_grammar_refined_to_whole_gib` | DD-2 | L1 | yes |
| `overdrive-core/tests/acceptance/cpu_quantity_grammar.rs` | `the_pinned_accept_table_parses_to_the_pinned_quantity_and_canonical_form`, `display_is_canonical_and_round_trips`, `every_spelling_of_one_quantity_is_the_same_value`, `serde_json_is_the_quoted_canonical_text_and_round_trips` | AC-2.1, AC-2.4, AC-2.5, DD-3 | L1 | no |
| same | `sad_text_outside_the_grammar_is_malformed`, `sad_an_unknown_suffix_is_named`, `sad_a_value_above_u32_millicores_is_overflow`, `sad_every_rejection_names_its_remedy_and_the_messages_differ`, `sad_serde_refuses_a_bare_number`, `four_or_more_fractional_digits_are_always_too_many_decimals`, `text_without_a_leading_digit_is_always_malformed`, `the_u32_boundary_is_exact` | AC-2.3, DD-3 | L1 | yes |
| `overdrive-core/tests/acceptance/workload_resources_spec.rs` | `a_non_canonical_spelling_is_read_back_canonically` (every kind), `an_admitted_workload_carries_the_declared_size_in_its_intent_resources`, `equal_sizes_in_different_spellings_have_the_same_spec_digest`, `describe_reads_back_the_canonical_form` | AC-1.1, AC-1.5, AC-1.9, AC-2.1, AC-2.5, DD-6 | L1/L2 | no |
| same | `sad_the_removed_keys_are_refused_naming_the_unknown_key_and_the_expected_ones`, `sad_a_missing_cpu_or_memory_is_refused_naming_it`, `sad_each_malformed_memory_is_refused_at_parse_time_naming_the_grammar`, `sad_each_malformed_cpu_is_refused_at_parse_time_naming_the_grammar`, `sad_admission_follows_the_non_zero_and_whole_mib_rules_at_both_sites` (zero CPU admitted, `1048575B` / `1048577B`), `sad_the_wire_refuses_the_removed_keys` | AC-1.2, AC-1.3, AC-2.2, AC-2.3, D-1, D-4, D-7, DD-8 | L1/L2 | yes |
| `overdrive-core/tests/acceptance/workload_resources_projections.rs` | `no_capacity_prose_renders_the_canonical_quantities` (the persisted `NoCapacity` cause text pinned by DESIGN; no production emit site today, GH #261) | DD-4 | L1 | no |
| `overdrive-core/tests/acceptance/workload_resources_examples.rs` | `every_example_spec_parses_with_the_human_readable_resources` | AC-1.8, AC-2.6, D-6 | L1, reads checked-in files | no |
| `overdrive-core/tests/schema_evolution/workload_resources_fixtures.rs` | `the_workload_intent_fixture_decodes_unchanged_and_reads_back_canonically`, `the_service_spec_fixture_decodes_unchanged_and_reads_back_canonically` | AC-1.6, AC-2.6, D-5, DD-5 | L1 | no |
| `overdrive-cli/tests/acceptance/deploy_workload_resources.rs` | `a_spec_in_the_new_shape_passes_local_validation` | AC-1.1, AC-2.1 | L2 | no |
| same | `sad_deploy_refuses_the_removed_keys_naming_the_unknown_key`, `sad_deploy_refuses_malformed_memory_naming_the_expected_form`, `sad_deploy_refuses_malformed_cpu_naming_the_expected_grammar`, `sad_deploy_refuses_zero_and_sub_mib_aligned_memory_naming_the_memory_field` | AC-1.2, AC-1.3, AC-2.2, AC-2.3, D-1, D-4, DD-8 | L2 (`[job]`, `[service]`, flat `id =` shapes; flat is live, `examples/coinflip.toml`) | yes |
| `overdrive-cli/tests/acceptance/render_workload_resources.rs` | `describe_prints_the_canonical_memory_and_cpu_lines_for_every_kind` (Job, Service, Schedule; no raw byte count), `describe_prints_the_size_once_for_a_multi_replica_workload` | AC-1.5, AC-2.8, DD-7, D-2 | L2 | no |
| same | `describe_prints_a_zero_cpu_as_the_canonical_zero` (`CPU:           0`), `describe_omits_both_size_lines_without_a_resolved_size` | AC-2.8, DD-7 presence guard | L2 | yes |
| `overdrive-control-plane/tests/acceptance/workload_resources_wire.rs` | `the_resources_block_is_emitted_as_canonical_strings`, `openapi_describes_cpu_and_memory_as_strings_in_the_three_schemas` | AC-1.7, DD-6 | L1 | no |
| same | `sad_the_resources_block_refuses_the_removed_keys`, `sad_memory_that_is_not_a_whole_number_of_mib_is_refused_and_nothing_is_stored` (real submit handler: `400 validation` on `memory`, store unchanged, describe 404) | AC-1.2, AC-1.3, DD-8 | L1 / L2 | yes |

AC trace (every DISCUSS acceptance criterion): AC-1.1 and AC-2.1 admission rows in the spec matrix, the deploy control and the grammar accept tables; AC-1.2 and AC-2.2 removed-key tests at parse, wire, deploy and the API body; AC-1.3 and AC-2.3 the grammar rejection classes plus the parse, deploy and handler refusals; AC-1.4 the byte-size round-trip proptests; AC-1.5 the describe tests; AC-1.6 the two fixtures; AC-1.7 the API body and OpenAPI tests; AC-1.8 the examples test; AC-1.9 the digest test, the canonical read-back and the accept tables; **AC-1.10** compiler-enforced field types, and the zero-memory refusal in the existing `aggregate_validation.rs` (see § AC-1.10 coverage); AC-2.4 the CPU accept table (`500m` and `0.5` are 500, `2` is 2000); AC-2.5 the CPU round-trip proptests and the digest test; AC-2.6 fixtures, API body and examples; AC-2.7 the grep gate named in AC-2.7 itself, run at FINALIZE (no test); **AC-2.8** `describe_prints_a_zero_cpu_as_the_canonical_zero` and the `CPU:` lines of the every-kind test.

Not tested here, by design: the vCPU and `memory.max` derivations downstream of the typed fields (`vcpus_for` stays covered by the existing `vm_resources_derivation` tests, AC-2.4); the operator docs (gated by FINALIZE and the AC-2.7 grep gate); GH #348 volume scenarios beyond the pinned refinement row (see § DELIVER obligations).

## Wave: DISTILL / [REF] WS strategy

Brownfield, extend in place (DISCUSS § WS strategy): no walking-skeleton scenario. The existing end-to-end path (`deploy` -> parse -> intent store -> `GET /v1/allocs` -> `workload describe`) is covered at its seams: parser (`workload_resources_spec`), command handler (`deploy_workload_resources`), submit handler, wire and OpenAPI (`workload_resources_wire`), renderer (`render_workload_resources`). Architecture of Reference: the driving ports are real (command handler, submit handler through the production router, the renderer); there is no driven external port in this feature, so nothing is faked beyond the `Sim*` composition the existing `service_stream_support` harness already provides.

## Wave: DISTILL / [REF] Adapter coverage

No driven adapter is added or changed (DESIGN § Driven ports): the parsers are pure functions. The examples test reads the real checked-in files and the OpenAPI test the real generator. No guest-placed or integration-lane test is added: a server round trip through `serve` adds no coverage the submit-handler test and the digest test do not hold, and the guest group cannot boot on the current Metal box.

## Wave: DISTILL / [REF] Scaffolds

| File | Scaffold | Behaviour today |
|---|---|---|
| `crates/overdrive-core/src/byte_size.rs` (`// SCAFFOLD: true`) | `ByteSize(u64)` with `new`, `bytes` (real, total), `ByteSizeParseError { Malformed, MissingUnit, DecimalUnit, Fractional, Overflow }` (each `{ input }`), `FromStr`, `Display`, `Serialize`, `Deserialize`; derives exactly as pinned | `FromStr` returns `Malformed` for every input; `Display` renders nothing; serde follows `Display` / `FromStr` |
| `crates/overdrive-core/src/cpu_quantity.rs` (`// SCAFFOLD: true`) | `CpuQuantity(u32)` with `from_millis`, `millis` (real, total), `CpuQuantityParseError { Malformed, UnknownSuffix { input, suffix }, TooManyDecimals, Overflow }`, `FromStr`, `Display`, `Serialize`, `Deserialize`; derives as pinned | same shape: refuses every input, renders nothing |
| `crates/overdrive-core/src/lib.rs` | `pub mod byte_size; pub mod cpu_quantity;` (two lines; no re-export, none is pinned) | |

Both follow the `volume.rs` precedent (file-level `#![allow(...)]` for the scaffold lints). `VolumeSize` and `ByteCount` are untouched; the volume refinement test is RED against the existing `VolumeSize` scaffold. No production field is renamed. Scaffold grep: `grep -rn "SCAFFOLD: true" crates/overdrive-core/src/byte_size.rs crates/overdrive-core/src/cpu_quantity.rs`.

## Wave: DISTILL / [REF] Test placement

| Where | Why (precedent) |
|---|---|
| `overdrive-core/tests/acceptance/*.rs`, wired in `tests/acceptance.rs` | `aggregate_validation.rs`, `vm_volume_size.rs`; the binary already carries `overdrive_test_default!(host_pure)` |
| `overdrive-core/tests/schema_evolution/workload_resources_fixtures.rs`, wired in `tests/schema_evolution.rs` | `.claude/rules/testing.md` § "Archive schema-evolution roundtrip"; the hex literals are read out of the owning fixture files with `include_str!`, never copied or edited |
| `overdrive-cli/tests/acceptance/*.rs`, `placement!(host_pure)` | `render_workload_describe.rs`; `crates/overdrive-cli/CLAUDE.md` § "What to write in `tests/acceptance`" (handler called as a library function, no subprocess); the deploy refusal needs no server because validation precedes `ApiClient::from_config` |
| `overdrive-control-plane/tests/acceptance/workload_resources_wire.rs`, `placement!(host_pure)` | `openapi_gate.rs` and `guest_listeners_http_rejection.rs` (same `placement!` form; the submit-handler test uses the same `Node::boot` harness) |

`cargo xtask placement-lint`: clean. No guest-placed and no `native_launcher` test.

## Wave: DISTILL / [REF] Driving adapter coverage

| Driving port (DESIGN § Driving ports) | Scenario |
|---|---|
| `overdrive deploy <SPEC>` (`commands::deploy::deploy`) | `deploy_workload_resources.rs`: accepted and refused specs, three spec shapes (`[job]`, `[service]`, flat) |
| submit handler (`POST /v1/workloads`) | `workload_resources_submit_refusal.rs::sad_memory_that_is_not_a_whole_number_of_mib_is_refused_and_nothing_is_stored` (integration-gated) (real handler through the router), `workload_resources_spec.rs::sad_the_wire_refuses_...` (the wire decode) |
| `overdrive workload describe` (`render::workload_describe`) | `render_workload_resources.rs`; `workload_resources_spec.rs::describe_reads_back_the_canonical_form` (the projection it renders from) |
| `api/openapi.yaml` | `workload_resources_wire.rs` (generator output; the existing OpenAPI gate holds the checked-in file equal to it) |

The deploy tests call the handler as a library function (`overdrive-cli/CLAUDE.md`: no subprocess). `deploy_streaming` (the TTY lane) shares the same parser and admission calls and is not exercised separately.

## Wave: DISTILL / [REF] Pre-requisites

DESIGN driving ports above; `cargo xtask metal run -- env OVERDRIVE_BPF_NATIVE=1 cargo xtask bpf-build` before any compile on a fresh checkout. No guest kit is needed: every test runs in the default lane.

## Wave: DISTILL / [REF] Fault and boundary obligations (testing.md § DISTILL)

This is a parser, admission and rendering feature: no concurrency, ordering, retry, timing, kernel or reconciler behaviour, no protocol, no Quint model (DESIGN § Formal protocol model: not applicable). No Tier 1 simulation, no seeded `overdrive-sim` invariant and no Tier 3 kernel evidence is owed. The composed-system obligation reduces to one contract:

| Contract | Owner path | Fault stimulus | Oracle | Tier |
|---|---|---|---|---|
| A refused size leaves no record, and an equal size in another spelling is the same spec | `POST /v1/workloads` (real submit handler) -> `WorkloadSpecInput` -> `from_submit` -> intent store | memory that is not a whole number of MiB, against the whole-MiB acceptance rows of the admission matrix | `400 validation` on `memory`; intent store unchanged; `describe` answers 404; `1024MiB` and `1GiB` give one digest (digest test) | default lane (handler and library) |

Bounds: each case is one request or one pure call, no duration or restoration obligation. The proptests carry the input-space boundary (`u64` / `u32` ceilings, drawn within two of the boundary in every unit, and case and unit permutations). Mutation target: the two parsers and the admission check (kill rate at least 80%, DESIGN § Technology choices); the should_panic bodies are the oracle the gate runs against.

## Wave: DISTILL / [REF] EDD expectation

One expectation is authored, not captured: `verification/expectations/O08-workload-resources-human-readable-refusal-and-describe/README.md` (indexed in `INDEX.md`, status `pending`). The claim is narrowed to what no assertion holds: a refusal "names the remedy" to a reader who has never seen the grammar, and `describe` reads the declared size back in canonical units on the built binary, from a handful of contrasting inputs. Grammar, admission, digest and rendering rules are covered by the test tiers and are not duplicated there. Evidence is captured at DELIVER or DEVOPS by a different agent from the implementer.

## Wave: DISTILL / [REF] AC-1.10 coverage

AC-1.10 (node capacity carries `CpuQuantity` and `ByteSize`) has no new test. The field types are compiler-enforced: a raw integer cannot be assigned once DELIVER renames `NodeSpecInput` and `Resources`. The zero-memory refusal is already covered by `aggregate_validation.rs`, which DELIVER's rename sweep updates mechanically. `NodeSpecInput` has no serde form, so no compile-stable test can pin "a sub-MiB node capacity is admitted" without naming the renamed fields; that half of AC-1.10 rests on `Node::new` keeping only the non-zero check (DD-8). The gated `node_capacity_resources.rs` of the first DISTILL cut was deleted.

## Wave: DISTILL / [REF] DELIVER obligations surfaced

1. Remove the `#[should_panic]` marker of each pending test when its behaviour lands, and run the body unchanged. The tests whose RED is the string wire (`red-classification.md` § "Tests whose RED is the string wire") are unreachable past the decode until the rename step; DELIVER removes their markers in that step and confirms the next semantic RED, if any, before changing production code.
2. Existing assertions made stale by DD-7 and the renames, to be updated in DELIVER with the fixtures sweep: the `Memory:        1024` goldens in `overdrive-cli/tests/acceptance/render_workload_describe.rs` (`ND295_SERVICE_GOLDEN`, `ND295_JOB_GOLDEN` gain a `CPU:` line and a canonical memory), the `Memory:` line in `guest_stack_mtls_egress.rs`, and the `memory_bytes` assertion in `vm_resources_sizing.rs`.
3. Reconcile `overdrive-core/tests/acceptance/vm_volume_size.rs` (GH #348, DESIGN § Corrections item 3): its `text_outside_the_grammar_is_rejected` strategy draws `[0-9]{0,6}MiB`, and `1024MiB` is now an accepted volume size; `the_volume_size_is_the_shared_grammar_refined_to_whole_gib` here pins the refinement rows.
4. The refusal text assertions are stated at the wording DESIGN pins and no further: `Malformed` names the grammar (the five units for a byte size; no wording is pinned for a CPU quantity), a decimal unit names the binary units, `Fractional` says "smaller", `UnknownSuffix` for `M` says "lowercase", `TooManyDecimals` says "three", `Overflow` says "too large", the whole-MiB refusal says "whole number of MiB" and the received canonical value. Wrapped surfaces (TOML, deploy, JSON) assert the key or field name and the class keyword only.

## Wave: DISTILL / [REF] Review dispositions, round 1

| Finding | Disposition |
|---|---|
| S-H1 shared decode RED | Applied: one named test (`a_non_canonical_spelling_is_read_back_canonically`) is RED on the TOML parser; the other string-wire tests share the helper text `string cpu/memory resources are not accepted yet`, recorded per test in `red-classification.md`; no probe added |
| S-H2 / Eclipse node capacity | Applied: `node_capacity_resources.rs` deleted; AC-1.10 coverage recorded above (compiler plus `aggregate_validation.rs`); no compile-stable sub-MiB test exists, stated as the residual |
| S-H3 pinned wording only | Applied across the grammar, spec, deploy, wire tests and the rejection-text tests |
| S-M4 boundary windows | Applied to the `u64` boundary proptest, and to the `u32` one, which had the same flaw |
| S-M5 canonical unit choice | Applied: `any_count_in_any_unit...` asserts the independent model's canonical text |
| S-M6 `NoCapacity` | Applied: one prose test kept; the file header records that the text is pinned by DESIGN and has no production emit site today (GH #261); the other tests dropped |
| Subtractive 1 + S-M7 | Applied: the guest-placed round-trip file replaced by one host-pure submit-handler test; the respelling case is held by the digest test and the existing `idempotent_resubmit.rs` |
| Removals 2-10, L8-L10 | Applied: vCPU test and helpers, bare-memory-number test, merged parse tests, two-spellings, fixed-point and ordering tests, missing-unit and too-many-decimals example tests, OpenAPI and docs name checks, how-to page test removed |
| Trims 12-19, S-L11, S-L12 | Applied: serde spelling tests, bare-number loops of the struct-level tests, malformed matrices on one kind, deploy malformed loops to one representative per shape, admission tests merged (zero CPU folded in; `1048575B` / `1048577B` added), OpenAPI on the generated document only, render tests merged; the flat deploy shape is live (`examples/coinflip.toml`) and kept |
| O08 | Applied: narrowed to the refusal-text and describe read-back claims |
| Docs | Applied: `red-classification.md` keeps the table and the notes only; the scenario table above is trimmed |
| DISCUSS delta (AC-1.10, AC-2.8, AC-2.4 narrowed) | Applied: AC trace above; `describe_prints_a_zero_cpu_as_the_canonical_zero` covers AC-2.8; the vCPU test is removed per the narrowed AC-2.4 |
