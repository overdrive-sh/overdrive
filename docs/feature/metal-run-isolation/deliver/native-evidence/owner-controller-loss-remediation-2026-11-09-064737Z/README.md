# Controller-loss fixture ordering — reproduced RED, targeted GREEN

## Scope and source

This folder preserves complete native `red/` and `green/` foreground captures,
invocation receipts and the exact dirty test-driver diff used by each run.
The `final-driver.diff` records the submitted driver after restoring ordinary
random-seed selection. No production, model, manifest, error/config surface or
other test identity changes were made.

Both diagnostic runs selected the existing seed **0x8c542458** through the
already-supported private `quint_run` seed attribute. The ordinary command
grammar does not accept a QUINT_SEED prefix, and pinned quint-connect 0.1.2
reads that environment key at library compilation with `option_env!`; a runtime
prefix would neither be authorized by the existing grammar nor select this
seed reliably. No new environment key/allowlist exception was added. The
attribute override was temporary and is absent from the final driver, whose
four-sample/24-step randomized coverage remains unchanged.

## Actual RED before the ordering correction

Source HEAD receipt: `73b47241a84db1a3db8faa5c411f61e992001298`; the raw receipt retains all
pre-existing dirty paths plus the diagnostic-only driver edit. The archived
diff shows the unchanged LoseController implementation, temporary seed and
failure-only diagnostic context. Run `022a073ef7c34c364efec25b` returned
**100**; the selected randomized identity failed with the same terminal-poll
message as the original 060408Z capture.

```sh
cargo xtask metal run -- cargo nextest run -p xtask --lib --features integration-tests -E 'test(=metal_run_isolation_conformance::protocol_randomized_owner_traces)' --no-capture
```

The native failure now records the exact action and actual owner state before
cleanup, rather than relying on the earlier offline prediction:

- Action **LoseController**, actor **2**, value 0. The expected model next
  state has actor 2 at phase 5/terminal 75, actor 1 absent and maintenance 3.
- Actual unit `overdrive-metal-run-7b32882826ec503938d59353.service`, MainPID
  **2286825**, was **active/loaded** and retained `waiting_for_lease`.
- The kernel reported an actual READ flock on the fixture maintenance barrier.
  This run's FD 10 resolved the checkout lock but had no FLOCK entry; FD numbers
  were discovered, not prescribed by the test.
- The actual `terminal.v1` read returned NotFound. Actual fixture inputs showed
  barrier present, **checkout absent**, no short-wait/stop/body.pid file.
- The owner argv identifies the real captured `lease-holder.sh --durable`
  entry and its eighteen arguments. The full phase record, descriptor targets
  and fdinfo are retained verbatim in the raw log.

The original seven-second poll expired because the driver had closed stdin
and then awaited terminal while its own external checkout gate still prevented
the owner from reaching the production control read. No premature Collect was
involved. This reproduced state confirms the earlier logged-seed diagnosis;
it is not evidence that the production helper skipped a required transition.

## Minimal fixture correction and preserved obligations

Pre-start loss still closes the real stdin. It then releases only the
fixture's existing barrier/checkout operation inputs before waiting for the
owner's **actual legal admission → EOF-read → terminal → main-end** path.
These inputs are scheduler gates in the test adapter, not production locks:
real flocks, ownership and the production control/error/cleanup code remain
unchanged. After START, loss retains the original detached-command behavior.

The existing model comparison/projection and terminal/stop observations remain
active. The driver additionally requires the exact runner cause
`controller_eof_before_start`, no selected-body PID and byte-identical source
contents for pre-start loss. It does not write records, manufacture a phase,
send START/ABORT, skip an action, filter a trace or increase a timeout. The
seven-second observation poll and 120-second owner budget are unchanged.

Failure-only context now retains the actual step, unit/phase/terminal,
owner argv, discovered fdinfo and separately labeled fixture input files before
TempDir cleanup. This is diagnostic output, not a driver ledger used as state.

## Actual targeted GREEN

Source HEAD receipt: `dd85152c439151a7703475f5b53b7f70b8acfc78` plus the exact archived dirty
corrected-driver diff. Run `b6d20e6b5d764f20c8245f03` returned **0**:

```sh
cargo xtask metal run -- cargo nextest run -p xtask --lib --features integration-tests -E 'test(=metal_run_isolation_conformance::protocol_randomized_owner_traces) | test(=metal_run_isolation_conformance::protocol_controller_eof_before_start) | test(=metal_run_isolation_conformance::protocol_controller_loss_after_start)' --no-capture
```

All **three selected existing identities passed**: the seeded randomized
identity with its unchanged four-trace/24-step configuration, controller EOF
before START, and controller loss after START. The latter retains the actual
255 result after disconnection. No broader native/model-check campaign was
run and this receipt is not relabeled as a new full-suite run.

The temporary seed attribute was then removed, restoring the original random
selection. Final-source formatting and canonical Lima strict Clippy
(`cargo clippy -p xtask --tests --features integration-tests --locked -- -D warnings`,
using the existing writable Cargo caches) passed. No further source behavior
changed after the targeted GREEN, and no fabricated DES phase event or mutation
run was recorded. Original `/tmp` capture bytes are unchanged.
