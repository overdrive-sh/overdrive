# Service-kind VM workloads spike decisions

## 2026-09-06 — Design evidence gate disposition

**Decision:** DISCARD from production; retain committed evidence until the
implementation it informs supersedes it.

H1–H6 were ad-hoc probes used to resolve feasibility questions before
Application/component DESIGN. They were not a production walking skeleton and
the user did not choose PROMOTE. The probe code therefore remains isolated
under `spike-scratch/service-kind-vm-workloads/`; no production crate imports or
builds it, and no test or CI tier depends on it.

The measured constraints may inform a separately reviewed design. No
provisional port number, protocol vocabulary, concurrency value, process-group
mechanism, Rust API, or component ownership in the harness is promoted by this
decision.

## 2026-09-06 — Codec A/B spike disposition

**Decision:** DISCARD the throwaway codec harness from production; retain its
source, commands, failed attempts, raw captures, and findings as committed
design evidence.

The native-metal A/B measured a representative existing-JSON baseline plus
either a typed JSON VM-Exec codec or a bytechecked `rkyv` VM-Exec codec. It did
not create an accepted wire type or production API, so none of the harness is
promoted into a crate or test tier.

**DESIGN decision informed by the spike:** use `rkyv` for the VM-Exec control
protocol. Apply the project's existing rkyv versioning discipline: a per-type
versioned envelope enum whose variants retain historical payload types,
writers select the latest variant, readers validate before access and
up-convert known historical variants to the latest payload, and schema bumps
append a variant while preserving golden bytes for every prior version. Do not
carry the spike's standalone integer schema-prefix experiment into the design
as a second versioning mechanism.

## 2026-09-06 — VM Exec split from GH #257

**Decision:** GH #257 ships VM Services with guest-targeted HTTP/TCP probes and
rejects VM Exec probes at parse time before intent commit. Optional in-guest
Exec is deferred to GH #280.

The research and spikes remain valid feasibility evidence for GH #280, but
none of their guest-control, codec, persistent-supervisor, session, or process-
containment mechanisms belong to the active `service-kind-vm-workloads`
design. The earlier `rkyv` selection is preserved as direction for GH #280,
not as a dependency or implementation requirement for GH #257.
