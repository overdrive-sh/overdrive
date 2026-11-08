# Ordinary concurrency native attempt 3 — driver failed after partial proof

This stable evidence copy preserves the completed capture from
`/tmp/metal-native-retry-20261108T214703Z`. The root orchestrator ran the
existing `infra/metal/tests/worktree_journey.sh` against the current
`ouagadougou` checkout and the prepared peer worktree. The run used ordinary
admission for both invocations. The capture is **not a complete K1–K3 proof**:
the overlap and first survivor-boundary assertions passed, then the driver
exited at its repeat-collection stderr comparison. The original capture under
`/tmp` remains untouched.

## Source and substrate attribution

The driver ran against target `ubuntu@151.115.99.251`, as recorded in both
bootstrap transcripts. Its command was the existing explicit core guest suite:

```text
env OVERDRIVE_FWD_GUEST_GROUP=core cargo run -p overdrive-testing --bin overdrive-fwd-guest-suite --features integration-tests
```

The captured driver streams show the input checkout paths, commands, run IDs,
and remote run records. The invocation shell did not persist a separate copy of
its complete environment, so this artifact does not assert the configured
kernel/rootfs paths. The guest console in the collected output reports Linux
`7.0.0-38-generic`. `before.json` preserves the physical host baseline returned
by the existing read-only observer: BPF programs, module sizes, nftables,
routes, `test_cgroups`, and the initial vsock namespace modes. The capture has
no `after.json`, so it provides no final physical-kernel delta comparison.

| Input | Source marker commit | Source digest | Checkout key | Captured status |
|---|---|---|---|---|
| `/Users/marcus/conductor/workspaces/helios/ouagadougou` | `7e5602e6de3ce8a96f3c93d89aa735f87f722233` | `65f9bd9cbd2c4fd014c710ae8e453f162edea9cf175550baa5d5a06c4f8e6ab0` | `e3d831d7dafe621226ae6047ebcac3536f37470cafcf5dbb64eb9598886af410` | See `first-source.status`; preserved service-feature and `AGENTS.md` changes were present. |
| `/private/tmp/metal-run-isolation-peer.UKMMtE6B` | `2ba2232208ce9a5e91666f61b6b9845d9797c62a` | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` | `052a646313948d416e08f7afb85c82411ecddc98659ed0825aa09bcde3be0f39` | `second-source.status` was empty before the run. |

The physical baseline and every observer sample captured before the driver
exit are retained as separate JSON files. No host kernel release was emitted
by this observer capture.

## Native observations that passed

The registered ordinary runs were `3e5e8389f4a0bce6bf593ac4` and
`a708cb766a23a4e2c982cfbf`; both controller streams contain their
`OVERDRIVE_METAL_RUN_STARTED` marker.

`overlap.json` exists only after the driver's full overlap predicate passed.
It records both distinct checkouts active in `phase=running`, ordinary
execution class, a shared-barrier read lock held by each durable owner,
distinct checkout locks, matching source markers, and the designed root
ownership and modes. It also records a live Cloud Hypervisor process holding
`/dev/vhost-vsock` for each run:

| Run | VMM PID | VMM network namespace | Existing CID |
|---|---:|---|---:|
| `3e5e8389f4a0bce6bf593ac4` | `1806559` | `net:[4026532936]` | `16785416` |
| `a708cb766a23a4e2c982cfbf` | `1806818` | `net:[4026532989]` | `16785416` |

The failed assertions printed while polling for this sample were intermediate
observations: the driver retries them. The saved successful `overlap.json`
proves that both live VMMs were eventually observed with the same CID in
disjoint network namespaces.

`survivor-boundary.json` also exists only after all of its assertions passed.
It records run `3e5e…` stopped with command exit code `0`, a complete receipt,
no remaining processes, and all three owned roots absent. Run `a708…` was still
active in that sample; its VMM PID, network namespace, roots, checkout record,
and source marker matched the overlap snapshot.

The first command's collected stdout and repeat stdout are byte-identical and
contain `test result: ok. 86 passed; 0 failed`. Its collector stderr contains
`OVERDRIVE_METAL_RUN_COMPLETE run_id=3e5e8389f4a0bce6bf593ac4 exit_code=0
cause=command_returned` on both collections.

## Driver failure and later read-only collection

The fatal failure is at `infra/metal/tests/worktree_journey.sh:116`, where the
driver byte-compares `first.collect.stderr` with `first.repeat.stderr`. Both
files contain the same collector marker, but the `cargo xtask metal collect`
wrapper printed a nondeterministic Cargo duration before it:

```text
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.22s
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.23s
```

That changes byte 70 on line 1. The stdout comparison at line 115 passed. The
driver then exited under `set -e`; it did not write `first-complete.json`, wait
for and collect the second controller, or write `after.json`. As a result, this
capture does not establish the second run's result at driver exit or the final
physical-kernel, process, root, and survivor complements.

After the driver had exited, a separate canonical read-only
`cargo xtask metal collect a708cb766a23a4e2c982cfbf` was run. Its exact
stdout/stderr and process status are retained in `post-driver-collect.*`. It
returned process status `0`; the collector reported
`OVERDRIVE_METAL_RUN_COMPLETE run_id=a708cb766a23a4e2c982cfbf exit_code=0
cause=command_returned`, and the guest output reports `86 passed; 0 failed`.
This later read confirms the second command's terminal result and that its
completion receipt passed the collector's validation. It cannot fill in the
missing `after.json` observation or turn the failed driver invocation into a
complete K1–K3 proof.

The original raw capture files remain unedited beside this note, including the
early polling failures and the differing collection stderr files. No
admission or placement driver was run in this attempt, and no DES phase event
was recorded for this root-executed native run.
