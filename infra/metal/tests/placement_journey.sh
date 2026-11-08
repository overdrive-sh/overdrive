#!/usr/bin/env bash
# External developer-tool acceptance; never an Overdrive expectation runner.
# CONTRACT_SHAPE: bounded-change (selected actual attempts and invocation roots;
# the initial physical test-kernel state is the read-only observed complement).
set -euo pipefail
[ "$#" = 2 ] || { echo 'usage: placement_journey.sh CHECKOUT OUTPUT_DIR' >&2; exit 64; }
checkout="$(cd "$1" && pwd -P)"
mkdir -p "$2"
out="$(cd "$2" && pwd -P)"
[ -z "$(ls -A "$out")" ] || { echo "choose a new empty evidence directory" >&2; exit 64; }
python3 - "$out" "$checkout" <<'PY_OUTPUT'
import sys
from pathlib import Path
out=Path(sys.argv[1]).resolve()
if any(out.is_relative_to(Path(source).resolve()) for source in sys.argv[2:]):
 raise SystemExit('Choose an empty evidence directory outside the input worktrees so capture cannot change source identity')
PY_OUTPUT
exec 2> >(tee -a "$out/driver.stderr" >&2)
: "${OVERDRIVE_METAL_TARGET:?export existing configured Metal target}"
observe() { ssh "$OVERDRIVE_METAL_TARGET" 'sudo -n python3 -' "$@" < "$checkout/infra/metal/tests/observe_host.py"; }
run_case() {
  local label="$1" expected="$2"; shift 2
  local status=0
  (cd "$checkout" && cargo xtask metal "$@") > "$out/$label.stdout" 2> "$out/$label.stderr" || status=$?
  printf '%s\n' "$status" > "$out/$label.status"
  [ "$status" = "$expected" ] || { cat "$out/$label.stderr" >&2; echo "$label: expected $expected, got $status" >&2; exit 1; }
}
observe > "$out/before.json"
# The negative foreign-path cases use one fresh driver-owned external fixture.
# If a refusal regresses, the selected target remains owned test scratch rather
# than another invocation's source/build tree. EX setup/cleanup is explicit.
run_case foreign-fixture-setup 0 run --exclusive -- mktemp -d /tmp/metal-placement-foreign.XXXXXXXX
foreign_root="$(sed -n '/^\/tmp\/metal-placement-foreign\.[A-Za-z0-9]*$/p' "$out/foreign-fixture-setup.stdout" | tail -1)"
[[ "$foreign_root" =~ ^/tmp/metal-placement-foreign\.[A-Za-z0-9]+$ ]] || { echo 'missing actual owned external fixture path' >&2; exit 1; }
cleanup_external_fixture() {
  local original="$1" cleanup_status=0
  trap - EXIT
  (cd "$checkout" && cargo xtask metal run --exclusive -- python3 -c 'import shutil,sys; shutil.rmtree(sys.argv[1])' "$foreign_root") > "$out/foreign-fixture-cleanup.stdout" 2> "$out/foreign-fixture-cleanup.stderr" || cleanup_status=$?
  printf '%s\n' "$cleanup_status" > "$out/foreign-fixture-cleanup.status"
  if [ "$original" = 0 ] && [ "$cleanup_status" != 0 ]; then exit "$cleanup_status"; fi
  exit "$original"
}
trap 'cleanup_external_fixture "$?"' EXIT
safe_filter='test(=acceptance::metal_run_isolation::placement::non_metal_and_explicit_exclusive_attempts_preserve_the_original_exec_chain)'
host_boot_id="$(ssh "$OVERDRIVE_METAL_TARGET" 'cat /proc/sys/kernel/random/boot_id')"
run_case attempts 100 run -- env PROPTEST_CASES=7 PROPTEST_RNG_SEED=123456 RUST_BACKTRACE=1 "RUST_LOG=metal_host_boot_id=$host_boot_id" cargo nextest run --manifest-path infra/metal/tests/fixtures/attempt/Cargo.toml --target-dir target/metal-run-isolation-fixture --run-ignored only -E 'test(selected_environment_and_status)' --retries 1 --no-capture
python3 - "$out/attempts.stdout" <<'PY'
import re,sys
rows=re.findall(r'METAL-ISOLATION-ATTEMPT attempt=(\d+) boot_id=([0-9a-f-]+) helper=true',open(sys.argv[1]).read())
assert len(rows)==2 and {r[0] for r in rows}=={'1','2'}, rows
assert len({r[1] for r in rows})==2, 'a retry must have a fresh guest kernel'
PY
attempt_id="$(sed -n 's/^OVERDRIVE_METAL_RUN_ID=\([0-9a-f]\{24\}\)$/\1/p' "$out/attempts.stderr" | head -1)"
run_case attempts-collect 100 collect "$attempt_id"
run_case attempts-repeat 100 collect "$attempt_id"
cmp "$out/attempts-collect.stdout" "$out/attempts-repeat.stdout"
cmp "$out/attempts-collect.stderr" "$out/attempts-repeat.stderr"
run_case timeout 100 run -- env "RUST_LOG=metal_host_boot_id=$host_boot_id" cargo nextest run --manifest-path infra/metal/tests/fixtures/attempt/Cargo.toml --target-dir target/metal-run-isolation-fixture --run-ignored only -E 'test(selected_attempt_waits_for_its_timeout)' --no-capture
rg -q 'METAL-ISOLATION-TIMEOUT body_started=true' "$out/timeout.stdout"
rg -q 'TIMEOUT' "$out/timeout.stdout"
timeout_id="$(sed -n 's/^OVERDRIVE_METAL_RUN_ID=\([0-9a-f]\{24\}\)$/\1/p' "$out/timeout.stderr" | head -1)"
run_case timeout-collect 100 collect "$timeout_id"
observe "$timeout_id" > "$out/timeout.complete.json"
python3 - "$out/timeout.complete.json" <<'PY_TIMEOUT'
import json,sys
for owner in json.load(open(sys.argv[1]))['owners'].values():
 assert owner['completion.v1']['kind']=='complete'
 assert not owner['processes'] and not any(root['exists'] for root in owner['roots'].values())
PY_TIMEOUT
run_case guest-execution-failure 100 run -- env "RUST_LOG=metal_host_boot_id=$host_boot_id" cargo nextest run --manifest-path infra/metal/tests/fixtures/attempt/Cargo.toml --target-dir target/metal-run-isolation-fixture --run-ignored only -E 'test(selected_attempt_resets_its_guest)' --no-capture
rg -q 'METAL-ISOLATION-RESET guest_boot_id=.* resetting=true' "$out/guest-execution-failure.stdout"
rg -q 'OVERDRIVE_METAL_TEST_ERROR error=guest_execution_failed' "$out/guest-execution-failure.stdout"
rg -q 'guest peer.*control connection|VMM.*exited' "$out/guest-execution-failure.stdout"
for profile in default ci; do
  run_case "profile-$profile" 100 run -- env "RUST_LOG=metal_host_boot_id=$host_boot_id" cargo nextest run -p xtask --test acceptance --features integration-tests --profile "$profile" --run-ignored only -E 'test(=acceptance::metal_run_isolation::placement::selected_guest_body_uses_its_own_kernel_in_existing_profiles)' --no-capture
  rg -q "METAL-ISOLATION-PROFILE guest_boot_id=.* profile=$profile" "$out/profile-$profile.stdout"
done
run_case direct-native 0 run -- cargo nextest run -p overdrive-host --test integration --features integration-tests,kvm-tests -E 'test(integration::vsock_vmm_device::the_fork_boots_and_passes_data_under_the_production_launch_identity)' --no-capture
run_case named-nested 0 run -- cargo nextest run -p overdrive-fwd --test integration --features integration-tests,kvm-tests -E 'test(integration::guest_boundary::a_guest_application_reset_reaches_the_host_application_as_a_reset)' --no-capture
run_case operator-cell 0 run -- cargo nextest run -p overdrive-init --bin overdrive-init --features integration-tests,kvm-tests -E 'test(guest_flow::real_kernel::the_accept_path_matches_the_sim_per_connection)' --no-capture
run_case owner-cell 0 run -- cargo nextest run -p overdrive-init --bin overdrive-init --features integration-tests,kvm-tests -E 'test(guest_flow::real_kernel::a_burst_of_host_aborts_resets_every_application_connection)' --no-capture
run_case command-255 255 run --exclusive -- sh -c 'exit 255'
command_id="$(sed -n 's/^OVERDRIVE_METAL_RUN_ID=\([0-9a-f]\{24\}\)$/\1/p' "$out/command-255.stderr" | head -1)"
run_case command-255-collect 255 collect "$command_id"
run_case command-255-repeat 255 collect "$command_id"
cmp "$out/command-255-collect.stdout" "$out/command-255-repeat.stdout"
cmp "$out/command-255-collect.stderr" "$out/command-255-repeat.stderr"
run_case unknown-invariant 1 run -- cargo run -p overdrive-sim --bin dst -- --only metal-isolation-does-not-exist
run_case raw-test 64 run -- cargo test -p overdrive-core
run_case shell-chain 64 run -- sh -c true
run_case host-module 64 run -- cargo xtask kmod-load --help
run_case config-replacement 64 run -- cargo nextest run -p xtask --test acceptance -E "$safe_filter" --config-file .config/nextest.toml
run_case debugger-replacement 64 run -- cargo nextest run -p xtask --test acceptance -E "$safe_filter" --debugger true
run_case archive-reuse 64 run -- cargo nextest run -p xtask --test acceptance -E "$safe_filter" --archive-file "$foreign_root/absent.tar"
run_case foreign-target 64 run -- cargo nextest run -p xtask --test acceptance -E "$safe_filter" --target-dir "$foreign_root/target"
run_case foreign-manifest 64 run -- cargo nextest run -p xtask --test acceptance -E "$safe_filter" --manifest-path "$foreign_root/absent/Cargo.toml"
run_case duplicate-env 64 run -- env RUST_LOG=info RUST_LOG=debug cargo check -p xtask
run_case context-env 64 run -- env OVERDRIVE_METAL_RUN_ID=0123456789abcdef01234567 cargo check -p xtask
run_case invalid-bpf-selector 64 run -- env OVERDRIVE_BPF_NATIVE=0 cargo check -p xtask
run_case invalid-group 64 run -- env OVERDRIVE_FWD_GUEST_GROUP=unknown cargo check -p xtask
run_case env-option 64 run -- env -i cargo check -p xtask
run_case conflicting-package 64 run -- cargo run -p overdrive-sim --package overdrive-testing --bin dst
for label in raw-test shell-chain host-module duplicate-env context-env invalid-bpf-selector invalid-group env-option conflicting-package; do
  rg -q 'OVERDRIVE_METAL_RUNNER_ERROR error=unsupported_ordinary_command' "$out/$label.stderr"
done
for label in config-replacement debugger-replacement archive-reuse; do
  rg -q 'OVERDRIVE_METAL_RUNNER_ERROR error=execution_override_refused' "$out/$label.stderr"
done
for label in foreign-target foreign-manifest; do
  rg -q 'OVERDRIVE_METAL_RUNNER_ERROR error=artifact_scope_mismatch' "$out/$label.stderr"
done
run_case invalid-mode 2 sync --exclusive
for flags in '' '--no-sync' '--no-sudo' '--no-sync --no-sudo'; do
  case_label="pure$(printf '%s' "$flags" | tr ' ' '_')"
  # Splitting is confined to these four checked-in test inputs.
  # shellcheck disable=SC2086
  run_case "$case_label" 0 run $flags -- cargo nextest run -p xtask --test acceptance --features integration-tests -E 'test(placement::non_metal_and_explicit_exclusive_attempts_preserve_the_original_exec_chain)'
done
observe "$attempt_id" "$command_id" > "$out/after.json"
python3 - "$out" <<'PY'
import json,sys
from pathlib import Path
out=Path(sys.argv[1]); before=json.loads((out/'before.json').read_text()); after=json.loads((out/'after.json').read_text())
assert before['physical']==after['physical'], 'ordinary test physical-kernel complement changed'
for o in after['owners'].values():
 assert o['completion.v1']['kind']=='complete'
 assert not o['processes'] and not any(root['exists'] for root in o['roots'].values())
print('METAL-ISOLATION-PLACEMENT exact_guest_attempts=true fresh_retry=true actual_helper=true nextest_status=100 command_status=255 physical_complement_unchanged=true')
PY
