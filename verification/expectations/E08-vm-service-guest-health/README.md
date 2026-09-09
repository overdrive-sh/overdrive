# E08 — VM Service health belongs to the guest workload

Status: `satisfied` — independently audited in [E08 different-fox review](../../../docs/feature/service-kind-vm-workloads/deliver/review-e08-evidence.md)
Surface: E — built-product end to end
Execution substrate: `native-metal`
Walking skeleton: yes; sole walking skeleton for GH #257

## Expectation

The one checked-in `examples/service-kind-vm-workloads/` journey runs through
the built default-feature `overdrive` binary. `serve` provisions and boots the
VM; `deploy <SPEC>` is accepted; wildcard/omitted TCP and HTTP probe hosts
reach the guest `workload_addr`; startup produces Stable and readiness keeps
the backend eligible. The runner then deploys the checked-in VM Job client,
whose guest-installed static binary opens an ordinary plaintext socket to
`service-vm-e08.svc.overdrive.local:18081`; the platform's existing mesh path
selects the Service frontend and the Job succeeds only after receiving the
byte-exact guest reply `SVM-E08-GUEST-OK`.

## Anchors

- Anchor: S-SVM-01 in `docs/feature/service-kind-vm-workloads/distill/test-scenarios.md`.
- Anchor: US-SVM-1 and US-SVM-2 in `docs/feature/service-kind-vm-workloads/feature-delta.md`.
- Anchor: ADR-0090 (registration-time VM target projection).
- Anchor: ADR-0091 (ServiceSpecV3 and VM Exec exclusion).
- Anchor: K1/K2 in `docs/feature/service-kind-vm-workloads/feature-delta.md`.

## Exact public boundary

The activated runner must execute, in this order, under one canonical
native-metal lease:

```text
cargo build -p overdrive-cli --bin overdrive
examples/service-kind-vm-workloads/prepare.sh prepare
target/debug/overdrive serve --bind <isolated-bind> --data-dir <prepared-data-dir>
target/debug/overdrive deploy examples/service-kind-vm-workloads/service.toml
target/debug/overdrive workload describe service-vm-e08
target/debug/overdrive deploy --detach examples/service-kind-vm-workloads/client-healthy.toml
target/debug/overdrive workload describe service-vm-e08-client
```

Required oracle:

1. `serve` becomes ready without a test-only driver/runner override.
2. `deploy` exits 0; stdout orders one Accepted before one Stable event;
   Stable names startup probe index 0; stderr has no failure block.
3. `workload describe` exits 0 and identifies the existing VM driver plus TCP
   and HTTP results at the guest address.
4. The peer VM client Job reaches ordinary `Succeeded` only after resolving the
   Service name and receiving byte-exact `SVM-E08-GUEST-OK` through the
   Service frontend. Its checked-in `[job] + [vm]` spec runs the static client
   installed in the private rootfs by the bundle's one E07-style `prepare.sh`.
   The binary holds no SVID and opens only a plaintext socket; the platform
   originates mTLS. A host-to-`workload_addr` request is not accepted as the
   Service-traffic oracle because H6 already proves that narrower path.
5. Stop/runner teardown proves zero owned VMM, probe task, netns, TAP/veth,
   cgroup-scope, VM run-directory, mounted rootfs, loop device, or
   marker-owned preparation-tree delta.

Arguments, exit codes, complete stdout/stderr, client terminal result,
allocation ID, effective guest address, and cleanup counts are captured
verbatim. The runner
must not call Cargo tests, import/link an `overdrive-*` crate, reproduce probe
logic in shell, or use the discarded H6 spike as the system under test.

DISTILL intentionally leaves execution pending because the current parser
rejects `[service] + [vm]`. `runner.sh` returns the catalogue's fail-closed
pending code 75 until DELIVER activates the product example.
