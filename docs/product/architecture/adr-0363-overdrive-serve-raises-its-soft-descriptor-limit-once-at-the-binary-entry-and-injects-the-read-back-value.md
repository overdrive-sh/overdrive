# ADR-0363 — `overdrive serve` raises its soft descriptor limit to the hard limit once at the binary entry and injects the read-back value

## Status

**Accepted** (GH #382, DESIGN decision GPD-8; approved by the user 2026-11-25 as ruling A on GPD-1, recorded as its own decision beside [ADR-0362](adr-0362-dns-forwards-in-flight-are-bounded-by-the-descriptor-limit-and-excess-is-shed-at-once.md), which consumes the value).
Interface contract: `docs/feature/guest-public-dns-resolution/feature-delta.md` § *[REF] Interface contracts* (the descriptor limit).

## Context

The soft descriptor limit (`RLIMIT_NOFILE`) is a process-global resource. The guest-flow owner already reads it and refuses boot below its own requirement (`FdBudget`). DNS forwarding adds a second consumer, the in-flight bound of ADR-0362. The raise is useful to the guest-flow owner whether or not DNS forwards.

Two forces. Changing a process-global limit from library code changes it for every in-process test in the same binary. Two consumers that read the limit separately can see different values.

## Decision

- The soft limit is raised to the hard limit once, in the `overdrive serve` binary entry (`crates/overdrive-cli/src/main.rs`) before the server starts, and the resulting soft limit is read back. The read-back value is authoritative.
- The read-back value is one injected input. It is passed to the production host adapter constructor of the guest-flow seam and returned as `GuestHostFacts.nofile_limit`. The guest-flow descriptor probe and the DNS permit count both read it through that one host-facts seam.
- The value does not travel in `ServerConfig`, and the library calls `getrlimit` nowhere. The only source of the value is the binary entry (production) or the test composition (tests).
- A raise the kernel refuses is not fatal: `serve` continues with the current soft limit, read back, and logs it. The guest-flow probe already refuses boot when the limit is too low.
- The raise has no cap: the hard limit the image sets is the bound.

## Alternatives considered

- **Raise inside the server entry (`run_server`).** Mutates a process-global resource from library code, so every in-process test that boots the server changes the limit of the test binary. Rejected.
- **A `ServerConfig.nofile_soft` field.** A test-settable value can contradict the kernel limit and bypass the `FdBudget` refusal; ADR-0219 keeps `ServerConfig` to configuration, with no field that injects a fault or an adapter-level fact. Rejected.
- **A `getrlimit` inside `ServerConfig::new()`.** A hidden process read in a constructor; the value can still disagree with the one the adapter reports. Rejected.
- **Read the limit live at each use.** Two readers can see different values across a change; a limit changed after boot would silently move the bound. Rejected for one value read back after the raise.
- **Cap the raise at a stated ceiling.** A number chosen without a measured reason; the hard limit is already bounded by the kernel's `nr_open`. Rejected.

## Consequences

- Positive: in-process tests never change the process limit; the probe and the permit count cannot disagree; production gets the value only from the read-back at the binary entry; test compositions pass the sim value.
- Negative: the production host adapter constructors take the value, so every composition that builds the host bundle supplies it; tests supply the sim seam's value (feature-delta lists the call sites).
- The raise is a bounded effect on the `serve` process: soft limit to hard limit only, inherited by the non-VMM children `serve` spawns. VMMs are unaffected because the launcher sets their limit explicitly.
