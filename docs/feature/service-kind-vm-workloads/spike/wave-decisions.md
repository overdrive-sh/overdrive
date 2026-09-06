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
