# Placement rerun — guest-lane artifact check uses the wrong target directory

This directory preserves the current placement capture copied from
`/tmp/metal-native-remaining-20261109T065706Z-placement`. Its foreground
invocation reports exit 1 at source HEAD
`dd85152c439151a7703475f5b53b7f70b8acfc78`; the dirty source status is retained
in `foreground-invocation.json`.

## Captured result

The direct-native and named-nested cases passed in this attempt:

| Case | Run ID | Result |
|---|---|---|
| Direct-native production VMM identity | `e6dff0eeabb316ade8cc2a40` | 0; retained V1C identity, confinement, vhost-device, and data-round-trip receipt. |
| Named nested guest boundary | `0c0863c9f6923e640147a3dc` | 0. |
| Guest-lane operator selected ELF | `e729cf26ccc3b47d53f750f2` | 100; `lane_not_prepared` before the selected body. |

Foreign-fixture setup and cleanup both returned 0. The selected operator
command was `cargo nextest run -p overdrive-init --bin overdrive-init
--features integration-tests,kvm-tests -E
'test(guest_flow::real_kernel::the_accept_path_matches_the_sim_per_connection)'
--no-capture`. Nextest failed in 0.063 seconds with
`OVERDRIVE_METAL_TEST_ERROR error=lane_not_prepared`. The driver then exited 1
and did not run later placement cases or write its final `after.json`.

## Existing producer and consumer paths

The closed placement map assigns
`overdrive-init::bin/overdrive-init::guest_flow::real_kernel::the_accept_path_matches_the_sim_per_connection`
to `guest-lane-operator` with fixture
`integration::in_guest_lane::the_guest_accept_path_matches_the_sim_in_a_real_guest`
(`infra/metal/test-placement.json`). That descriptor and the selected binary
identity are present; missing or malformed placement would produce the
existing placement refusal, not this lane-preparation refusal.

The prepared dispatcher and the prepared in-guest test binary have different
existing locations:

| Artifact | Existing producer path |
|---|---|
| Host dispatcher | `<checkout>/target/debug/overdrive-fwd-guest-suite` (`bootstrap.sh:673`; `guest-kit_prepare.rs:213-216`) |
| `overdrive-init` in-guest `--bin` test ELF | `<target>/guest-kit/in-guest/overdrive-init--overdrive-init` (`guest-kit_prepare.rs:169-173, 286-293`; `guest_lane.rs:144-162`) |

`test-wrapper.sh:213-224` correctly resolves and validates the dispatcher, but
then derives the lane ELF as a sibling of the dispatcher:
`<checkout>/target/debug/in-guest/overdrive-init--overdrive-init`. That path
does not match the existing preparation owner’s `target/guest-kit/in-guest`
output. The wrapper emits the generic `lane_not_prepared` marker and captures
no `stat` of the tested path, so this raw run does not prove which of the
wrapper's artifact existence predicates failed. The exact producer/consumer
path mismatch is present in source and explains the refusal if the existing
prepared file is at the producer path.

The earlier `guest-kit-prepare` receipt retained with the 054741Z attempt
reports the `overdrive-init overdrive-init` artifact at
`target/guest-kit/in-guest/overdrive-init--overdrive-init`; that is a separate
run and source revision, not proof of the remote file state for this 065706Z
run. The current producer function retains the same path contract. The
minimal private correction is for `test-wrapper.sh` to check the existing
`<source-root>/target/guest-kit/in-guest/overdrive-init--overdrive-init`
artifact rather than deriving it from the dispatcher directory. Keep the
check fail-closed and retain the selected binary identity and fixture
mapping. No public API, flag, configuration, test body, or oracle change is
indicated.
