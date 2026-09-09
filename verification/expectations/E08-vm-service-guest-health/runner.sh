#!/usr/bin/env bash
# E08 black-box expectation. The checked-in product example owns the exact
# built-binary journey; this runner supplies the canonical native-metal lease.
set -euo pipefail

readonly EXAMPLE="$REPO_ROOT/examples/service-kind-vm-workloads/run-example.sh"
readonly RAW_OUTPUT="$(mktemp "${TMPDIR:-/tmp}/svm-e08-metal.XXXXXX")"
export OVERDRIVE_METAL_KERNEL="${OVERDRIVE_METAL_KERNEL:-/srv/vm/overdrive-testing/kernel}"
export OVERDRIVE_METAL_ROOTFS="${OVERDRIVE_METAL_ROOTFS:-/srv/vm/overdrive-testing/rootfs.ext4}"

cleanup() {
  rm -f -- "$RAW_OUTPUT"
}
trap cleanup EXIT HUP INT TERM

"$EXAMPLE" check-source

{
  echo '# E08 invocation'
  echo 'command: cargo xtask metal run -- examples/service-kind-vm-workloads/run-example.sh run healthy'
  echo "started_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
} >"$EVIDENCE_DIR/product-run.meta"

set +e
timeout 900s cargo xtask metal run -- \
  examples/service-kind-vm-workloads/run-example.sh run healthy \
  >"$RAW_OUTPUT" 2>&1
run_rc=$?
set -e

sed -E 's/@[A-Za-z0-9._-]+/@<metal-host-redacted>/g' \
  <"$RAW_OUTPUT" >"$EVIDENCE_DIR/product-run.out"
{
  echo "finished_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "exit: $run_rc"
} >>"$EVIDENCE_DIR/product-run.meta"

[[ "$run_rc" -eq 0 ]] || {
  cat "$EVIDENCE_DIR/product-run.out" >&2
  exit "$run_rc"
}
grep -Fq 'E08 PASS: peer VM Job received byte-exact SVM-E08-GUEST-OK through the Service frontend' \
  "$EVIDENCE_DIR/product-run.out" || {
  echo 'E08 runner: product example exited zero without the public peer-VM success result' >&2
  exit 1
}

cat "$EVIDENCE_DIR/product-run.out"
