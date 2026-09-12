# VM lifecycle latency — refactoring quality metrics

## Measurement method

Metrics were collected over the twelve feature-touched production Rust files
listed below with `tokei`. The before snapshot was the pre-refactor `HEAD`;
the after snapshot was taken after the complete L1–L6 batch. Structural smell
counters were collected with `rg` over the affected production regions. No
tests were edited or used as a refactoring metric.

## Before and after

| Metric | Before | After | Interpretation |
|---|---:|---:|---|
| Production Rust files measured | 12 | 12 | Scope unchanged |
| Total Rust lines | 19,017 | 19,010 | Seven fewer lines after removing duplicated control flow |
| Rust code lines | 13,942 | 13,935 | Seven fewer code lines |
| Rust comment lines | 3,756 | 3,754 | Two fewer stale/redundant comment lines |
| Rust blank lines | 1,319 | 1,321 | Formatting/layout only; not claimed as an improvement |
| `begin_group_termination(` occurrences in PID 1 source | 6 | 4 | Parsed control-line paths share one private handler; EOF and the direct-status path remain explicit |
| Nested admission guards in convergence owner | 2 | 1 | Capacity and admission predicates are expressed at one level |
| Duplicated originating-session `Weak::ptr_eq` predicates | 2 | 1 | `ClaimGuard` owns one private predicate used by both paths |
| Feature production diff versus `origin/main` | 1,360 additions / 369 deletions | 1,389 additions / 406 deletions | The after value includes this behavior-preserving refactor batch; overlapping line edits are counted by `git diff` |
| Refactor batch delta versus pre-refactor `HEAD` | — | 91 additions / 98 deletions | Six production files; no tests or unrelated production files |

The line-count decrease is modest because the feature's behavior and exact
ownership contracts are intentionally preserved. No claim is made for
maintainability index, cyclomatic complexity, coverage, or mutation score: no
comparable analyzer was part of the bounded refactor measurement, tests were
immutable, and mutation testing is a final DELIVER gate rather than a
per-step/refactor operation.

## Commands used for metrics

Before batch:

```text
tokei <the twelve feature-touched production Rust files>
Rust: 12 files, 19,017 lines, 13,942 code, 3,756 comments, 1,319 blanks
begin_group_termination calls: 6
nested convergence admission guards: 2
originating-live predicate copies: 2
```

After batch:

```text
tokei <the same twelve feature-touched production Rust files>
Rust: 12 files, 19,010 lines, 13,935 code, 3,754 comments, 1,321 blanks
begin_group_termination calls: 4
nested convergence admission guards: 1
originating-live predicate copies: 1
```

`git diff --check` reported no whitespace errors for the refactor batch before
the terminating verification sequence.

## Final metal diagnosis

The required bounded 16-test CLI gate is not green: the 14th ordered case,
`restarted_vm_boots_from_a_clean_unmodified_rootfs_copy`, timed out at 120.008
s while the other 15 cases passed. The focused current-tree case and the
minimized ordered pair were flaky; the clean `79ae96d2` comparison pair passed.
The terminating current-tree strace captured a concrete same-process fixture
race rather than a quality regression: boot #2 reclaimed the old
`alloc-vm-clean-rootfs-0` VMM, then the old boot #1 reaper removed the newly
created fixed run directory before its kernel copy was written (`ENOENT`). The
test consequently waited in `poll_until_restarted` with no second VMM. No
production metric is claimed to improve for this race, and no unapproved
production synchronization or timeout was added.
