# Immediate-close UDP slot ownership: reproduced path and correction

Date: 2026-10-24. The retained increment-C and increment-D probes ran the
production `overdrive-init` as guest PID 1 in disposable Cloud Hypervisor
guests on the Metal runner. The guests reported Linux `7.0.0-38-generic`.
Probe source and raw run output remain in the adjacent increment directories;
the probes did not modify production or permanent test sources.

## Contract and reachable owner path

The accepted slot contract says that `overdrive-init` owns the
`slot -> (cookie, destination)` table, recovers lost `sock_release` reports by
an audit, and releases a slot in abort (when a pair exists), receive-queue
discard, disconnect, then pool-return order. The `GuestOwnerIo` contract
already exposes `slot_owner`, `end_association`, `release_slot`, and
`live_cookies`; no additional interface was needed (`feature-delta.md` § Guest
intake model and § GuestFlowOwnerIo).

The reproduced path is an ordinary UDP send whose application socket closes
before `SlotReady` is dispatched. The guest capture program claims
`SLOT_META`/`SLOT_OWNER`; `sock_release` reports the cookie but does not free
the slot. `KernelGuestOwnerIo::poll_once` later reports `SlotReady`, and
`GuestFlowOwner::start_datagram` asks `slot_owner` before adding an owner-table
entry. The kernel adapter read the cookie and destination from `SLOT_META`,
then used `diagnostic_source_port(cookie)` to find the still-live application
socket. If the application had closed, that lookup returned `None` and the
adapter returned `Ok(None)` without cleaning up the kernel claim. Consequently
the owner had no published slot owner for either `SocketReleased` or
`audit_slots` to select.

## Reproduction and control

The C guest creates 1,100 ordinary UDP sockets, sends one datagram, and drops
each socket without waiting for an echo. It waits through three one-second
owner audit periods, then tries a fresh send and requires the exact echo.
Against the unmodified production adapter, 1,024 sends were admitted, 76 were
correctly refused with `EPERM` while the pool was full, all application
sockets were closed, and the post-audit send still returned `EPERM`. The
guest exited 2 and the host observed zero requests. Raw evidence:

- `increment-c/evidence/reruns/default-kit-20261024T120620Z-2078/runner.raw.log`
- Kernel line: `Linux 7.0.0-38-generic`.
- SHA-256: `2a68bee896ec581fe427797fb5d96bc0e568e6c046e833e34473db572962f4ca`.

The unchanged D contrast receives and checks each of its 1,100 exact echoes
before closing the corresponding socket. Before and after the correction it
passed all exchanges and the fresh post-audit send (`HOST_REQUESTS=1101`,
guest exit 0). This confirms that ordinary observed-and-released slots recycle
and isolates the missing owner-publication path in C.

## Bounded implementation and post-fix evidence

When `slot_owner` can read a valid kernel claim but the application cookie no
longer appears in inet-diag, no guest owner flow or forwarder pair has been
created yet. The adapter now calls the existing `end_association(slot)` and
then `release_slot(slot)`, and returns `Ok(None)`. The first operation
discards the slot socket's receive queue and disconnects it; the second
returns the index to the existing pool. This follows the accepted no-pair
release order and uses no new type, method, map, owner table, retry, or model
state.

The same C workload then exited 0: 1,032 sends were admitted and 68 were
legitimately refused while the finite pool was occupied; after all sockets
closed and the audit wait elapsed, the fresh send was echoed. The host observed
one request, the post-audit probe. Raw evidence:

- `increment-c/evidence/reruns/default-kit-20261024T121819Z-14810/runner.raw.log`
- SHA-256: `456f251699d78256dbb605d46c163d09463543c626aec9f3d41f439b9bdafc92`.

The unchanged D contrast also passed after the correction:

- `increment-d/evidence/reruns/default-kit-20261024T121945Z-16107/runner.raw.log`
- SHA-256: `e212d77a56fc4e86c4125434ce23bba4faba11cbf6a0ad85ab0ab4fa312b853e`.

Both C and D were executed through their existing `run.sh default-kit`
commands. The runner, guest console, exit status, and ownership-repair receipt
are retained under each cited run directory. Source hashes:

| Probe source | SHA-256 |
|---|---|
| C `src/host.rs` | `8ad9da23c98714fd950e5c7be1d2ef364373355d7db47081118990b85b2c476a` |
| C `src/guest.rs` | `f400179cb28dfe16173dc36ca77ea5c7d8bdf590843c2e508e43399f29f4191d` |
| D `src/host.rs` | `f5355fc674a905854d25f35243b4f3f877e0e797fbeaa0ee94238bf60c23015b` |
| D `src/guest.rs` | `1382dae459d6f665ae41e8da5af8b1c924385bf158b7101c9ca2a9e010196cfa` |

The C build and console output were retained locally before the necessary
metadata-only ownership repair for canonical Metal sync; the repair receipts
record the original output hashes and confirm bytes and modes were preserved.
