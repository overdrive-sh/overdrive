#!/usr/bin/env bash
# run-expectation.sh — capture pinned, executed evidence for one expectation.
#
# Usage:  verification/harness/run-expectation.sh <ID>
#         SEED=42 verification/harness/run-expectation.sh <ID>
#
# Pins commit SHA + dirty state + DST seed + harness invocation, runs the
# expectation's runner.sh (real commands, in Lima), captures verbatim output
# to evidence/, validates the external anchor, writes evidence/verification.yaml.
#
# It does NOT fabricate evidence. No runner.sh -> status pending, manual
# capture required. A failing runner.sh is DATA (status candidate: partial/
# broken), not a harness error.
set -uo pipefail

ID="${1:-}"
if [[ -z "$ID" ]]; then
  echo "usage: $0 <EXPECTATION_ID>   (e.g. O01, E01)" >&2
  exit 2
fi

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VERIFICATION_DIR="$(cd "$HARNESS_DIR/.." && pwd)"
REPO_ROOT="$(git -C "$VERIFICATION_DIR" rev-parse --show-toplevel)"
export REPO_ROOT

# Resolve the expectation dir by ID prefix.
shopt -s nullglob
matches=("$VERIFICATION_DIR"/expectations/"$ID"-*/)
shopt -u nullglob
if [[ ${#matches[@]} -ne 1 ]]; then
  echo "error: expected exactly one expectations/${ID}-* dir, found ${#matches[@]}" >&2
  exit 2
fi
EXPECTATION_DIR="${matches[0]%/}"
export EXPECTATION_DIR
EVIDENCE_DIR="$EXPECTATION_DIR/evidence"
export EVIDENCE_DIR
mkdir -p "$EVIDENCE_DIR"

export SEED="${SEED:-1}"

# --- Pin everything (governing rule 2) ---------------------------------------
SHA="$(git -C "$REPO_ROOT" rev-parse HEAD)"
HARNESS_SHA="$(git -C "$REPO_ROOT" log -1 --format=%H -- "$VERIFICATION_DIR" 2>/dev/null || echo "uncommitted")"
DATE_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
DIRTY="false"
if [[ -n "$(git -C "$REPO_ROOT" status --porcelain)" ]]; then
  DIRTY="true"
  git -C "$REPO_ROOT" status --porcelain >"$EVIDENCE_DIR/dirty-status.txt"
  git -C "$REPO_ROOT" diff HEAD >"$EVIDENCE_DIR/dirty-diff.patch"
fi

echo "=== expectation $ID ==="
echo "  dir:     $EXPECTATION_DIR"
echo "  sha:     $SHA (dirty=$DIRTY)"
echo "  seed:    $SEED"

# --- Validate external anchor (governing rule 3) -----------------------------
# Anchors are declared in the expectation README as lines: `- Anchor: <ref>`.
ANCHOR_STATUS="present"
mapfile -t ANCHORS < <(grep -E '^- Anchor:' "$EXPECTATION_DIR/README.md" 2>/dev/null | sed 's/^- Anchor:[[:space:]]*//')
if [[ ${#ANCHORS[@]} -eq 0 ]]; then
  ANCHOR_STATUS="MISSING -> unanchored-claim"
  echo "  anchor:  NONE — claim will be tagged unanchored-claim"
else
  printf '  anchor:  %s\n' "${ANCHORS[@]}"
fi

# --- Execute (governing rule 1: executed, not narrated) ----------------------
RUNNER="$EXPECTATION_DIR/runner.sh"
RUNNER_RC="n/a"
EXECUTED="false"
if [[ -x "$RUNNER" ]]; then
  EXECUTED="true"
  echo "  --- runner.sh output ---"
  # Do not let set -e abort on a failing runner; the failure is evidence.
  ( cd "$REPO_ROOT" && SEED="$SEED" \
      EVIDENCE_DIR="$EVIDENCE_DIR" EXPECTATION_DIR="$EXPECTATION_DIR" \
      REPO_ROOT="$REPO_ROOT" \
      bash "$RUNNER" ) 2>&1 | tee "$EVIDENCE_DIR/run.log"
  RUNNER_RC="${PIPESTATUS[0]}"
  echo "  --- runner.sh exit: $RUNNER_RC ---"
else
  echo "  runner.sh: ABSENT — status stays 'pending', manual evidence required"
fi

# --- Manifest ----------------------------------------------------------------
cat >"$EVIDENCE_DIR/verification.yaml" <<YAML
expectation_id: "$ID"
date_utc: "$DATE_UTC"
overdrive_sha: "$SHA"
working_tree_dirty: $DIRTY
dst_seed: $SEED
harness_sha: "$HARNESS_SHA"
harness_invocation: "verification/harness/run-expectation.sh $ID"
executed_in_lima: $EXECUTED
runner_exit_code: "$RUNNER_RC"
anchors: [$(printf '"%s",' "${ANCHORS[@]}" | sed 's/,$//')]
anchor_status: "$ANCHOR_STATUS"
# NOTE: 'satisfied' is a HUMAN/auditor verdict written into README.md after
# adversarial review of the captured evidence below — never auto-stamped here.
YAML

echo "  manifest: $EVIDENCE_DIR/verification.yaml"
echo "=== done ($ID) — review evidence/ adversarially, then set status in README.md ==="
[[ "$RUNNER_RC" == "0" || "$RUNNER_RC" == "n/a" ]]
