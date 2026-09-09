#!/usr/bin/env bash
set -euo pipefail

readonly EXAMPLE="examples/service-kind-vm-workloads/run-example.sh"
readonly EXAMPLE_SPEC="examples/service-kind-vm-workloads/liveness-restart.toml"
readonly REMOTE_RUN_BUDGET_SECONDS=1200
readonly REMOTE_CLEANUP_GRACE_SECONDS=90
readonly TRANSPORT_TIMEOUT_SECONDS=$((REMOTE_RUN_BUDGET_SECONDS + REMOTE_CLEANUP_GRACE_SECONDS + 60))
readonly RAW_OUTPUT="$(mktemp "${TMPDIR:-/tmp}/svm-e12-metal.XXXXXX")"
readonly OUTPUT_ROOT="/srv/vm/overdrive-testing/svm-e08"
export OVERDRIVE_METAL_KERNEL="${OVERDRIVE_METAL_KERNEL:-/srv/vm/overdrive-testing/kernel}"
export OVERDRIVE_METAL_ROOTFS="${OVERDRIVE_METAL_ROOTFS:-/srv/vm/overdrive-testing/rootfs.ext4}"

cleanup() { rm -f -- "$RAW_OUTPUT"; }
trap cleanup EXIT HUP INT TERM

"$REPO_ROOT/$EXAMPLE" check-source
expected_threshold="$(awk '
  /^\[\[health_check\.liveness\]\]/ { in_liveness = 1; next }
  in_liveness && /^\[\[/ { exit }
  in_liveness && $1 == "failure_threshold" { print $3; exit }
' "$REPO_ROOT/$EXAMPLE_SPEC")"
[[ "$expected_threshold" =~ ^[1-9][0-9]*$ ]] \
  || { echo "E12 runner: liveness fixture omitted a positive failure threshold" >&2; exit 1; }
{
  echo '# E12 invocation'
  echo "command: cargo xtask metal run -- $EXAMPLE run liveness-restart"
  echo "remote_command: timeout --signal=TERM --kill-after=${REMOTE_CLEANUP_GRACE_SECONDS}s ${REMOTE_RUN_BUDGET_SECONDS}s env SVM_E08_OUTPUT_ROOT=$OUTPUT_ROOT $EXAMPLE run liveness-restart"
  echo "remote_timeout: ${REMOTE_RUN_BUDGET_SECONDS}s journey + ${REMOTE_CLEANUP_GRACE_SECONDS}s cleanup grace"
  echo "transport_timeout: ${TRANSPORT_TIMEOUT_SECONDS}s"
  echo "output_root: $OUTPUT_ROOT"
  echo "started_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
} >"$EVIDENCE_DIR/product-run.meta"

set +e
timeout --foreground --signal=TERM --kill-after=30s "${TRANSPORT_TIMEOUT_SECONDS}s" \
  cargo xtask metal run -- bash -lc \
  "timeout --signal=TERM --kill-after=${REMOTE_CLEANUP_GRACE_SECONDS}s ${REMOTE_RUN_BUDGET_SECONDS}s env SVM_E08_OUTPUT_ROOT=$OUTPUT_ROOT $EXAMPLE run liveness-restart" \
  >"$RAW_OUTPUT" 2>&1
run_rc=$?
set -e
sed -E 's/@[A-Za-z0-9._-]+/@<metal-host-redacted>/g' <"$RAW_OUTPUT" \
  >"$EVIDENCE_DIR/product-run.out"
{
  echo "finished_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "exit: $run_rc"
} >>"$EVIDENCE_DIR/product-run.meta"

awk '/^--- E12 ledger begin ---$/ { keep=1; next }
  /^--- E12 ledger end ---$/ { keep=0 }
  keep' "$EVIDENCE_DIR/product-run.out" >"$EVIDENCE_DIR/liveness-restart.tsv"

[[ "$run_rc" -eq 0 ]] || {
  cat "$EVIDENCE_DIR/product-run.out" >&2
  exit "$run_rc"
}

row_count="$(awk -F '\t' 'NR > 1 { count++ } END { print count + 0 }' \
  "$EVIDENCE_DIR/liveness-restart.tsv")"
[[ "$row_count" -eq 3 ]] \
  || { echo "E12 runner: expected exactly three lifecycle rows, observed $row_count" >&2; exit 1; }

awk -F '\t' -v expected_threshold="$expected_threshold" '
  NR == 1 {
    if ($0 != "phase\tallocation_id\tstate\trestarts\tprobe_role\tprobe_status\tterminal_reason\tresponse\tstartup_observed_at\tstarted_at\tthreshold_failures") exit 1
    next
  }
  {
    if (NF != 11 || ($1 != "before" && $1 != "terminal" && $1 != "after") \
      || $2 !~ /^alloc-/ || $3 !~ /^(Running|Terminated|Failed)$/ \
      || $4 !~ /^[0-9]+$/ || $5 != "liveness" \
      || $9 !~ /^[0-9]+$/ || $10 == "" || $11 !~ /^[0-9]+$/) exit 1
    if ($1 == "before" \
      && ($3 != "Running" || $4 != "0" || $6 != "pass" || $7 != "none" \
        || $8 != "HTTP 204" || $11 != "0")) exit 1
    if ($1 == "terminal" \
      && ($3 !~ /^(Terminated|Failed)$/ || $4 != "0" || $6 != "fail" \
        || $7 != "liveness-probe" || $8 != "HTTP 503" \
        || $11 < expected_threshold)) exit 1
    if ($1 == "after") {
      if ($3 != "Running" || $4 !~ /^[1-9][0-9]*$/ || $7 != "liveness-probe") exit 1
      if (($6 != "pass" && $6 != "fail") \
        || ($6 == "pass" && $8 != "HTTP 204") \
        || ($6 == "fail" && $8 != "HTTP 503")) exit 1
    }
    seen[$1]++
    ids[$1] = $2
    startup_observed_at[$1] = $9
    started_at[$1] = $10
    threshold_failures[$1] = $11
    rows++
  }
  END {
    if (rows != 3 || seen["before"] != 1 || seen["terminal"] != 1 || seen["after"] != 1) exit 1
    if (ids["before"] != ids["terminal"] || ids["terminal"] != ids["after"]) exit 1
    if (startup_observed_at["before"] == startup_observed_at["after"] || started_at["before"] == started_at["after"]) exit 1
    if (threshold_failures["after"] != threshold_failures["terminal"]) exit 1
  }
' "$EVIDENCE_DIR/liveness-restart.tsv" \
  || { echo 'E12 runner: lifecycle, identity, probe, threshold, or fresh-replacement assertions failed' >&2; exit 1; }

grep -Fq 'last terminated:' "$EVIDENCE_DIR/product-run.out" \
  || { echo 'E12 runner: public describe omitted the prior terminal observation' >&2; exit 1; }
! grep -Eqi 'readiness probe\[0\].*last=fail' "$EVIDENCE_DIR/product-run.out" \
  || { echo 'E12 runner: readiness failure was incorrectly attributed to the restart' >&2; exit 1; }
grep -Fq 'E12 teardown deltas: vm=0 probe=0 network=0 cgroup=0 run-directory=0 mount=0 loop=0 preparation=0' \
  "$EVIDENCE_DIR/product-run.out" \
  || { echo 'E12 runner: teardown did not leave a zero cleanup delta' >&2; exit 1; }
grep -Fq 'E12 PASS: liveness stop, ordinary same-ID replacement, no readiness restart, no dead revival' \
  "$EVIDENCE_DIR/product-run.out"
cat "$EVIDENCE_DIR/product-run.out"
