# Slice 01: `[resources]` takes human-readable `cpu` and `memory`

Stories: US-1 and US-2 (`feature-delta.md`). Job: J-OPS-002 (secondary J-OPS-003). Estimate: 2 to 2.5 days plus mechanical fixture churn (about 242 files under `crates/` and 33 example specs; +0.5 to 1 day). Reference class: field rename plus type change across the existing deploy -> intent -> describe path.

## Why one slice

`Resources`, both `ResourcesInput` twins and `ResourcesBody` carry `cpu` and `memory` in one struct, and the design renames both fields together.
Splitting by field would ship a mixed `cpu_milli` + `memory` spec form for one commit window, and rewrite the same fixture literals, the three
OpenAPI schemas and the 33 example specs twice. The two stories keep their own acceptance criteria; they are one DELIVER cut.

## Goal

An operator writes `memory = "128MiB"` and `cpu = "500m"`, deploys, and `overdrive workload describe` reads back `Memory:        128MiB` and `CPU:           500m`; `vcpus_for` is unchanged; `memory_bytes` and `cpu_milli` no longer exist anywhere operator-facing.

## IN scope

- `ByteSize` and `CpuQuantity` in `overdrive-core` on the grammars pinned in `feature-delta.md` § Interface contracts: `FromStr`, canonical `Display`, serde string, typed rejection.
- `Resources`, both `ResourcesInput` twins, `NodeSpecInput`, `ResourceEnvelope` and `ResourcesBody` carry the newtypes as `cpu` and `memory` (DD-4).
- Admission: zero memory and non-whole-MiB memory refused at `Job::from_submit` and the service constructor (DD-8); `vcpus_for` and `cpu_weight_for` take a `CpuQuantity`.
- No envelope version: existing golden fixtures decode unchanged through the new types, hex literals untouched (DD-5).
- `api/openapi.yaml` regenerated; `cpu` and `memory` as `type: string`.
- `render::workload_describe` prints the canonical `Memory:` and a new `CPU:` line (DD-7).
- The grep-gate sweep of `feature-delta.md` § Component decomposition: fixtures, raw literals, examples, website how-to, the E06 runner script, `benchmarks/catalogue.md`; the four sub-MiB sim fixtures raised to at least 1 MiB.
- Old keys refused through `deny_unknown_fields`.

## OUT of scope

- Resource limits vs requests, CPU pinning; `[vm.volume]` restructuring; (GH #348's shipped `VolumeSize` and its `vm_volume_size.rs` expectation are IN scope, per R-4;) no migration or compatibility read (D-8).

## Learning hypothesis

Disproved if: one shared byte-size grammar cannot serve memory and the volume size without changing the volume contract, no single grammar expresses sub-core fractions and whole cores unambiguously, a `cpu` string parses to millicores other than the operator meant, or an existing golden fixture fails to decode through the new types. Confirmed if: all four hold.

## Acceptance criteria

AC-1.1 to AC-1.10 and AC-2.1 to AC-2.8 in `feature-delta.md`.

## Dogfood moment

Deploy `examples/service-kind-vm-workloads/service.toml` with `memory = "128MiB"` and `cpu = "500m"`, then one VM service with `cpu = "2"`; observe `CPU:           500m` and `CPU:           2` in `workload describe`. Compare `workload describe` output and spec digest with the same spec spelled `"1024MiB"` and `"1GiB"` (production example specs, not synthetic).

## Taste tests

- Two value types and one field rename across the existing path; no new container, port or dependency.
- The CPU grammar tests (AC-2.4) make the CPU hypothesis falsifiable: each `cpu` spelling has a pinned millicore value.
