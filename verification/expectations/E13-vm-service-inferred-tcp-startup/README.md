# E13 — zero declared probes infer guest-targeted TCP startup health

Status: `satisfied` (native capture and independent evidence audit complete; see [final audit](../../../docs/analysis/review-02-04-final-evidence.md))
Surface: E — built-product end to end
Execution substrate: `native-metal`
Walking skeleton: no; bounded compatibility journey

## Expectation

Two VM Services declare listeners and no health-check table. The bound-listener
case must reach Stable and describe an inferred TCP startup Pass at the guest
workload address. The unbound-listener case must fail with
`StartupProbeFailed`, remain ineligible, and describe an inferred TCP failure
at its guest workload address. This preserves the existing no-declaration
Service experience across the VM driver.

- Anchor: S-SVM-29 in `docs/feature/service-kind-vm-workloads/distill/test-scenarios.md`.
- Anchor: US-SVM-1 and ADR-0058 inferred startup compatibility.
- Anchor: ADR-0090 VM default-target projection.

## Verification

The activated `zero-probes` mode deploys `zero-probes.toml` and
`zero-probes-failure.toml` through separate isolated built-product instances.
It then deploys their matching `client-zero-probes*.toml` VM peer Jobs. Both
run the guest-installed static client from the bundle's one E07-style private
rootfs materialization. The ledger records deploy exits, terminal states,
describe probe `inferred` flag, effective target and port, the VM peer Jobs'
public terminal results, eligibility, and cleanup. Passing requires both
complementary outcomes, an exact guest reply only through the healthy Service
frontend, no host Exec client, and no explicit health-check block in either
checked-in Service spec.
