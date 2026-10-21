# ADR-0198 — The flow-forwarder module unloads only when nothing references it

## Status

**Accepted — EVMOD-LIFE, approved by the user 2026-10-17, with the user's
ruling MODULE-REFCOUNT of 2026-10-20**: `ovd_fwd` is unload-safe by module
reference counting. Not implemented. GH #295.
Follows from ADR-0197 (accepted 2026-10-16) and the user's design input of
that ruling: "the module is built into the appliance kernel, or otherwise
cannot unload while any socket is hooked". This ADR decides only how long
the module's text lives. How the module is built is ADR-0203 (EVMOD-BUILD),
who loads it ADR-0205 (EVMOD-LOAD), which module a Linux guest carries
ADR-0206 (EVMOD-GUEST-KIT); how a pair's freed state is kept from a late
callback is ADR-0204 (EVMOD-SAFE). Each could be kept while another is
replaced. The contract is in the #295 feature delta, § *[REF] vsock data
path — the in-kernel event-driven forwarder (EVMOD)*, "Module placement and
lifetime", and § *Pair end*, "Stale invocations", rule 5.

## Context

The forwarder of ADR-0197 replaces each forwarded socket's `sk_data_ready`,
`sk_state_change` and `sk_error_report`, and takes its write-space wakeups
from an entry on the socket's wait queue (ADR-0204, rule 5). Three facts make
unloading the module a memory-safety question:

- **Callbacks are not all run under RCU.** The vsock receive path calls them
  in process context with the socket lock held
  (`net/vmw_vsock/virtio_transport_common.c:1661`, `:1693`, `:1705 @ v7.0`),
  so `synchronize_rcu()` alone does not prove that no CPU is still executing
  module text (`spike/findings-evmod.md`, caveat 6). Other paths run inside a
  region a vanilla RCU grace period waits for (TCP input with the socket not
  owned; the vsock orphan sweep under `spin_lock_bh`; UDP receive under
  `rcu_read_lock`; `include/linux/rcupdate.h:671-675`, `:687-691 @ v7.0`).
- **`sk_write_space` has a caller with neither.** For a vsock or UDP socket,
  `sock_wfree` calls `sk_write_space` from preemptible process context with
  no lock and no RCU section (`net/core/sock.c:2675-2693 @ v7.0`; the vhost
  worker and the guest's virtio-vsock transmit work free every vsock
  transmit skb, `drivers/vhost/vsock.c:252`,
  `net/vmw_vsock/virtio_transport.c:361`). A task preempted in an override
  of it is waited for by nothing the module could hold.
- **Another subsystem can save a pointer into the module and call it after
  unload.** With kTLS installed after the module hooked a socket, the TLS
  ULP saves the module's `sk_data_ready` (`net/tls/tls_sw.c:2693-2694 @
  v7.0`), calls it from `tls_rx_msg_ready` (`:2511-2516`) and restores it on
  release (`:2603-2606`). The follow-up spike reproduced a guest panic by
  unloading the module in that state (`spike/findings-evmod-followup.md`,
  item 4). ADR-0200 refuses to attach a socket that carries a ULP, and no
  Overdrive owner installs one after attach.

The kernel unloads a module only when its reference count is zero.
`delete_module` refuses a module that is still initialising or already
going with `-EBUSY` (`kernel/module/main.c:810-816 @ v7.0`), and a module
with an `init` and no `exit` with `-EBUSY`, permanently (`:818-825`). Otherwise it calls
`try_stop_module` (`:828`), whose `try_release_module_ref` subtracts the
loader's base reference and, if any other reference remains, puts it back
(`:730-742`). The syscall then returns `-EWOULDBLOCK` (`EAGAIN`) and runs no
`exit` (`:744-751`). Only at zero does it mark the module `GOING`, after
which `try_module_get` fails (`:907-920`), and call the module's `exit`
(`:832-835`) before freeing it (`:847`). The one bypass is
`CONFIG_MODULE_FORCE_UNLOAD`, under which `delete_module(…, O_TRUNC)` skips
the count (`:714-721`, `:747-751`). A kernel without `CONFIG_MODULE_UNLOAD`
has no unload path: `delete_module` is the not-implemented stub
(`kernel/sys_ni.c:92 @ v7.0`), and `try_module_get` / `module_put` /
`__module_get` are inline no-ops (`include/linux/module.h:722-736 @ v7.0`).

Measured on Linux 7.0 (P-70, `spike/findings-ev-ctl.md` § 3.1) with a module
without `exit`: `delete_module` refused for every flag with and without live
pairs, and a live pair kept forwarding. The spike's own `exit` (detach every
pair, then return) was measured clean without kTLS (90 of 90) and panicking
with a saved kTLS pointer.

## Decision

`ovd_fwd` has `module_init` and `module_exit`, and it holds a module
reference for every object through which the kernel can still call into its
text. Unload therefore happens only when nothing can call it: `delete_module`
returns `-EWOULDBLOCK` while any reference is held and succeeds at zero.

- **References.** An open forwarder context holds one: the misc device's
  `file_operations.owner` is the module, so `misc_open` takes it through
  `fops_get` (`drivers/char/misc.c:131`, `include/linux/fs.h:2336-2339 @
  v7.0`) and `__fput` drops it with `fops_put` only after the release
  returned (`fs/file_table.c:468-469`, `:474`). Every hooked socket holds
  one — each half of a pair and each watched socket — taken with
  `__module_get` (`kernel/module/main.c:898-904`) inside attach or watch,
  under the context's reference, before its overrides are installed, and
  dropped with `module_put` (`:923-932`) as the last statement of the pair's
  or watch's free work. That work runs on the module's own workqueue after
  the RCU grace period and `cancel_work_sync` of ADR-0204's rule 3, and after
  rule 5: each half's write-space wait-queue entry was removed and its
  callbacks were restored with its socket lock held. So no module text can
  be running or about to be entered on that socket's behalf. A TCP socket
  that has a ULP installed when its overrides are removed keeps its
  reference for the life of the boot: the ULP may hold a copy of an
  override (a vsock or UDP socket cannot carry a ULP).
- **Work in flight** holds no reference of its own. Every work item belongs
  to a pair or watch whose socket references outlive it, and runs on the
  module's workqueue. The free work's own return after its `module_put` is
  waited for by `module_exit`'s `destroy_workqueue`, which drains every
  pending and running item (`kernel/workqueue.c:5962`, `:5979`, `:4183-4194
  @ v7.0`).
- **`module_exit`** runs only at zero references, so no context is open and
  no socket is hooked. It deregisters the device, destroys the workqueue
  (the drain above), then frees the scratch pool. The module registers no
  timer, notifier, `call_rcu` or `kfree_rcu` callback, so no `rcu_barrier()`
  is needed; its wait-queue entries live and die with the hooked sockets'
  references.
- **Forced unload stays off** wherever modules can be unloaded, because it
  bypasses the count. The appliance image kernel has no module unloading
  (ADR-0068 § 7), so nothing unloads a module there and forced unload cannot
  be configured. The stock 7.0 kernels of the development and Tier-3 hosts
  keep unloading with forced unload off.
- **No permanence check.** The control device reports its ABI version only.
  The probe reads neither a permanence flag nor `/proc/modules`.

## Alternatives considered

- **Permanent: `module_init` and no `module_exit`.** It is safe for any
  saved pointer, but a module change then needs a kernel boot. On a shared
  development VM that reboot ends every workspace's in-flight runs, and every
  module change (whose version is the module's identity, ADR-0203)
  invalidates the loaded module. Rejected.
- **Built into the appliance kernel.** The user's data-path ruling admits
  it, and it never unloads. It lost for three reasons:
  1. Building it in needs the module's source and a Kconfig entry inside the
     kernel source tree, linked into `vmlinux`. That is a modified kernel
     tree, which the #295 charter excludes. ADR-0068 § 4's built-in vsock is
     configuration over unmodified source.
  2. The stock-kernel Tier-3 hosts need the loadable form anyway.
  3. Every forwarder change would become a kernel rebuild.
- **A reference per hooked socket dropped at unhook.** It loses because a
  saved kTLS pointer outlives the unhook: dropping the reference there lets
  the module unload while the saved pointer is still live. Rejected.
- **Unloadable, with an `exit` that detaches every pair first** (the
  spike's `exit`). An `exit` runs only after the count reached zero, so it
  cannot detach a pair that still holds a reference; detaching at `exit`
  instead of counting is what panicked with a saved kTLS pointer. Rejected.

## Consequences

- A module change on a development host is an unload and a load while the
  module is idle: `cargo xtask kmod-load` reloads a stale idle module, and
  refuses a busy one with a message naming the holder class (an open
  context and its process; socket references whose free has not run yet,
  or a context reference not visible in `/proc`, such as a descriptor in
  flight or an io_uring fixed file). No reboot is needed. Any `EBUSY` on a
  live module is reported as busy ("retry"), never as a reboot.
- On the appliance a module change is an image change, which reboots; the
  module is loaded once at boot and nothing unloads it.
- A socket on which some subsystem installed a ULP after attach keeps its
  reference until the machine reboots, and `delete_module` stays refused.
  No Overdrive owner produces that state (ADR-0200); the module logs it.
- Restoring a half's callbacks and removing its wait-queue entry take its
  socket lock (rule 5). The pair's end runs in its work item in process
  context, so taking the lock is allowed there.
- The real-kernel cell: `delete_module` is refused while a context is open
  and while a socket is hooked, succeeds once both are released, and a
  module loaded again then serves.
