# Walking skeleton findings — metal-test-placement-attribute

**Verdict: WORKS.** A test declared with
`#[overdrive_testing::overdrive_test(guest = "cgroup-default")]` and no
`test-placement.json` entry runs on an ordinary Metal run: the host half boots
a fresh `cgroup-default` guest, re-runs the same test binary inside it with
`--exact <name>`, and reports the guest result as the host result.

No probe was run. The user chose to promote directly on 2026-11-13 because the
dispatcher (`fwd_guest_suite`) already ran named tests inside a guest with
`--exact`. The skeleton is the first slice of the feature, committed in
`db6fb7589`.

## Substrate

- Host: the native x86_64 Metal box, through `cargo xtask metal run --`
  (ordinary runs).
- Guest kernel: `Linux version 7.0.0-38-generic`, booted by Cloud Hypervisor
  (from the guest console). The host's own `uname -r` was not captured:
  `metal run --exclusive` timed out on the shared maintenance barrier (120 s,
  then 900 s). The guest image is the host's stock kernel image, so this is the
  same 7.0 build, but the host value is unrecorded.

## Migrated test

`overdrive-cli` integration binary:
`integration::cluster_and_node_commands::cluster_status_with_no_server_returns_transport_error`.
Its table entry is deleted. No dispatcher cell named it, so it does not run
twice.

Demo command:

```text
cargo xtask metal run -- cargo nextest run -p overdrive-cli --features integration-tests,kvm-tests --no-capture --status-level all --final-status-level all -E 'test(=integration::cluster_and_node_commands::cluster_status_with_no_server_returns_transport_error)'
```

## Observed results

| Case | Observed |
|---|---|
| Body passes | `PASS [ 4.764s] ... cluster_status_with_no_server_returns_transport_error`; the guest console shows the 7.0.0-38 boot and the body ran inside it |
| Body fails (temporary edit, reverted) | `failed inside the "cgroup-default" guest with exit status 101`; host test FAIL, guest output visible, run exit 100 |
| Unknown group (temporary edit, reverted) | `OVERDRIVE_METAL_TEST_ERROR error=placement_invalid group=cgroup-defualt` |
| Unmarked sibling test, table entry removed (temporary edit, reverted) | `OVERDRIVE_METAL_TEST_ERROR error=test_unclassified`, status 78 |
| Not-yet-migrated dispatcher test | `http_client::cluster_status_..._actionable_message` passes through the dispatcher (6.525 s, 34 other tests running) |
| Dispatcher unit tests, re-keyed to `overdrive-testing` `guest_suite::tests::*` | 9 pass |

These results are as reported by the crafter, who ran them. The raw run logs
were not retained in this directory.

## Per-test guest boot time (one guest per test, `cgroup-default`)

From the `OVERDRIVE_GUEST_KIT_TIMING` lines the shared attempt function prints
on stderr:

| Run | staging_ms | boot_ms | total_ms | nextest wall |
|---|---|---|---|---|
| passing body | 446 | 2398 | 4705 | 4.764 s |
| failing body | 465 | 2416 | 3542 | 3.606 s |

Two samples only. Later runs failed with
`rsync: write failed ... No space left on device (28)` on the Metal box, so no
distribution was taken. Treat these as a point estimate: about 0.45 s staging
and 2.4 s boot per test, before the body runs.

This matches the cost before the change. Under the table, ordinary Metal runs
already boot a fresh guest for each guest-placed test attempt; the attribute
changes how the test reaches its guest, not how many guests boot.

## How the wrapper recognises a declared test

- The macro emits a `<module path>\0<group>\0` record into the ELF section
  `overdrive_test_placement` of the test binary. `#[used]` alone did not
  reliably survive linker section garbage collection, so the generated test
  references the record.
- `test-wrapper.sh` reads that section (python3, ELF section headers). A record
  for the exact test name places it as a guest test with no table lookup.
  Otherwise the existing table lookup runs unchanged, so an unmarked test with
  no entry is still refused `test_unclassified`.
- An unknown group is refused `placement_invalid group=<name>`.
- The wrapper gives a declared test the same `run_local_vsock_namespace`
  isolation a table-placed guest test gets.

## Not demonstrated

- Fail-closed behaviour off Metal (Lima, CI): reasoned from `GuestKit::provision`
  refusing when the kit is unprepared, not run.
- Behaviour when a test has both a marker and a table entry: the marker wins;
  no ambiguity check exists.

## Edge cases and constraints found

- The whole dispatcher moved into `overdrive-testing/src/guest_suite.rs`
  (about 4.9k lines); `bin/fwd_guest_suite.rs` is a 3-line `main`. Both paths
  share `run_nextest_group_attempt`.
- Group names are now listed in three places: the wrapper, the dispatcher plan
  and the dispatcher `main`.
- `inside_guest()` reads `OVERDRIVE_TEST_IN_GUEST=1`, an environment variable.
- Off Metal, the migrated test now fails instead of running its body on the
  host, so any Lima or CI lane that runs the whole `overdrive-cli` integration
  binary will see it fail.
- The xtask placement-parity test needed a narrow change: a reviewed row whose
  table entry is gone is accepted only when the function carries an exact
  `overdrive_test(guest = "<same group>")` declaration. The same test already
  fails on `main` at an unrelated control-plane row
  (`a_repair_of_a_held_listener_keeps_its_socket_and_its_port`).
