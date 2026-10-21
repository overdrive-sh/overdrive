# ADR-0204 — Neither a late forwarder callback nor a control call by id touches an ended pair's state

## Status

**Accepted — EVMOD-SAFE, approved by the user 2026-10-17; rule 5 follows the
user's ruling MODULE-REFCOUNT of 2026-10-20.** Not implemented. GH #295.
ADR-0198 keeps the module's text alive while anything can still call it
(EVMOD-LIFE, by module reference counting); this ADR keeps a pair's freed
data from being touched (rules 1 to 4) and fixes when a hooked socket's
module reference may be dropped (rule 5). Follows from ADR-0197 (accepted
2026-10-16). The contract is in the #295 feature delta, § *[REF] vsock data
path — the in-kernel event-driven forwarder (EVMOD)*, "Pair end", "Stale
invocations".

## Context

- The forwarder installs its own `sk_data_ready`, `sk_state_change` and
  `sk_error_report` on each half and restores the saved originals when the
  pair ends; it takes write-space wakeups from an entry on each half's
  socket wait queue (rule 5 says why `sk_write_space` is not overridden).
  The kernel reads a socket's callback pointer at each call site without a
  lock, so a CPU may enter an override after the pair's end has removed it,
  and an override already running may still be running when the pair's
  state would be freed.
- `sock_wfree` calls `sk_write_space` for a socket without
  `SOCK_USE_WRITE_QUEUE` (vsock, UDP; TCP sets it, `net/ipv4/tcp.c:477 @
  v7.0`) with no socket lock, outside any RCU section and with bottom halves
  enabled (`net/core/sock.c:2675-2693 @ v7.0`), from preemptible process
  context: the host's vhost worker and the guest's virtio-vsock transmit
  work free every vsock transmit skb (`drivers/vhost/vsock.c:252`,
  `net/vmw_vsock/virtio_transport.c:361`,
  `net/vmw_vsock/virtio_transport_common.c:475-491`; destructor set at
  `:288-289`), and a NIC driver may free a UDP transmit skb there
  (`net/core/dev.c:3464-3470`, `drivers/net/virtio_net.c:6304-6315`,
  `:6336-6345`, `:7097 @ v7.0`).
- A wait-queue entry's function runs only with the wait-queue head's lock
  held and interrupts disabled (`kernel/sched/wait.c:104-108`, `:118-127 @
  v7.0`), and `remove_wait_queue` takes the same lock (`:69-76`).
- The vsock receive path calls the callbacks in process context with the
  socket lock held (`net/vmw_vsock/virtio_transport_common.c:1661`, `:1693`,
  `:1705 @ v7.0`); the vhost device release's orphan sweep calls
  `sk_error_report` without the socket lock, under `vsock_table_lock`
  (`drivers/vhost/vsock.c:725-750`, `net/vmw_vsock/af_vsock.c:458-471 @
  v7.0`). No callback sleeps.
- `rcu_dereference_sk_user_data` reads `sk_user_data` with `rcu_dereference`
  (`include/net/sock.h:682`, `:691-692 @ v7.0`).
  `rcu_assign_pointer(p, NULL)` with a constant `NULL` compiles to a plain
  `WRITE_ONCE` with no release barrier (`include/linux/rcupdate.h:575-576 @
  v7.0`), so it does not order earlier stores before the clear.
- An override that finds no pair must still deliver the event to the
  socket's reader. Calling "the socket's current callback" is safe only if
  that callback is no longer the override; otherwise the override calls
  itself.
- The control interface reaches a pair or watch by its id, not through a
  socket: `OVD_FWD_ABORT` asks for the abort, which a pair's own work item
  carries out, and `OVD_FWD_PAIR_STATE` reads a pair's counters (ADR-0199).
  `cancel_work_sync` guarantees that a work item is neither pending nor
  running on return only "as long as there aren't racing enqueues"
  (`kernel/workqueue.c:4481-4484 @ v7.0`), so every path that can queue a
  pair's work item must be closed before the free waits for it.

## Decision

The module keeps five rules. Rules 1 to 4 make a late callback and a
control call by id harmless to a pair's state; rule 5 makes the module's
unload harmless to a late callback:

1. **An override finds its pair only through `sk_user_data`, read with
   acquire semantics** (`smp_load_acquire`) inside `rcu_read_lock()`. A
   pair found is processed (a spurious run is harmless); no other path
   from a socket to a pair exists. The only path to a pair that does not
   start at a socket is the context's id table (rule 4). The one exception
   from a socket is the half's write-space wait-queue entry, embedded in the
   pair: its function runs only under the wait-queue head's lock, and the
   pair's end removes it under that lock before the free work is queued, so
   no entry function holds the pair once the removal has returned.
2. **A pair's end removes its overrides in one order, on each half, under
   `sk_callback_lock` for writing** (after removing the half's write-space
   wait-queue entry under the socket lock, rule 5): first each callback pointer is set back
   to its saved original (`WRITE_ONCE`), then `sk_user_data` is cleared with
   a release store (`smp_store_release`). An override that reads `NULL`
   therefore sees the restored originals, and calls the socket's current
   callback of the same kind (`READ_ONCE`), which is the original — never
   itself.
3. **A pair's state is freed in this order: an RCU grace period that
   follows the removal, then the completion of the pair's work item, then
   the free.** A pair's end runs inside the pair's own work item, which
   cannot wait for or cancel itself, so the end does not free: it marks the
   pair ended, removes its overrides (rule 2), removes its id from the
   context's table (rule 4), and as its last step queues the pair's free
   work with `queue_rcu_work`, which runs only after a full grace period
   (`kernel/workqueue.c:2639-2664 @ v7.0`); it touches the pair no further.
   After the grace period no override can still hold the pair (rule 1 reads
   it inside `rcu_read_lock()`), no wait-queue entry function can (its entry
   was removed before), and since the id left the table before the
   free work was queued no control call can reach it (rule 4), so nothing
   can queue its work item again. The free work then calls
   `cancel_work_sync` on the pair's work item, which with no racing enqueue
   returns only once that item is neither pending nor running
   (`:4477-4498`) — the ending run and any run an override or a control
   call queued earlier, each of which finds the pair ended and does nothing
   — and only then frees the state. The pair's end record is not part of
   what is freed while it is queued; it is freed when the owner reads it
   (ADR-0202).
4. **A control call reaches a pair or watch by id only under the context's
   record-queue lock.** The context's id table maps each live id to its
   pair or watch and is guarded by the same spinlock as the record queue.
   `OVD_FWD_ABORT`, `OVD_FWD_ABORT_ALL` and `OVD_FWD_PAIR_STATE` look the id
   up, and act on what they find — queue the pair's work item with the
   abort requested, or copy its counters — before they release the lock;
   after it they never dereference the pair. A pair's end, which is always
   its own work item's (a watch's end belongs to the path that removes its
   id, below), removes the id and queues the end record in one section under
   that lock, before it queues the free work. So a control call either finds the id and finishes with
   the pair before the end's section, or finds no id (`ENOENT`, the record
   is queued or was read). An abort that must return after its record waits
   on the context's wait queue and re-checks by id under the lock, never
   through the pair. Ids are never reused within a context (ADR-0202), so a
   stale id never names a later pair. A context's release runs only when no
   control call is in flight on its file, and frees the context only after
   the last of its ends has left that section.
5. **A hooked socket's module reference is dropped only when no module text
   can be running or entered on its behalf** (MODULE-REFCOUNT, ADR-0198) —
   the whole text of an override, prologue and epilogue included. The module
   is entered from a socket in two ways:
   - **The overrides** (`sk_data_ready`, `sk_state_change`,
     `sk_error_report`). Each is invoked by a caller of one of three
     classes. **lock:** it owns the socket lock (the vsock receive path, TCP
     backlog processing in `release_sock`). **rcu:** it runs inside a region
     a vanilla RCU grace period waits for — an `rcu_read_lock` section,
     preemptible or not, or a region with bottom halves, preemption or
     interrupts disabled (`include/linux/rcupdate.h:671-675`, `:687-691 @
     v7.0`): TCP input with the socket not owned
     (`net/ipv4/tcp_ipv4.c:2318-2321`), the orphan sweep under
     `spin_lock_bh`, UDP receive under `rcu_read_lock`
     (`net/ipv4/ip_input.c:240-242`), ICMP-error delivery in softirq.
     **work:** the forwarder's own work item. So rule 2's removal is done
     with each half's socket lock held (`lock_sock`, then
     `sk_callback_lock` for writing, in process context); a **lock** caller
     that loaded the old pointer has returned before the removal, and none
     can load it after. The free work runs after a vanilla RCU grace
     period, which waits for every **rcu** caller, and after
     `cancel_work_sync` (rule 3), which waits for the **work** caller.
   - **The write-space wait-queue entry** (pairs only). `sk_write_space` is
     never overridden, because its fourth caller class, **wfree** —
     `sock_wfree` in preemptible process context with no lock and no RCU
     section (Context) — is waited for by neither mechanism above. The
     socket's own `sk_write_space` wakes the socket's wait queue instead;
     the entry's function runs only under the wait-queue head's lock with
     interrupts disabled, and the pair's end removes the entry with
     `remove_wait_queue` under the socket lock, before the free work is
     queued. Once that returns, no entry function is running or can be
     entered, whoever the waker is.

   The free work runs on the module's own workqueue and calls `module_put`
   as its last statement, once for each hooked socket of the pair or watch.
   Two cases differ:
   - If a **TCP** half (`sk_family` `AF_INET` / `AF_INET6`, `sk_protocol`
     `IPPROTO_TCP`) has an upper-layer protocol installed when its
     overrides are removed (`inet_csk_has_ulp`,
     `include/net/inet_connection_sock.h:347-350 @ v7.0`), the ULP may have
     saved a copy of an override. That half's reference is then never
     dropped; the module logs the socket, and `delete_module` stays refused
     until the machine reboots. The check is made on TCP halves only: the
     helper reads `inet_connection_sock` fields that a vsock or UDP socket
     does not have, and neither can carry a ULP.
   - `module_exit`'s `destroy_workqueue` waits for the free work's return
     after its `module_put` (`kernel/workqueue.c:5979 @ v7.0`).

   **Excluded by precondition.** `sk_error_report` is also called with no
   lock from process context when an error-queue completion is delivered:
   a `MSG_ZEROCOPY` completion (`net/core/skbuff.c:1868-1878 @ v7.0`) or a
   transmit timestamp through `sock_queue_err_skb` (`:5468-5485`). Neither
   reaches a hooked socket: the forwarder sends without `MSG_ZEROCOPY` and
   without a timestamping control message, a watched socket's holder sends
   the same way, and no owner sets `SO_ZEROCOPY` or transmit timestamping
   on a socket it attaches or watches. Attach and watch refuse a half that
   already has `SOCK_ZEROCOPY` set or a transmit-timestamping bit
   (`SOF_TIMESTAMPING_TX_RECORD_MASK`) in `sk_tsflags`, under the existing
   unsupported-socket refusal (`include/net/sock.h:1005`,
   `include/uapi/linux/net_tstamp.h:59-63 @ v7.0`). The refusal is partial,
   since an owner can set either option after the attach, so the owner
   obligation stands beside it.

A watch follows the same four rules with its own state in place of a
pair's. Its end belongs to whichever path removes its id under the
record-queue lock — its override on the guest's classified end, an owner's
abort, or the release — and that path queues the watch's record (none at
release) in the same section. The watch's overrides are then removed, by
that path in process context or, when the path is an override, by the
watch's work item, and its free work is queued only after that. A path that
finds the id already removed does nothing more to the watch.

The second rule's "the current callback is the original" holds only while
no other subsystem saved a copy of an override, which ADR-0200 guarantees
(no socket with an upper-layer protocol is attached, and no owner installs
one on an attached socket).

## Alternatives considered

- **Clear `sk_user_data` with `rcu_assign_pointer` and read it with
  `rcu_dereference`.** For a constant `NULL` the store has no release
  barrier, and a dependency-ordered load does not order the later read of
  the callback pointer; a reader could see `NULL` and the override still
  installed and call itself. Rejected.
- **An override that finds no pair returns without calling anything.** The
  event (a wakeup, a state change) would be lost to the socket's reader in
  the window between removal and the kernel's next read of the pointer.
  Rejected.
- **A per-pair reference count taken by every override.** The count lives in
  the state it protects; taking it after finding the pair still needs the
  same grace period before the free. Adds an atomic per callback and changes
  nothing. Rejected.
- **A control call that looks an id up under RCU and queues the work after
  leaving the lock.** The queue can land after the end's section and after
  the free work is queued, which is the racing enqueue `cancel_work_sync`
  does not cover: the work item could then run on freed state. Rejected.
- **Rule 5 without the socket lock at removal** (drop the module reference
  after the grace period only). A process-context caller that loaded the
  old pointer while owning the socket lock is in no RCU read-side section,
  so the grace period does not wait for it. It could enter the override
  after the module's text is freed. Rejected.
- **Keep the `sk_write_space` override and call `synchronize_rcu_tasks()`
  in the free work before `module_put`.** An RCU Tasks grace period waits
  for a task preempted in module text, which is how ftrace and BPF
  trampolines free text. `CONFIG_TASKS_RCU` defaults to
  `NEED_TASKS_RCU && PREEMPTION` (`kernel/rcu/Kconfig:101-107 @ v7.0`);
  without it `synchronize_rcu_tasks` is `synchronize_rcu`
  (`include/linux/rcupdate.h:182-185`). On a kernel without `PREEMPTION`
  that still waits for the **wfree** caller, because a task running the
  non-sleeping override cannot be preempted in it. The gap is a
  `PREEMPTION` kernel with no option that selects `NEED_TASKS_RCU`: there
  the call is a vanilla grace period, which does not wait for a preempted
  **wfree** caller. The design would therefore depend on the kernel
  configuration, and each pair end would block a worker for an RCU Tasks
  grace period, to defend an entry the wait-queue entry removes. Rejected
  for that configuration dependency and that slow wait.
- **Keep the override and run its body inside `rcu_read_lock`, or take a
  module reference inside it.** The instructions before `rcu_read_lock` or
  the reference, and after the matching unlock or put, are themselves
  module text that a preempted **wfree** caller can be executing. Neither
  closes the window. Rejected.
- **Override `sk_write_space` on TCP halves only and use the wait-queue
  entry on vsock and UDP halves.** Correct, since TCP sets
  `SOCK_USE_WRITE_QUEUE`, but it keeps two mechanisms for one trigger and a
  per-family branch in attach and in the removal. Rejected for the one
  mechanism.
- **A reference taken by each control call, held while it waits for the
  pair's end.** It keeps the pair alive for an abort that only needs to know
  the id is gone; the id check under the lock gives that with no count.
  Rejected.

## Consequences

- The rules are internal to the C module; no port or wire format carries
  them. The DELIVER review of the module step checks the following against
  the source: the order of rule 2, the free order of rule 3, the id-table
  locking of rule 4, and for rule 5: the socket lock at removal; no
  assignment to `sk_write_space` anywhere in the module, and the wait-queue
  entry removed before the free work is queued; `module_put` at the end of
  the free work; the ULP check guarded by `sk_family` `AF_INET` /
  `AF_INET6` and `sk_protocol` `IPPROTO_TCP`; no `MSG_ZEROCOPY` or
  timestamping send in the module; the attach- and watch-time refusal of a
  half with `SOCK_ZEROCOPY` or a transmit-timestamping bit; and no
  `SO_ZEROCOPY` or transmit-timestamping option set by an owner on a socket
  it attaches or watches, before or after the attach.
- The module's Tier-3 unload cell checks rule 5's effect. `delete_module` is
  refused while a context is open and while a socket is hooked, and stays
  refused for a socket that has a ULP installed after attach. It succeeds
  once every context is closed and every free has run, and the module loaded
  again then serves.
- A Tier-3 stress of the module step ends pairs while their sockets receive
  at line rate on every CPU and while the owner issues `OVD_FWD_ABORT` and
  `OVD_FWD_PAIR_STATE` on ids that are ending, at least 10^5 pair ends on
  the stock 7.0 kernel, and requires no kernel oops or recursion report
  (V-EV-SAFE; not
  contract-shaping: a failure is a module defect, fixed in the module). The
  same suite shows a stalled direction resuming through the wait-queue entry
  for a TCP, a vsock and a UDP target; the earlier override's measured
  resumption is not evidence for the entry.
- A decision that allows a ULP on a forwarded socket reopens rules 2 and 5.
