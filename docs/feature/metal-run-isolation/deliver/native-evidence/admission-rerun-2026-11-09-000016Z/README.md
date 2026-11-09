# Admission rerun — timeout body-start marker is not in the controller stream

This stable copy preserves the completed root capture from
`/tmp/metal-native-remaining-20261109T000016Z-admission`. It contains the raw
ordinary controller stdout/stderr and driver stderr. The driver exited before
it captured an `ordinary-running.json` owner observation, so this attempt does
not establish admission or exclusive-waiter behavior.

## Captured execution

Run ID: `10d61cd38baa759626482c31`. The ordinary command reached START and
Nextest started the selected fixture case
`metal-run-isolation-attempt-fixture::attempt::selected_attempt_waits_for_its_timeout`.
After 10.002 seconds, Nextest reported the selected case `TIMEOUT`; the
durable command retained aggregate exit 100 with `cause=command_returned`.
The expected `METAL-ISOLATION-TIMEOUT body_started=true` marker is absent from
`ordinary.stdout`, and the driver stderr capture is empty. Root reports the
separate approved ordinary `guest-kit-prepare` run
`f004fc91a009ef2c3b0ceeff` completed with status 0 and prepared artifacts;
its preparation output is not part of this capture.

The fixture body reads and validates the guest boot ID, prints and flushes the
body-start marker, then sleeps for 180 seconds. The driver waits for that marker
in the outer `ordinary.stdout` (`infra/metal/tests/admission_journey.sh:23-30`)
before it requests any owner observation. The captured Nextest timeout proves
the wrapper command timed out, but the capture contains no evidence whether
the body reached its first statement before timeout.

## Output and ownership path

The selected Nextest case is dispatched to the `core` guest group. The host
dispatcher invokes the actual selected ELF inside the disposable guest through
`GuestRun::exec_nextest_attempt`:

- `fwd_guest_suite.rs:1419-1447` sends the guest command through
  `self.guest.peer().exec(...)` and returns its status.
- `guest_peer.rs:1294-1300` sends a `PeerCommand::Exec` and consumes only the
  resulting `PeerReply::Exit(code)`; the protocol has no stdout frame.
- `bin/guest_peer.rs:1774-1788` runs the selected program with
  `Command::status()`, inheriting its stdout/stderr to the guest PID 1 console
  and replying with only the exit status.
- `fwd_guest_suite.rs:1638-1647` prints the disposable guest console only when
  `GuestRun::finish` is reached after a returned attempt. A Nextest timeout
  terminates the wrapper before this normal finish path can publish that
  console to the outer controller stream.

Thus `println!` plus `flush()` in the selected guest body does not place the
marker in the outer Metal controller's `ordinary.stdout`. `--no-capture` on
the outer Nextest command cannot change the guest peer protocol or expose the
VMM serial console. The missing marker is not evidence that the test body did
not start; this attempt has no separate host observation of its run-owned
console or a live guest owner at that point.

The result is a native-driver observability failure, not evidence of missing
guest-kit preparation, source-root misresolution, a durable admission failure,
or a selected-body assertion failure. The current owner path can take up to
the fixture's 10-second Nextest slow-timeout to prepare/provision the guest and
reach the body, while the driver relies on guest-console output appearing in a
different controller stream. The driver therefore cannot distinguish
pre-body timeout from a body that started and printed to the guest console.

The minimal follow-up belongs to the original acceptance author: retain the
body-start condition and observe it through an existing host-readable,
run-owned guest console/owner surface before the guest is cleaned up. Do not
weaken the timeout fixture or alter production stdout/status behavior to fit
the current probe. No owner snapshot was captured, so no admission, exclusive
wait, or waiter claim is made here.

No source/test change or remote command was made during this diagnosis. This
capture does not receive native acceptance credit and is not a DES phase event.
