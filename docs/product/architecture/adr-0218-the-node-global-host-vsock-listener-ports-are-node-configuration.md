# ADR-0218 — The node-global host vsock listener ports are node configuration

## Status

**Accepted — user ruling PORTS-CONFIG, 2026-10-19** ("DISTILL rulings, after
round 10"), which replaces the earlier ruling of the same day that kept the
ports hardcoded (U-1 (b)). Scope narrowed by the user's ruling
NODE-CONFIG-LOADER (2026-10-20): the node-configuration file section is GH
#327, not #295 (Decision, "Deferred to GH #327"). GH #295. Not yet implemented. The configuration
type, its validation and the owners' bound-port accessors are architect
elaborations under the ruling, pending adversarial review. Contract: the
#295 feature delta, § *[REF] Composition — production entry points*, "Node
vsock ports (PORTS-CONFIG)".

Related decisions, each separately reversible: the guest learns the ports
from the kernel command line its VM driver builds (ADR-0217); `run_server`
receives its driven adapters from the composition root (ADR-0219); guest
connections are attributed by peer CID and launch token (ADR-0177); the
control session has its own listener (ADR-0166).

## Context

A node runs three host vsock listeners that every guest of the node
connects to: the VM driver's beacon (1234), the guest-flow owner's flow
stream listener (1240) and its control listener (1243). Each binds
`VMADDR_CID_ANY` on one port. A vsock port is unique per network namespace:
a second bind of a bound port fails `EADDRINUSE`
(`net/vmw_vsock/af_vsock.c:820-821 @ v7.0`).

With the ports constants, two `serve` processes on one host — two
development workspaces sharing one Lima VM, or a VM test beside a non-VM
`serve` test — cannot both boot. Keeping non-VM tests from binding the ports
required a simulated seam in the production boot, which the user rejected
(NO-OVERRIDE-SEAMS, ADR-0219). The kernel can choose a free port: a bind to
`VMADDR_PORT_ANY` (`-1U`, `include/uapi/linux/vm_sockets.h:109 @ v7.0`)
draws from a per-network-namespace counter, seeded once at random above
`LAST_RESERVED_PORT` (1023, `include/net/af_vsock.h:19 @ v7.0`;
`af_vsock.c:787-788 @ v7.0`) and shared with every outbound auto-bind
(`vsock_auto_bind` binds `VMADDR_PORT_ANY`, `af_vsock.c:251-259 @ v7.0`):
each try takes the counter's value and advances it, wrapping to 1024, and
the first unbound value wins; after `MAX_PORT_RETRIES` (24,
`af_vsock.c:229 @ v7.0`) consecutive bound values the bind fails
`EADDRNOTAVAIL` (`af_vsock.c:792-810 @ v7.0`). `getsockname` reports the
port the bind took (`vsk->local_addr`, set at `af_vsock.c:824`, read at
`:1133-1138 @ v7.0`).

## Decision

- **The three ports are node configuration.** Each is either a fixed port or
  kernel-assigned. The production defaults are fixed 1234, 1240 and 1243.
- **A port's owner reports the port it bound.** The beacon intake reports
  the beacon port, the guest-flow owner reports the stream and control
  ports; consumers read the bound value, never the configuration.
- **A port is resolved once per process.** Each owner holds its listening
  socket for the process's life. A listener repair after the accept task
  ended re-spawns accept on the held socket, which is still listening
  (`SO_ACCEPTCONN`, `net/core/sock.c:1993-1995`; `TCP_LISTEN` set at
  `af_vsock.c:1941 @ v7.0`), so it never closes or binds again and a guest
  started earlier keeps reaching the same port. A restart reads the
  configuration again; every VM of the previous process is reclaimed before
  the first assignment (NO-SURVIVE), so no running guest holds a port of a
  previous process.
- **Validation:** a fixed port must lie above the reserved range (1–1023;
  binding there needs `CAP_NET_BIND_SERVICE`, `af_vsock.c:815-818 @ v7.0`)
  and must not be `VMADDR_PORT_ANY`; the three fixed ports are pairwise
  distinct; and the three ports are either all fixed or all kernel-assigned
  — a kernel-assigned port bound at boot step 2 could take a fixed port that
  is bound only at step 9. The configuration value is validated where it is
  constructed, so an invalid port set never reaches the boot.
- **Where the configuration is set.** It is a field of the server's
  configuration value. `overdrive serve` passes the production defaults. A
  test passes kernel-assigned ports. Under GH #295 no configuration-file
  section or command-line flag carries it, because `serve` reads no node
  configuration file. Loading the operator node configuration file is
  GH #327 (user ruling NODE-CONFIG-LOADER, 2026-10-20); the file section it
  would read is recorded below.

### Deferred to GH #327 — the `[vsock]` node-configuration section

**Not part of the #295 decision and not to be implemented under #295.**
This design text is kept here so that GH #327, which makes `overdrive serve`
load the operator node configuration file (ADR-0025 `[node]`, ADR-0049
`[dataplane]`), inherits it. #327 decides whether to adopt it as written.

- An optional `[vsock]` section with three keys: `beacon_port`,
  `flow_stream_port` and `flow_control_port`. A missing section or key
  takes the production default of that listener (1234, 1240, 1243).
- In the file, the value `0` means kernel-assigned (`VMADDR_PORT_ANY`).
  Any other value is a fixed port. The file convention exists because TOML
  has no natural spelling for the typed `KernelAssigned`; the Rust API
  never treats `0` as kernel-assigned (a literal fixed `0` is in the
  reserved range and is refused).
- The parsed values go through the same constructor and validation as the
  in-process configuration (reserved range, `VMADDR_PORT_ANY`, all fixed or
  all kernel-assigned, pairwise distinct). A bad value refuses the start
  before any boot effect, with `Validation { field: "vsock.<key>" }`
  naming the offending key.
- The section was withdrawn from #295 because `serve` reads no node
  configuration file today: a `[vsock]` parser would have no production
  caller, which is also true of today's `[dataplane]` parser.

## Alternatives considered

- **Keep the ports hardcoded and keep non-VM tests from binding them** (the
  round-10 `GuestFlowIoOverride` and `vm_override` seams). Puts simulation
  adapters into the production boot's configuration, and VM tests still
  collide across workspaces. Rejected by the user.
- **Per-test network namespaces for `serve` tests.** The bound-port table
  is per namespace (`__vsock_find_bound_socket_net`, `af_vsock.c:803`,
  `:820 @ v7.0`), but production `serve` runs in the root namespace and its
  boot writes other root-namespace state (the gateway route, the
  `overdrive-mtls` table); a namespaced `serve` test would no longer run the
  production composition. Rejected.
- **Kernel-assigned ports only, no fixed default.** Correct, but a fixed,
  documented port is easier to observe on a production node and costs
  nothing there. Rejected as the production default; available as a value.

## Consequences

- Tests configure kernel-assigned ports and read the bound ports back, so no
  two `serve` processes on a host collide on a vsock port. Node-global
  kernel state other than the ports (the gateway route, the nft table)
  still serializes those tests in `host-kernel-shared`.
- The guest no longer hard-codes the ports (ADR-0217).
- No repair window exists in which an outbound auto-bind could take a
  fixed port: a repair keeps the listening socket. At boot the fixed ports
  are bound before any host vsock connection of the process is opened (the
  beacon at step 2, the flow listeners at step 9, the first dial after
  them), and the `Mixed` refusal keeps a kernel-assigned beacon from taking
  a fixed flow port.
