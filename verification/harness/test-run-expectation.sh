#!/usr/bin/env bash
# Host-safe branch coverage for run-expectation.sh. All fixture repositories
# and evidence stay under a temporary directory; no expectation is executed.
set -euo pipefail

HARNESS_SOURCE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/run-expectation.sh"
readonly HARNESS_SOURCE
TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/overdrive-harness-test.XXXXXX")"
trap 'rm -rf -- "$TEST_ROOT"' EXIT HUP INT TERM

fail() {
  echo "run-expectation harness test: $*" >&2
  exit 1
}

assert_manifest() {
  local manifest="$1"
  local expected_status="$2"
  local expected_substrate="$3"
  local expected_lima="$4"
  grep -Fxq "execution_status: \"$expected_status\"" "$manifest" \
    || fail "$manifest does not record status $expected_status"
  grep -Fxq "execution_substrate: \"$expected_substrate\"" "$manifest" \
    || fail "$manifest does not record substrate $expected_substrate"
  grep -Fxq "executed_in_lima: $expected_lima" "$manifest" \
    || fail "$manifest does not record executed_in_lima=$expected_lima"
}

make_expectation() {
  local id="$1"
  local substrate="$2"
  local runner_rc="$3"
  local expectation="$TEST_ROOT/verification/expectations/${id}-fixture"
  mkdir -p "$expectation"
  printf '# %s fixture\n\n- Anchor: host-safe harness branch test\n' "$id" \
    >"$expectation/README.md"
  if [[ "$substrate" != "default" ]]; then
    printf '%s\n' "$substrate" >"$expectation/execution-substrate"
  fi
  if [[ "$runner_rc" != "absent" ]]; then
    printf '#!/usr/bin/env bash\nexit %s\n' "$runner_rc" >"$expectation/runner.sh"
    chmod 0755 "$expectation/runner.sh"
  fi
}

run_case() {
  local id="$1"
  local expected_rc="$2"
  local output="$TEST_ROOT/${id}.out"
  local actual_rc
  set +e
  "$TEST_ROOT/verification/harness/run-expectation.sh" "$id" >"$output" 2>&1
  actual_rc=$?
  set -e
  if [[ "$expected_rc" == "zero" ]]; then
    [[ "$actual_rc" -eq 0 ]] || fail "$id returned $actual_rc, expected success"
  else
    [[ "$actual_rc" -ne 0 ]] || fail "$id returned success, expected fail-closed"
  fi
}

mkdir -p "$TEST_ROOT/verification/harness" "$TEST_ROOT/verification/expectations"
cp "$HARNESS_SOURCE" "$TEST_ROOT/verification/harness/run-expectation.sh"
chmod 0755 "$TEST_ROOT/verification/harness/run-expectation.sh"
git -C "$TEST_ROOT" init -q
git -C "$TEST_ROOT" config user.name harness-test
git -C "$TEST_ROOT" config user.email harness-test@example.invalid

make_expectation S01 default 0
make_expectation S02 native-metal 0
make_expectation S03 default 75
make_expectation S04 other 9
make_expectation S05 native-metal absent
make_expectation S06 invalid-substrate 0
git -C "$TEST_ROOT" add verification
git -C "$TEST_ROOT" commit -qm 'test fixtures'

run_case S01 zero
assert_manifest "$TEST_ROOT/verification/expectations/S01-fixture/evidence/verification.yaml" \
  succeeded lima true

run_case S02 zero
assert_manifest "$TEST_ROOT/verification/expectations/S02-fixture/evidence/verification.yaml" \
  succeeded native-metal false

run_case S03 nonzero
assert_manifest "$TEST_ROOT/verification/expectations/S03-fixture/evidence/verification.yaml" \
  pending lima true

run_case S04 nonzero
assert_manifest "$TEST_ROOT/verification/expectations/S04-fixture/evidence/verification.yaml" \
  failed other false

run_case S05 nonzero
assert_manifest "$TEST_ROOT/verification/expectations/S05-fixture/evidence/verification.yaml" \
  pending native-metal false
grep -Fxq 'runner_invoked: false' \
  "$TEST_ROOT/verification/expectations/S05-fixture/evidence/verification.yaml" \
  || fail "absent-runner branch did not record runner_invoked=false"

run_case S06 nonzero
grep -Fq "invalid execution substrate 'invalid-substrate'" "$TEST_ROOT/S06.out" \
  || fail "invalid-substrate branch did not emit its diagnostic"

echo "run-expectation harness branch tests passed"
