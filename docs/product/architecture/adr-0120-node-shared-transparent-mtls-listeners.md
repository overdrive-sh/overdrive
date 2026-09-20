# ADR-0120 — Own one node-shared leg-F listener and one node-shared leg-C listener

## Status

**Accepted — the current #295 contract is user-approved and independently
approved through D-295-DISTILL-7 at review iteration 9 on 2026-09-17;
D-295-DISTILL-11 task-owner evidence is autonomously authorized and pending the
trusted-checkpoint review.** ADR-0123 owns registration generations;
ADR-0124 owns runtime recovery; ADR-0125 owns destination allowed-port
membership.

## Context

The current worker owns one leg-F and one leg-C transparent listener per
allocation. At the proposed T1 population of 16,384 allocations, that shape
requires exactly 32,768 listener file descriptors and 32,768 idle accept tasks.

Part D proved the structurally different node-shared shape on native metal. One
leg-F listener and one leg-C listener served two real microVM allocations while
preserving production TLS 1.3, kTLS TX/RX, kernel splice, platform-held identity,
per-VM cgroups, allocation-scoped connection teardown, and fail-closed capability
selection. It captured a predecessor capability, retired membership, selected
the successor on a new accept, and rejected stale/post-removal use. It did not
hold a real enforcement call across retirement or execute the late-handle
publish/teardown branch. At T1, the bounded comparison
used two listener file descriptors and two idle accept tasks; listener/task RSS
was 316 KiB versus 29,484 KiB for async per-allocation listeners. The counts are
structural; the RSS and timing observations are single-run evidence.

Cilium at local commit `e99150f8d8f403eca51ed82138d4ae20a265c8f3`
independently separates per-endpoint TCX programs from node-shared,
reference-counted HTTP/TLS listeners. That validates the cardinality split, not
Overdrive's identity or teardown mechanism: Cilium does not supply Overdrive's
allocation-generation/SPIFFE capability or concrete kTLS/splice handle owner.

## Decision

The node listener owner binds exactly one transparent leg-F listener and one
transparent leg-C listener. Their lifetime is node-scoped; stopping an
allocation never closes or replaces either listener.

One module-private registry inside `MtlsInterceptWorker` owns the complete
Pending/Active/Retiring capability lifecycle. It checks generation overflow and
key conflicts before effects, publishes indexes only with full element
ownership, issues RAII exact-generation claims, fences returned-handle
publication against retirement, waits claims without holding the lock, and
retains predecessor reservations through handle/element drain completion.
Listener sockets/tasks, node guard, and unrelated capabilities remain outside
that registry's mutation universe.

Retirement may take ownership while registration is Pending. It retains every
reservation and cannot expose a drain until the Pending owner either activates
atomically or relinquishes all partial element effects. Activation after that
handoff never publishes Active; it transfers its effects to the Retiring record
and wakes the same waiter. Pending cancellation drops partial guards before it
signals completion. No registry lock crosses guard teardown or an await, and
owner shutdown uses the same handshake for every Pending registration.

If activation observes that retirement already owns the generation, it never
reports successful installation. `start_alloc` returns the typed allocation-
identified registration-retired failure while the retirement owner keeps every
guard and reservation through drain completion. The action shim therefore
keeps EXEC closed, stops the driver, joins that same retirement cleanup, tears
down the guest network, and releases the address last; no second cleanup owner
is created.

The existing `MtlsInterceptWorker` is that owner. Its mandatory four-port
constructor remains side-effect free. A boot lifecycle starts the two listeners
and constant rule targets once; one failure-observation future feeds the
control-plane supervisor; exact-port converge/audit performs runtime recovery;
and owner shutdown closes and drains the complete listener tree.
After allocation elements are removed, shutdown retains the constant empty
rules for the accepted listenerless mark/local-route fail-closed state and
next-boot revalidation.

On a fresh process only, boot first proves the EXEC gate is closed and stale
managed-TAP ownership is empty, then identifies the retained owned rules, binds
fresh ephemeral F/C ports, atomically retargets only those owned rules, and
requires complete socket/rule/set read-back before admission. No listener is
adopted or persisted. Runtime recovery remains exact-port rebind with target
rewrite forbidden.

Per-allocation start/stop owns only registration capabilities, shared set
elements, claims, and enforcement handles. Exact signatures and typed error
projection live only in the #295 feature delta. ADR-0076 retains privileged
intercept-port ownership but does not assign listener cardinality or lifetime
per allocation.

The owner maintains two allocation-registration indexes. Leg F resolves the
accepted socket's validated source guest address and recovered original
destination. Leg C resolves the recovered original destination. Each resolution
captures one immutable `(AllocationId, generation, SpiffeId)` capability.
ADR-0123 makes that generation a checked, non-zero node-session counter owned by
this listener owner; it is not part of the guest-network handoff. ADR-0125 makes
destination registration IP-keyed with an allowed set of distinct TCP ports.

Before enforcement, the connection owner claims that exact capability only if
the same generation is active. It never re-resolves or re-attributes an accepted
connection. After enforcement returns an `EnforcedConnection`, the owner
publishes the handle under that capability only if the same generation remains
active; otherwise it tears the new handle down immediately. Unknown source,
unknown destination, inactive generation, and post-removal connection attempts
fail closed before enforcement.

Each allocation owns and drains only the enforced handles published under its
exact capability. Allocation stop removes its source/destination registrations
and retires its generation before draining those handles. Address reuse is
remove-before-reassign: the predecessor registration and active generation are
gone before a successor receives the address. The node-shared listeners and
unrelated capabilities remain live. Boot performs the existing VMM reclamation
first and starts with no adopted allocation capabilities; later registrations
follow the normal allocation lifecycle.

ADR-0124 supervises both listeners at runtime: task/socket failure immediately
closes new command release, receives a five-second bounded reconvergence window,
and fail-stops the serve process if recovery does not complete.

The worker retains both listener tasks through one module-private two-slot
Tokio owner. Each listener join is awaited by an owned observer whose drop
aborts rather than detaches the child; the owner keeps only a weak event sender
plus the real receiver. Normal return, I/O error, panic and cancellation are
classified from actual `JoinHandle` results, while unexpected loss of both
observer senders is the channel-closed error. Recovery replaces only a slot
already removed by terminal observation, and intentional owner shutdown
consumes and joins the owner without reclassifying its cancellation. Source-
local tests spawn real Tokio tasks and drive their actual return/error/panic/
abort/channel-close causes through this owner. No public task-control hook or
constructed consequence is part of the contract; exact private signatures
remain single-sourced in the #295 feature delta.

## Alternatives considered

### Async listener pair per allocation

Rejected. It removes the blocking-pool ceiling but retains `2N` listening file
descriptors and `2N` idle accept tasks. Part D proved that this cardinality is
unnecessary and measured 32,768 of each at T1 versus two of each for the shared
shape.

### Shared listeners without immutable generation capability and publish fence

Rejected. Looking up the allocation only after enforcement, or re-looking it up
after address reuse, can attribute a predecessor socket or enforcement handle to
the successor. Part D proved immutable pre-enforcement capture and successor
selection; the post-enforcement retirement race remains a required focused
production-path evidence gate.

### `SO_REUSEPORT` listener pair per allocation

Rejected. It preserves per-allocation socket/task cardinality and makes kernel
socket selection, rather than the explicit allocation registry, part of the
identity boundary.

## Consequences

Positive: listener and idle-task cardinality is exactly two per node instead of
`2N`; connection file descriptors remain workload-dependent and are not
misreported as listener file descriptors. Allocation stop is isolated while the
node accept surface remains available.

Negative: the listener owner gains security-critical source/destination
registration indexes, active-generation claims, a post-enforcement publish
fence, and allocation-scoped handle ownership. These mechanisms require the
real-socket negative, address-reuse, retirement-during-enforcement,
teardown-not-reattribute, in-flight-stop wait, owner-shutdown, and isolated-stop
probe contract recorded in the feature delta. Part D proves listener
cardinality, selection, stale rejection, and already-published scoped drain; it
does not prove the retirement/publish race, T1 throughput, flood behavior,
long-run performance, or the production implementation.
