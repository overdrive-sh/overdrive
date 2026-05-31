# shellcheck shell=bash
# Helpers sourced by per-expectation runner.sh scripts.
#
# Black-box rule: these wrap the *built* overdrive binary running on a real
# kernel via Lima. Nothing here imports an overdrive-* crate or reaches into
# crate internals — the only surface is the CLI and what the kernel exposes.

# Run the overdrive CLI inside Lima. The `cargo overdrive` alias already
# routes through `cargo xtask lima run -- cargo run -p overdrive-cli ...`,
# so the binary executes against the real kernel + cgroup v2, as CI does.
od() {
  ( cd "$REPO_ROOT" && cargo overdrive "$@" )
}

# Run an arbitrary command inside Lima (real kernel). Use for kernel-surface
# probes: bpftool map dump, ss -K, tcpdump, /proc inspection.
in_lima() {
  ( cd "$REPO_ROOT" && cargo xtask lima run -- "$@" )
}

# Capture a labelled command's verbatim stdout+stderr and exit code into the
# evidence dir. Returns the command's real exit code (a failure is DATA, not
# a harness error) so runner.sh can branch on it.
#   capture <label> <cmd> [args...]
capture() {
  local label="$1"; shift
  local out="$EVIDENCE_DIR/${label}.out"
  local meta="$EVIDENCE_DIR/${label}.meta"
  local rc=0
  {
    echo "# command: $*"
    echo "# seed:    ${SEED}"
    echo "# started: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  } >"$meta"
  "$@" >"$out" 2>&1 || rc=$?
  echo "# exit:    $rc" >>"$meta"
  echo "  [capture] $label -> ${label}.out (exit $rc)"
  return "$rc"
}

# Assert a captured output file contains a literal string. Records the verdict;
# does not abort the run (so every sub-claim is captured even when one fails).
#   evidence_contains <label> <literal>
evidence_contains() {
  local label="$1" needle="$2"
  if grep -qF -- "$needle" "$EVIDENCE_DIR/${label}.out"; then
    echo "  [PASS] '${label}.out' contains: $needle"
    return 0
  fi
  echo "  [FAIL] '${label}.out' missing: $needle"
  return 1
}

# Assert a captured output file does NOT contain a literal string. The RCA-A
# guard ("never claim (took live) for an exited Service") is exactly this shape.
#   evidence_absent <label> <literal>
evidence_absent() {
  local label="$1" needle="$2"
  if grep -qF -- "$needle" "$EVIDENCE_DIR/${label}.out"; then
    echo "  [FAIL] '${label}.out' unexpectedly contains: $needle"
    return 1
  fi
  echo "  [PASS] '${label}.out' does not contain: $needle"
  return 0
}
