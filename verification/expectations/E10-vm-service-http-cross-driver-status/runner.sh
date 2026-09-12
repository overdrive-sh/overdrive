#!/usr/bin/env bash
set -euo pipefail

readonly EXAMPLE="examples/service-kind-vm-workloads/run-example.sh"
EXPECTATION_SLUG="$(basename "$EXPECTATION_DIR")"
readonly EXPECTATION_SLUG
readonly REMOTE_CAPTURE_ROOT="verification/expectations/$EXPECTATION_SLUG/evidence/raw-cells"
readonly LOCAL_CAPTURE_ROOT="$EVIDENCE_DIR/raw-cells"
readonly E10_OUTPUT_ROOT="/srv/vm/overdrive-testing/svm-e08"
CAPTURE_TOKEN="e10-$(date -u +%Y%m%dT%H%M%SZ)-$$"
readonly CAPTURE_TOKEN
RAW_OUTPUT="$(mktemp "${TMPDIR:-/tmp}/svm-e10-metal.XXXXXX")"
readonly RAW_OUTPUT
RAW_PULL="$(mktemp "${TMPDIR:-/tmp}/svm-e10-pull.XXXXXX")"
readonly RAW_PULL
export OVERDRIVE_METAL_KERNEL="${OVERDRIVE_METAL_KERNEL:-/srv/vm/overdrive-testing/kernel}"
export OVERDRIVE_METAL_ROOTFS="${OVERDRIVE_METAL_ROOTFS:-/srv/vm/overdrive-testing/rootfs.ext4}"

cleanup() { rm -f -- "$RAW_OUTPUT" "$RAW_PULL"; }
trap cleanup EXIT HUP INT TERM

resolve_metal_target() {
  if [[ -n "${OVERDRIVE_METAL_TARGET:-}" ]]; then
    printf '%s' "$OVERDRIVE_METAL_TARGET"
    return 0
  fi
  if [[ -f "$REPO_ROOT/.env" ]]; then
    sed -n 's/^OVERDRIVE_METAL_TARGET=//p' "$REPO_ROOT/.env" | tail -1
  fi
}

require_file() {
  local path="$1"
  [[ -f "$path" ]] || {
    echo "E10 runner: required raw evidence is missing: $path" >&2
    return 1
  }
}

first_alloc_row() {
  awk '$1 ~ /^alloc-/ { print $1, $2, $3, $4; exit }' "$1"
}

e10_failed_probe_is_preserved() {
  local before_line="$1"
  local after_line="$2"
  local pattern='^([[:space:]]*startup[[:space:]]+probe\[0\][[:space:]]+http[[:space:]]+GET[[:space:]]+[^[:space:]]+[[:space:]]+last=fail[[:space:]]+\(.*\))[[:space:]]+last_observed_at=([0-9]+)$'
  local before_semantics before_observed_at after_semantics after_observed_at

  [[ "$before_line" =~ $pattern ]] || return 1
  before_semantics="${BASH_REMATCH[1]}"
  before_observed_at="${BASH_REMATCH[2]}"
  [[ "$after_line" =~ $pattern ]] || return 1
  after_semantics="${BASH_REMATCH[1]}"
  after_observed_at="${BASH_REMATCH[2]}"

  [[ "$after_semantics" == "$before_semantics" ]] \
    && [[ "$after_observed_at" -ge "$before_observed_at" ]]
}

snapshot_network_names() {
  awk '
    /^\[network\]$/ { in_network = 1; next }
    in_network && /^\[/ { exit }
    in_network && $1 ~ /^ovd-(hv|wl|tp|ns)-/ {
      split($1, parts, "@")
      print parts[1]
    }
  ' "$1" | sort -u
}

snapshot_materialization() {
  awk '
    /^\[materialization\]$/ { getline; print; exit }
  ' "$1"
}

snapshot_allocation_hypervisors() {
  awk '
    /^\[allocation-hypervisors\]$/ { in_hypervisors = 1; next }
    in_hypervisors && /^\[/ { exit }
    in_hypervisors && NF { print }
  ' "$1"
}

validate_e10_named_cleanup() {
  local cell="$1"
  local driver="$2"
  local status="$3"
  local alloc_id="alloc-service-$driver-http-$status-0"
  local observed="$cell/current-session-owned-resources.log"
  local current="$cell/current-session-resources.log"
  local runtime_after="$cell/post-runtime-cleanup-resources.log"
  local final_after="$cell/cleanup-resources-after.log"
  local complement="$cell/cleanup-resource-complement.log"
  local kind name observed_count recorded_count

  for name in "$observed" "$current" "$runtime_after" "$final_after" "$complement"; do
    require_file "$name" || return 1
  done
  grep -Fxq "allocation=$alloc_id" "$observed" || return 1
  for name in "$runtime_after" "$final_after"; do
    grep -Fq '[hypervisors]' "$name" || return 1
    grep -Fq '[cgroup-scopes]' "$name" || return 1
    grep -Fq '[vm-run-directories]' "$name" || return 1
    grep -Fq '[network]' "$name" || return 1
    grep -Fq '[loop-devices]' "$name" || return 1
    grep -Fq '[mounts]' "$name" || return 1
    grep -Fq '[allocation-hypervisors]' "$name" || return 1
    [[ -z "$(snapshot_allocation_hypervisors "$name")" ]] || return 1
    if grep -Fq "$alloc_id" "$name"; then
      echo "E10 runner: residual allocation resource in $name" >&2
      return 1
    fi
  done
  [[ "$(snapshot_materialization "$final_after")" == absent ]] || return 1
  if grep -Fq "$E10_OUTPUT_ROOT" "$final_after"; then
    echo "E10 runner: residual mount or loop resource in $final_after" >&2
    return 1
  fi

  observed_count=0
  while IFS='=' read -r kind name; do
    [[ "$kind" != allocation ]] || continue
    [[ -n "$kind" && -n "$name" ]] || return 1
    observed_count=$((observed_count + 1))
    case "$kind" in
      network)
        grep -Fxq "$name" <(snapshot_network_names "$current") || return 1
        ! grep -Fxq "$name" <(snapshot_network_names "$runtime_after") || return 1
        ! grep -Fxq "$name" <(snapshot_network_names "$final_after") || return 1
        ;;
      hypervisor)
        grep -Fxq "$name" <(snapshot_allocation_hypervisors "$current") || return 1
        ! grep -Fxq "$name" <(snapshot_allocation_hypervisors "$runtime_after") || return 1
        ! grep -Fxq "$name" <(snapshot_allocation_hypervisors "$final_after") || return 1
        ;;
      cgroup|run-directory)
        grep -Fq "$name" "$current" || return 1
        ! grep -Fq "$name" "$runtime_after" || return 1
        ! grep -Fq "$name" "$final_after" || return 1
        ;;
      *)
        echo "E10 runner: unknown named-resource kind: $kind" >&2
        return 1
        ;;
    esac
  done <"$observed"

  recorded_count="$(sed -n -E \
    's/^E10 named cleanup: observed-before-stop=([0-9]+) post-stop=absent serve=stopped preparation=absent$/\1/p' \
    "$complement")"
  [[ "$recorded_count" =~ ^[0-9]+$ && "$recorded_count" -eq "$observed_count" ]]
}

validate_e10_capture() {
  local capture_root="$1"
  local ledger="$2"
  local expected_token="$3"
  local row_count
  row_count="$(awk -F '\t' 'NR > 1 { count++ } END { print count + 0 }' "$ledger")"
  [[ "$row_count" -eq 8 ]] || {
    echo "E10 runner: expected exactly eight ledger rows, observed $row_count" >&2
    return 1
  }

  local driver status cell expected_exit name
  local before_probe after_probe settled_probe
  local -a required=(
    capture-token.log case-transcript.log
    service-deploy-command.log service-deploy-stdout.log
    service-deploy-stderr.log service-deploy-exit.log service-stream.log
    before-stop-describe.log current-session-observation.log
    current-session-resources.log current-session-owned-resources.log
    post-runtime-cleanup-resources.log stop-command.log stop-stdout.log
    stop-stderr.log stop-exit.log after-stop-describe.log
    post-cleanup-describe.log stale-session-refusal.log
    cleanup-resources-before.log cleanup-resources-after.log
    cleanup-resource-delta.log cleanup-resource-complement.log
  )
  for driver in exec vm; do
    for status in 204 302 404 503; do
      cell="$capture_root/e10-$driver-$status"
      for name in "${required[@]}"; do
        require_file "$cell/$name" || return 1
      done
      [[ "$(tr -d '\r\n' <"$cell/capture-token.log")" == "$expected_token" ]] || {
        echo "E10 runner: stale raw evidence token for $driver/$status" >&2
        return 1
      }
      grep -Fq ' deploy ' "$cell/service-deploy-command.log" || return 1
      grep -Fq ' job stop ' "$cell/stop-command.log" || return 1
      [[ "$(tr -d '[:space:]' <"$cell/stop-exit.log")" == 0 ]] || return 1
      grep -Fq "Stopped workload 'service-$driver-http-$status'." \
        "$cell/stop-stdout.log" || return 1
      [[ "$(first_alloc_row "$cell/before-stop-describe.log")" \
        == "$(grep '^current_row=' "$cell/current-session-observation.log" \
          | sed 's/^current_row=//')" ]] || return 1
      grep -Eq '^allocation_id=alloc-service-(exec|vm)-http-(204|302|404|503)-0$' \
        "$cell/current-session-observation.log" || return 1
      validate_e10_named_cleanup "$cell" "$driver" "$status" || return 1
      grep -Fq 'Terminated' "$cell/after-stop-describe.log" || return 1
      grep -Fq '    reason: stopped' "$cell/after-stop-describe.log" || return 1
      grep -Fq 'Terminated' "$cell/post-cleanup-describe.log" || return 1
      grep -Fq '    reason: stopped' "$cell/post-cleanup-describe.log" || return 1
      [[ "$(awk '$1 ~ /^alloc-/ { print $1, $2, $3; exit }' \
        "$cell/after-stop-describe.log")" == \
        "$(awk '$1 ~ /^alloc-/ { print $1, $2, $3; exit }' \
        "$cell/post-cleanup-describe.log")" ]] || return 1
      expected_exit=1
      if [[ "$status" == 204 ]]; then
        expected_exit=0
        grep -Fq "Service 'service-$driver-http-204' is stable" \
          "$cell/service-deploy-stdout.log" || return 1
        grep -Eq '^current_state=Running$' "$cell/current-session-observation.log" || return 1
        grep -Eq '^restart_count=0$' "$cell/current-session-observation.log" || return 1
      else
        require_file "$cell/recovery-observations.log" || return 1
        require_file "$cell/stop-observations.log" || return 1
        grep -Fq 'Error:' "$cell/service-deploy-stdout.log" || return 1
        grep -Fq "HTTP $status" "$cell/service-deploy-stdout.log" || return 1
        grep -Eq '^current_state=Running$' "$cell/current-session-observation.log" || return 1
        grep -Eq '^restart_count=[1-9][0-9]*$' "$cell/current-session-observation.log" || return 1
        grep -Eq '^prior_failure=.*Failed' "$cell/current-session-observation.log" || return 1
        grep -Eq "^startup_probe=.*HTTP $status" \
          "$cell/current-session-observation.log" || return 1
        grep -Fq 'before_stop_prior_failure=' "$cell/stale-session-refusal.log" || return 1
        grep -Fq 'after_cleanup_prior_failure=' "$cell/stale-session-refusal.log" || return 1
        [[ "$(awk '/^    last terminated: / { print; exit }' \
          "$cell/before-stop-describe.log")" == \
          "$(awk '/^    last terminated: / { print; exit }' \
          "$cell/post-cleanup-describe.log")" ]] || return 1
        before_probe="$(awk '/^  startup probe\[0\] / { print; exit }' \
          "$cell/before-stop-describe.log")"
        after_probe="$(awk '/^  startup probe\[0\] / { print; exit }' \
          "$cell/after-stop-describe.log")"
        settled_probe="$(awk '/^  startup probe\[0\] / { print; exit }' \
          "$cell/post-cleanup-describe.log")"
        e10_failed_probe_is_preserved "$before_probe" "$after_probe" || {
          echo "E10 runner: failed probe changed or regressed after stop for $driver/$status" >&2
          return 1
        }
        e10_failed_probe_is_preserved "$after_probe" "$settled_probe" || {
          echo "E10 runner: failed probe changed or regressed after cleanup for $driver/$status" >&2
          return 1
        }
      fi
      [[ "$(tr -d '[:space:]' <"$cell/service-deploy-exit.log")" \
        == "$expected_exit" ]] || {
        echo "E10 runner: deploy exit mismatch for $driver/$status" >&2
        return 1
      }
      if grep -Fq 'SVM-E10-FAILURE-BODY-MUST-NOT-LEAK' \
        "$cell/service-deploy-stdout.log" "$cell/service-deploy-stderr.log" \
        "$cell/before-stop-describe.log" "$cell/after-stop-describe.log" \
        "$cell/post-cleanup-describe.log"; then
        echo "E10 runner: failure-body sentinel leaked for $driver/$status" >&2
        return 1
      fi
    done
  done

  awk -F '\t' '
    NR == 1 { next }
    NF != 20 || ($1 != "exec" && $1 != "vm") ||
      $2 !~ /^(204|302|404|503)$/ ||
      $4 != ("alloc-service-" $1 "-http-" $2 "-0") ||
      $6 !~ /^[0-9]+$/ || $9 !~ /^[0-9]+$/ ||
      ($2 == "204" && ($3 != "0" || $5 != "Running" || $6 != "0" ||
          $7 != "none" || $8 != "Terminated" || $9 != "0" ||
          $10 != "Terminated" || $11 != "operator-stop-running")) ||
      ($2 != "204" && ($3 != "1" || $5 != "Running" ||
          $6 !~ /^[1-9][0-9]*$/ ||
          $7 != "deploy-exit-1+prior-failed+probe" ||
          $8 != "Terminated" || $9 != $6 || $10 != "Terminated" ||
          $11 != "recovered-running-operator-stop")) ||
      $12 != ("status-" $2) || $13 !~ /^[0-9]+$/ || $14 !~ /^[0-9]+$/ ||
      $15 !~ /^[0-9]+$/ || $16 != "0" || $17 != "0" ||
      $18 != "0" || $19 != "0" || $20 != "named-absent" { exit 1 }
    { seen[$1 SUBSEP $2]++; rows++ }
    END {
      if (rows != 8) exit 1
      for (driver in drivers) for (status in statuses)
        if (seen[driver SUBSEP status] != 1) exit 1
    }
    BEGIN {
      drivers["exec"] = 1; drivers["vm"] = 1
      statuses["204"] = 1; statuses["302"] = 1
      statuses["404"] = 1; statuses["503"] = 1
    }
  ' "$ledger" || {
    echo 'E10 runner: raw-backed ledger oracle failed' >&2
    return 1
  }
}

if [[ "${E10_VALIDATE_ONLY:-0}" == 1 ]]; then
  validate_e10_capture "${E10_CAPTURE_ROOT:?}" "${E10_LEDGER:?}" \
    "${E10_CAPTURE_TOKEN:?}"
  exit
fi

if [[ "${E10_VALIDATE_RESOURCE_ONLY:-0}" == 1 ]]; then
  validate_e10_named_cleanup "${E10_CELL:?}" \
    "${E10_DRIVER:?}" "${E10_STATUS:?}"
  exit
fi

if [[ "${E10_VALIDATE_PROBE_ONLY:-0}" == 1 ]]; then
  e10_failed_probe_is_preserved "${E10_PROBE_BEFORE:?}" \
    "${E10_PROBE_AFTER:?}"
  exit
fi

"$REPO_ROOT/$EXAMPLE" check-source
TARGET="$(resolve_metal_target)"
readonly TARGET
[[ -n "$TARGET" ]] || {
  echo 'E10 runner: OVERDRIVE_METAL_TARGET is not configured' >&2
  exit 1
}

{
  echo '# E10 invocation'
  echo "command: cargo xtask metal run -- $EXAMPLE run http-status-cross-driver"
  echo "remote_capture_root: $REMOTE_CAPTURE_ROOT"
  echo "capture_token: $CAPTURE_TOKEN"
  echo "started_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
} >"$EVIDENCE_DIR/product-run.meta"

set +e
timeout 1800s cargo xtask metal run -- bash -lc \
  "SVM_E08_MATRIX_CAPTURE_ROOT=$REMOTE_CAPTURE_ROOT SVM_E10_CAPTURE_TOKEN=$CAPTURE_TOKEN $EXAMPLE run http-status-cross-driver" \
  >"$RAW_OUTPUT" 2>&1
run_rc=$?
set -e
sed -E 's/@[A-Za-z0-9._-]+/@<metal-host-redacted>/g' \
  <"$RAW_OUTPUT" >"$EVIDENCE_DIR/product-run.out"
{
  echo "finished_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "exit: $run_rc"
} >>"$EVIDENCE_DIR/product-run.meta"

# Pull raw files after either a completed or partial run. The capture token
# prevents a prior remote file from satisfying this run's evidence oracle.
readonly -a SSH_OPTS=(-o StrictHostKeyChecking=accept-new -o ServerAliveInterval=30 \
  -o ConnectTimeout=15 -o BatchMode=yes)
install -d -m 0700 "$LOCAL_CAPTURE_ROOT"
set +e
rsync -az -e "ssh ${SSH_OPTS[*]}" \
  "$TARGET:overdrive/$REMOTE_CAPTURE_ROOT/" "$LOCAL_CAPTURE_ROOT/" \
  >"$RAW_PULL" 2>&1
pull_rc=$?
set -e
sed -E 's/@[A-Za-z0-9._-]+/@<metal-host-redacted>/g' \
  <"$RAW_PULL" >"$EVIDENCE_DIR/evidence-pullback.out"
printf 'exit: %s\n' "$pull_rc" >>"$EVIDENCE_DIR/evidence-pullback.out"
[[ "$pull_rc" -eq 0 ]] || {
  cat "$EVIDENCE_DIR/evidence-pullback.out" >&2
  exit "$pull_rc"
}

awk '/^--- E10 ledger begin ---$/ { keep=1; next }
  /^--- E10 ledger end ---$/ { keep=0 }
  keep' "$EVIDENCE_DIR/product-run.out" \
  >"$EVIDENCE_DIR/http-status-cross-driver.tsv"

[[ "$run_rc" -eq 0 ]] || {
  cat "$EVIDENCE_DIR/product-run.out" >&2
  exit "$run_rc"
}
validate_e10_capture "$LOCAL_CAPTURE_ROOT" \
  "$EVIDENCE_DIR/http-status-cross-driver.tsv" "$CAPTURE_TOKEN"
grep -Fq 'E10 PASS: 8/8 Exec/VM HTTP status cells with zero failure-body sentinel exposure' \
  "$EVIDENCE_DIR/product-run.out"
