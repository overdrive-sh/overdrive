# Adversarial review — step 02-07

## Iteration 1 metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Step | `02-07` — Bound Stop terminal contention |
| Reviewed commit | `56f3474ba7066db9ded19206104df9c818c1ecc5` |
| Parent | `5362af2a728910aa4529872a22d7e2f8611ff10d` |
| Subject | `fix(action-shim): bound terminal LWW contention` |
| Trailer | `Step-Id: 02-07` |
| Review iteration | 1 |
| Verdict | **APPROVED** |

## Scope and design sources

This review covers only the three files in the reviewed commit: the private
`StopAllocation` arm, its existing acceptance-test module, and the DES log.
The current worktree's unrelated `AGENTS.md` edit was preserved. No public API,
trait, type, error variant, schema, persistence surface, production test seam,
example, expectation, or mutation configuration was added.

The reviewed contract is:

| Source | Reviewed requirement |
|---|---|
| `feature-delta.md:1536-1576` | After cleanup, Stop makes no more than two fresh-read compound proposals; the first LWW loss rebases once, the second preserves the winner and finishes successfully without an event. |
| `docs/product/architecture/brief.md:10326-10341` | The concrete production race is Stop versus the exit observer; the two-proposal bound has no cancellation/replay/outbox extension. |
| `distill/test-scenarios.md:65-104` | S-GTI-BTR-01 requires exactly two fresh proposals, one supervision release, route removal, no broadcast, and preservation of the competing durable row. |
| `deliver/roadmap.json:1038-1058` | First acceptance, one-rejection acceptance, exact-terminal no-op, typed read/write failures, and atomicity remain in scope. |

## Production-path and behavior audit

The production path is reachable: the convergence loop drives
`run_convergence_tick` (`crates/overdrive-control-plane/src/lib.rs:3370-3380`),
which dispatches reconciler actions through the action shim
(`reconciler_runtime.rs:1633-1747`). For a Stop, the shim calls
`Driver::stop`, which sets the intentional-stop flag before signalling and
awaits the watcher (`overdrive-worker/src/driver.rs:660-725`). The observer can
then author the competing row through the same atomic lifecycle writer
(`worker/exit_observer.rs:458-549`) while the Stop arm completes its cleanup.
This is the specific, production-reachable race the design narrows.

The implementation at `action_shim/mod.rs:2567-2714` conforms to that contract:

- It retains the existing initial existence check and cleanup order, then takes
  a current-row read after cleanup for each of a fixed `for _ in 0..2` loop.
- `Ok(None)` on the first proposal reaches exactly the second fresh read; a
  second `Ok(None)` exits the loop without a state/occurrence write.
- A newly read exact requested terminal exits with no proposal or broadcast.
- An accepted first or second proposal retains the established release,
  route-removal, and best-effort broadcast tail.
- Read and compound-write errors retain the existing `ShimError` propagation;
  the latter releases supervision before returning, and neither error route
  removes the process-local driver route. The atomic port contract states that
  `Ok(None)` changes neither current state nor occurrence history
  (`overdrive-core/src/traits/observation_store.rs:2062-2076`).

No cancellation finding is warranted: graceful shutdown cancels and joins the
convergence task, waiting for any active dispatch (`lib.rs:1396-1417`), and the
loop observes cancellation only between drained batches (`lib.rs:3347-3396`),
which is the explicitly accepted design boundary.

## Test and Contract Shape audit

`stop_allocation_second_lww_rejection_completes_without_event`
(`action_shim_crash_observability.rs:1900-1927`) replaced the mapped RED
scaffold and carries the exact `/// CONTRACT_SHAPE: bounded-change.`
declaration. It drives the real public `action_shim::dispatch` composition with
only a test-owned `ObservationStore` decorator; no production accessor or
test-only production wiring was introduced.

The decorator forces the exit-observer-shaped winner before each of the first
two proposals, counts proposals, and panics if a third reaches the port
boundary (`:703-753`). The test case table verifies the following total
partitions (`:990-1242`):

| Partition | Oracle |
|---|---|
| First acceptance | One proposal, occurrence/event, release, and route removal. |
| First loss, second acceptance | Two proposals and a rebased accepted terminal row. |
| Two losses | Exactly two proposals, no fabricated bus event, one release, absent route, and the competing terminal row remains current. |
| Current row already exact | No proposal/event, one release, and route removal. |
| Read/write error | Existing typed observation error, no partial current/occurrence commit, and retained driver route. |

The one-loss partition is also a freshness oracle: the decorator's competing
row can be accepted only when the second proposal is derived from the new
winner and has a dominating timestamp. The two-loss partition rejects a third
proposal at the store boundary. This is a falsifiable production-entry test,
not fixture theater.

## Verification and process evidence

| Check | Result |
|---|---|
| Focused Linux/Lima test: `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane -E 'test(stop_allocation_second_lww_rejection_completes_without_event)'` | PASS — 1 passed, 565 skipped; nextest run `a175822a-2cee-4dbb-be20-bb97c96a5e9f`. |
| `cargo xtask lima run -- cargo fmt --all -- --check` | PASS. |
| `cargo xtask lima run -- cargo clippy -p overdrive-control-plane --test acceptance -- -D warnings` | PASS. |
| Parent-to-target `git show --check` and worktree `git diff --check` | PASS. |
| DES log JSON parse | PASS. `02-07` records `RED FAIL` (17:37:13Z), `GREEN PASS` (17:44:22Z), then `COMMIT PASS` (17:44:44Z). |
| Mutation discipline | PASS — no per-step mutation run or exclusion change. |

The direct macOS `cargo test` attempt cannot compile this Linux-oriented
control-plane dependency graph because `netlink-sys` requires Linux netlink
constants; the required Lima execution above is the authoritative result.

## Findings and disposition

No findings. The bounded implementation matches the accepted Stop-only
contract, preserves the established typed-error and atomicity behavior, and
the mapped test honestly proves the finite LWW-contention outcomes.

## Final verdict

**APPROVED.** Step `02-07` may advance to its next roadmap step.
