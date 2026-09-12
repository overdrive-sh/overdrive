# Service-kind VM guest health

This checked-in product example is one bundle supporting E08–E13. E08 is the
sole `service-kind-vm-workloads` walking skeleton; E09–E13 are bounded
non-walking-skeleton failure, matrix, lifecycle, and compatibility modes. The
healthy Service declares wildcard TCP startup and omitted-host HTTP readiness,
both of which must reach the guest address provisioned by Overdrive.

Run on the qualified native x86_64 metal host through:

```text
cargo xtask metal run -- examples/service-kind-vm-workloads/run-example.sh run healthy
```

`run-example.sh run healthy` is the active E08 product journey. It builds the
default-feature binary, prepares the checked-in guest bundle, runs one native
metal `serve` session, and uses only public `deploy`, `workload describe`, and
`job stop` commands. `prepare.sh` is the one checked-in native-metal
materialization and cleanup path. It directly follows E07's
`guest-stack-transparent-mtls-intercept` ownership token, pre-write trap,
bounded mount/loop cleanup, static-binary verification, private-rootfs, and
marker-owned removal conventions. The activated modes must add E07's isolated
session, bounded process ownership, and public-stop lifecycle without creating
a second preparation harness. They build Overdrive with default features and
use only its public CLI plus ordinary workload traffic; they may not
link/import an Overdrive crate or invoke a Rust test binary.

`guest_server.rs` is a dependency-free workload used in both VM and Exec HTTP
matrix cells. TCP/18081 answers the raw backend path; HTTP/18080 supports fixed
204/302/404/503 startup results, timed readiness 204→503→204, timed liveness
failure, and the byte-exact `SVM-E08-GUEST-OK` body at `/`. Every 503 health
response carries the nonempty bounded sentinel
`SVM-E10-FAILURE-BODY-MUST-NOT-LEAK`; E10 requires that deletion-sensitive
fixture to remain absent from all operator-visible output.

`client.rs` follows the checked-in dial-by-name and E07 VM Job precedent. Every
`client-*.toml` fixture is `[job] + [vm]`, runs the guest-installed static
`/opt/overdrive/examples/svm/e08-client`, resolves
`<service>.svc.overdrive.local`, and opens an ordinary plaintext TCP connection
through the Service frontend. The client binary holds no SVID; the platform
originates mTLS. Its VM Job succeeds only for the expected byte-exact reply
(or, in a negative-control mode, only when the known-unhealthy backend never
returns that reply). A direct host request to `workload_addr` is probe-path
evidence, not the Service-traffic oracle.

## Materialization contract

`prepare.sh check-source` proves that all seven peer clients remain VM Jobs
with the accepted kernel/rootfs fields and guest client command. It separately
pins E10's four `[service] + [exec]` files as cross-driver controls, never as
client Jobs. On qualified native metal, `prepare.sh prepare`:

- compiles `guest_server.rs` and `client.rs` for
  `x86_64-unknown-linux-musl` and rejects either binary if it has a dynamic
  interpreter;
- copies the selected kernel, reflinks one private rootfs at the paths named by
  every VM fixture, and installs both static binaries at their declared guest
  paths;
- retains the host `e08-server` binary solely for E10's existing Exec Service
  control cells;
- co-locates the rootfs and isolated serve data beneath
  `/srv/vm/overdrive-testing/svm-e08` with the confined-VMM traversal modes;
  and
- creates the isolated credential file used by the eventual E07-style serve
  session.

Preparation arms cleanup before its first write. Failure and signals perform
bounded unmount and loop detach, then remove only process-owned partial output.
Committed output carries `.svm-e08-owned`; explicit cleanup refuses an
unmarked, foreign-token, or mounted tree and removes only the fixed
`/srv/vm/overdrive-testing/svm-e08` materialization. `prepare.sh check`
remounts the private image and independently verifies both guest-installed
static binaries.

Modes and owners:

- `healthy` → E08, the sole walking skeleton;
- `tcp-truthfulness-100` → E09/K1, 100 isolated healthy/failing pairs;
- `http-status-cross-driver` → E10/K2, Exec/VM × 204/302/404/503; each
  isolated cell retains its public deploy command/output/exit, recovery and
  stop observations, before/after describe rows, current-session resource
  witness, and absolute named-resource cleanup complement before the eight-row
  ledger is derived; the six failure PTY summaries retain the terminal-only
  typed `Error:` block and exact HTTP status, omit the success-only CLI
  `Accepted.` prefix, and exit 1; a positive disappearance receipt is retained
  only for resources observed before stop, while an already-clean observation
  still passes only when every allocation-named resource remains absent; a
  repeated failed-probe observation must preserve its identity, configuration,
  status, and reason while its observation timestamp may advance but never
  regress;
- `readiness-recovery` → E11/K3, peer traffic before/during/after withdrawal;
- `liveness-restart` → E12, describe-visible restart; and
- `zero-probes` → E13, inferred TCP success/failure compatibility.
