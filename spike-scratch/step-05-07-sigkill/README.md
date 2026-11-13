# Historical VM termination diagnostic

This script was prepared on 2026-11-12 while investigating early `SIGKILL`
results in the guest-listener loopback test. It is a diagnostic source record,
not a product example, maintained regression test, or proof of the signal's cause.

The retained captures used source `5724c9da1e5c2436daa4f5990441f994fe0c8d06`.
Two canonical exclusive runs (`2c408ab4e09092e739473040` and
`a39a5c84003126535c2ff684`) produced four passing loopback controls. The second
run's concurrent full-host server failed at its occupied XDP slot before VM
reclamation, so it did not establish the proposed foreign-server kill path.

The checked-in revision changes that concurrent server to the existing
`serve_kernel_assigned_vsock_ports` fixture, whose simulated dataplane can
reach the real host-state reclamation boundary. That revised experiment did
not reach execution during the investigation. Its prediction remains unproven;
the script and its comments must not be cited as an observed failure or fix.

The investigation's workspace captures are under
`.context/step-05-07-native/post-rebase/`. The completed feature's reviewed
native receipts and their later serial-control results are linked from
[the final integration record](../../docs/feature/service-allocation-readiness-and-ending/deliver/final-integration-verification.md).
They do not attribute the earlier signal to this hypothesis.

The script observes host-wide VM scopes and therefore requests the existing
canonical exclusive maintenance class. It can install `strace` if missing.
Review those operations and the current fixture paths before any future use;
this historical revision has not been validated against the final source tree.
