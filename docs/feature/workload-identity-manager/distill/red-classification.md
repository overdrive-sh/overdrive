# RED Classification — workload-identity-manager

**Wave**: DISTILL  
**Status**: scaffold pending

The Rust scaffolds intentionally use:

```rust
#[should_panic(expected = "RED scaffold")]
```

This matches the existing workspace convention for DISTILL pending acceptance
tests: the files compile and are wired into Cargo, but the executable assertions
are not active until DELIVER replaces the `RED scaffold` bodies slice by slice.

| Scenario group | Scaffold status | DELIVER RED expectation |
|---|---|---|
| `SvidLifecycle` pure reconciliation | Pending scaffold | After unskipping, fails because `SvidLifecycle` / actions do not exist yet. |
| Action-shim `IssueSvid` | Pending scaffold | After unskipping, fails because dispatch arms / `IdentityMgr` hold path do not exist yet. |
| `IdentityRead` read contract | Pending scaffold | After unskipping, fails because the port trait / impl / sim double do not exist yet. |
| DST running-set invariant | Pending scaffold | After unskipping, fails because the invariant and held-set projection do not exist yet. |
| Integration walking skeleton / restart recovery | Pending scaffold | After unskipping, fails because production composition has no workload identity manager yet. |

Fail-for-right-reason gate for DELIVER: replace one scaffold at a time with a
real assertion, run that scenario, and confirm the failure is missing
functionality rather than import/setup failure before writing production code.
