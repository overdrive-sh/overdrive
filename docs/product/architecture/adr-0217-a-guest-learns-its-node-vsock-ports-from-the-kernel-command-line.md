# ADR-0217 — A guest learns its node's vsock ports from the kernel command line

## Status

**Accepted — user ruling GUEST-PORTS, 2026-10-19** ("DISTILL rulings, after
round 10"). GH #295. Not yet implemented. The carrier (three fields
appended to the one `overdrive.transport=` token), their names and the
guest's refusal are architect elaborations under the ruling, pending
adversarial review. Contract: the #295 feature delta, § *[REF] Guest
adaptation contract*, "Kernel command-line token", and DG-5 "Transport token
and setup".

Related decisions, each separately reversible: the node-global host vsock
ports are node configuration (ADR-0218); the guest's connections are
attributed by peer CID and the launch token on the same command line
(ADR-0177); a guest reset ends the VMM, so a command line names one boot
(ADR-0182).

## Context

A Linux guest's `overdrive-init` opens three kinds of connection to the
host: the beacon (READY, exit reports), the flow stream connections and the
control session. Each goes to (`VMADDR_CID_HOST`, port). Under ADR-0218 the
host's ports are node configuration and may be kernel-assigned, so a port
compiled into `overdrive-init` is no longer the port the host listens on.

The VM driver already passes per-launch facts to the guest on the kernel
command line it builds: the `overdrive.transport=` token carries the
workload address, the DNS gateway and the launch token, and `overdrive-init`
parses it before it configures the network or connects to the host.

## Decision

- **The VM driver passes the three host ports on the kernel command line it
  builds**, as three fields appended to the one `overdrive.transport=` token:
  `beacon=<port>,stream=<port>,control=<port>`, decimal. The beacon port is
  the one the driver's own beacon intake bound; the stream and control
  ports are the ones the guest-flow owner bound, carried to the driver in
  the allocation's transport assignment.
- **`overdrive-init` connects only to the ports its token names.** It holds
  no port constant for the host listeners.
- **A token whose port field is missing, not a decimal `u32`, in the
  reserved range 0–1023, equal to `VMADDR_PORT_ANY`, or equal to another
  port field is refused** before the guest configures anything, with the
  field named; `overdrive-init` reports it on the console and takes its
  existing pre-READY exit, which the driver observes as a start failure.

## Alternatives considered

- **A second kernel parameter** (`overdrive.ports=`). Two tokens to keep
  consistent, a second "missing" and "duplicate" case, and no separation
  gained: both are written by the same driver for the same launch.
  Rejected.
- **The guest asks the host for the ports at boot.** The guest cannot reach
  the host before it knows a port. Rejected.
- **Keep the ports compiled into `overdrive-init`.** Contradicts ADR-0218's
  kernel-assigned ports. Rejected by the ruling.

## Consequences

- The guest contract changes: the transport token has seven fields, and an
  `overdrive-init` built before this change cannot boot under a driver
  built after it. Both ship in one image (greenfield single cut).
- A guest's ports are fixed for its boot; the host keeps them for the
  process's life (ADR-0218), and a `serve` restart reclaims every VM.
- This decision covers Linux guests running `overdrive-init`; how a guest
  that runs no Linux kernel learns the ports is not decided here.
