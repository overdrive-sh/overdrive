#!/usr/bin/env bash
# E07 black-box expectation: run the checked-in product example once on the
# qualified native-metal substrate. Internal D7 guarantees remain exclusively
# owned by the Rust integration suite.
set -euo pipefail

readonly EXAMPLE_DIR="$REPO_ROOT/examples/guest-stack-transparent-mtls-intercept"
readonly EXAMPLE_RUNNER="$EXAMPLE_DIR/run-example.sh"
export OVERDRIVE_METAL_KERNEL="${OVERDRIVE_METAL_KERNEL:-/srv/vm/overdrive-testing/kernel}"
export OVERDRIVE_METAL_ROOTFS="${OVERDRIVE_METAL_ROOTFS:-/srv/vm/overdrive-testing/rootfs.ext4}"
RAW_OUTPUT="$(mktemp "${TMPDIR:-/tmp}/gti-e07-metal.XXXXXX")"
readonly RAW_OUTPUT

cleanup() {
  rm -f -- "$RAW_OUTPUT"
}
trap cleanup EXIT HUP INT TERM

redact_transport() {
  sed -E \
    -e 's/@[A-Za-z0-9._-]+/@<metal-host-redacted>/g' \
    -e 's/(ssh[^[:space:]]*[[:space:]]+)[A-Za-z0-9._-]+@[^[:space:]]+/\1<metal-target-redacted>/g'
}

"$EXAMPLE_RUNNER" check-source

{
  echo '# E07 invocation'
  echo 'command: cargo xtask metal run -- examples/guest-stack-transparent-mtls-intercept/run-example.sh run'
  echo "started_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
} >"$EVIDENCE_DIR/product-run.meta"

set +e
timeout 900s cargo xtask metal run -- \
  examples/guest-stack-transparent-mtls-intercept/run-example.sh run \
  >"$RAW_OUTPUT" 2>&1
run_rc=$?
set -e

redact_transport <"$RAW_OUTPUT" >"$EVIDENCE_DIR/product-run.out"
{
  echo "finished_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "exit: $run_rc"
} >>"$EVIDENCE_DIR/product-run.meta"

[[ "$run_rc" -eq 0 ]] || {
  cat "$EVIDENCE_DIR/product-run.out" >&2
  exit "$run_rc"
}
grep -Fq 'E07 PASS: one VM Job called one Exec Service and received the exact reply' \
  "$EVIDENCE_DIR/product-run.out" || {
  echo 'E07 runner: product example exited zero without its public success result' >&2
  exit 1
}

cat "$EVIDENCE_DIR/product-run.out"
echo 'E07 expectation PASS: checked-in VM Job received the Exec Service reply'
