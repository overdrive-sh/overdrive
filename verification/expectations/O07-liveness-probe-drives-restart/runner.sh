# shellcheck shell=bash
# O07 — a declared liveness probe reaches the reconciler's restart decision.
#
# ADR-0080 Stage 1 verification. Three Services differing in ONE input each,
# deployed against ONE ephemeral control plane, observed over a single window:
#
#   examples/liveness-absent-service.toml : no liveness probe          -> expect Restarts 0
#   examples/liveness-fails-service.toml  : liveness -> unbound port    -> expect Restarts > 0
#   examples/liveness-holds-service.toml  : liveness -> its OWN listener-> expect Restarts 0
#
# The baseline attributes churn to the liveness DECLARATION; the control
# attributes it to the liveness OUTCOME (`.claude/rules/debugging.md` § 5).
#
# BLACK-BOX ONLY. The surface is the built `overdrive` binary (CLI) and what the
# kernel exposes (`ip`, `ss`, `bpftool`, cgroupfs). No overdrive-* crate is
# linked, imported, or reached into — per `.claude/rules/verification.md`
# ("a runner.sh that imports or links an overdrive-* crate has become a fifth
# test tier and forfeited the independence that makes the evidence worth
# trusting").
#
# Bring-up model follows O03: ONE root-context Lima invocation owning serve +
# deploys + capture + teardown, with an EXIT trap so the sweep fires on every
# exit path. Leaked XDP on `lo` hangs the loopback for the user's OTHER
# Conductor workspaces sharing this VM, and leaked workload cgroups break later
# tests with a misleading EEXIST (`.claude/rules/{testing,debugging}.md`). This
# runner therefore sweeps BOTH before and after, and records both probes as
# evidence. `liveness-fails` restarts continuously by design, so its cgroup
# scope churns throughout the run — the teardown sweep is not optional here.
#
# KEK: `overdrive serve` boots the persistent workload-identity CA, which needs
# a key-encryption key. This runner supplies it through the PRODUCTION
# systemd-creds delivery path — `$CREDENTIALS_DIRECTORY/overdrive-ca-root` —
# NOT a test seam (no `SimKek`, no `with_credentials_dir`, no
# `OVERDRIVE_CA_KEK` dev fallback, which the production posture refuses without
# an explicit opt-in anyway).
#
# OBSERVATION WINDOW — two snapshots, and the reason for both:
#
#   T+20s  PRIMARY. Sub-claims 2-4 are evaluated here. All three allocations are
#          still `Running`, so the baseline's `Restarts 0` cannot be dismissed as
#          "terminal allocations just are not restarted" — the contrast is
#          between three RUNNING Services, and only the two that declare a
#          liveness probe churn.
#   T+40s  CONTEXT ONLY, not gated. By this point the baseline has been driven to
#          `Failed`: on this composition a TCP startup probe cannot reach a
#          netns-isolated workload either, so the startup budget eventually
#          exhausts. The two liveness-churning Services do NOT reach `Failed`,
#          because each restart resets their startup window. Capturing this
#          second snapshot keeps the window choice honest and auditable rather
#          than hiding the cliff that motivated it.
source "$REPO_ROOT/verification/harness/lima-helpers.sh"

ABSENT_SPEC="examples/liveness-absent-service.toml"
FAILS_SPEC="examples/liveness-fails-service.toml"
HOLDS_SPEC="examples/liveness-holds-service.toml"

for s in "$ABSENT_SPEC" "$FAILS_SPEC" "$HOLDS_SPEC"; do
  if [[ ! -f "$REPO_ROOT/$s" ]]; then
    echo "  [pending] fixture missing: $s"
    exit 0
  fi
done

# Seconds from deploy to the PRIMARY snapshot that gates sub-claims 2-4. 20s is
# comfortably inside the measured window in which the baseline is still
# `Running` (see the header), which is the property this value actually needs.
#
# CORRECTED 2026-08-02 — an earlier version of this comment justified 20s as
# ">> the ~4s liveness trip time (interval 2s x failure_threshold 2)". That
# arithmetic is WRONG, and THIS RUNNER'S OWN CAPTURED EVIDENCE refutes it: 38
# restarts by T+20s and 82 by T+40s is a cadence of ~0.46 s/restart, roughly 9x
# faster than the comment predicted. The probe INTERVAL does not set the
# cadence at all.
#
# Root cause (post-dates this capture — the RCA was written after the run and
# cites this expectation's README, so O07 could not have referenced it
# originally; it is cross-referenced here only now):
# `docs/analysis/root-cause-analysis-probe-runner-exec-inert-and-ungated-restart-loop.md`
# § 1 — the consecutive-failure streak advances once per RECONCILER EVALUATION
# on which the latest liveness row reads `Fail`, not once per probe execution.
# Measured cadence ~= failure_threshold x T where T ~= 0.239 s is the
# evaluation period, giving 2 x 0.239 ~= 0.48 s. The RCA independently
# reproduces the same fixture family at a 0.460 s median gap.
#
# The over-generous window did NOT invalidate sub-claims 2-4: a threshold that
# trips ~9x sooner than assumed still leaves the baseline `Running` at T+20s,
# which is the confound guard the sub-claims actually depend on.
OBSERVE_PRIMARY_S=20
# Seconds from deploy to the ungated CONTEXT snapshot, past the point at which
# the baseline's unreachable startup probe exhausts its budget.
OBSERVE_CONTEXT_S=40

INNER=$(cat <<INNER_EOF
set -uo pipefail
EVID="$EVIDENCE_DIR"
REPO="$REPO_ROOT"
SEED="$SEED"
OBSERVE_PRIMARY_S="$OBSERVE_PRIMARY_S"
OBSERVE_CONTEXT_S="$OBSERVE_CONTEXT_S"
ABSENT_SPEC="$REPO_ROOT/$ABSENT_SPEC"
FAILS_SPEC="$REPO_ROOT/$FAILS_SPEC"
HOLDS_SPEC="$REPO_ROOT/$HOLDS_SPEC"
cd "\$REPO"

BIND="127.0.0.1:7443"
LOOPBACK_PROBE_PORT="1"   # nothing listens here; a healthy loopback refuses fast
CFG_DIR="\$(mktemp -d /tmp/od-o07-cfg.XXXXXX)"
DATA_DIR="\$(mktemp -d /tmp/od-o07-data.XXXXXX)"
CREDS_DIR="\$(mktemp -d /tmp/od-o07-creds.XXXXXX)"
SERVE_PID=""

# ---- kernel-surface probe helpers (black-box; ip/bpftool/cgroupfs) ----------
probe_xdp() {
  for i in \$(ip -br link show | awk '{print \$1}'); do
    info="\$(ip link show "\$i")"
    case "\$info" in
      *xdpgeneric*|*xdpdrv*|*xdp\ *)
        echo "=== \$i ==="
        echo "\$info" | grep -E 'xdp(generic|drv)?'
        ;;
    esac
  done
  echo "--- bpftool prog show (xdp/sched_cls only) ---"
  bpftool prog show 2>/dev/null | grep -E '(xdp|sched_cls)' || echo "(none)"
}
probe_loopback() {
  if timeout 3 bash -c "echo > /dev/tcp/127.0.0.1/\$LOOPBACK_PROBE_PORT" 2>/dev/null; then
    echo "UNEXPECTED: 127.0.0.1:\$LOOPBACK_PROBE_PORT accepted (something is listening?)"
  else
    rc=\$?
    if [ "\$rc" -eq 124 ]; then
      echo "HANG: loopback connect timed out after 3s -> probable leaked XDP on lo"
    else
      echo "HEALTHY: loopback refused fast (no leaked XDP swallowing lo traffic)"
    fi
  fi
}
probe_cgroups() {
  ls /sys/fs/cgroup/overdrive.slice/workloads.slice/ 2>/dev/null | grep '^alloc-' || echo "(no alloc-*.scope)"
}

# ---- teardown sweep: fires on EVERY exit path, then a post-probe ------------
sweep() {
  [ -n "\$SERVE_PID" ] && kill "\$SERVE_PID" 2>/dev/null
  # cgroup mass-kill + rmdir (testing.md "Leaked workload cgroups"). cgroup.kill
  # SIGKILLs every PID inside each workload scope, including the socat backends.
  # Deliberately NO broad \`pgrep -f socat\`: this runner's own argv carries the
  # literal socat command string from the fixtures, so a \`pgrep -f\` on it would
  # match (and kill) this very shell. cgroup.kill is the scoped primitive.
  if cd /sys/fs/cgroup/overdrive.slice/workloads.slice 2>/dev/null; then
    for d in alloc-*.scope; do
      [ -d "\$d" ] && { echo 1 > "\$d/cgroup.kill" 2>/dev/null; rmdir "\$d" 2>/dev/null; }
    done
    cd "\$REPO" 2>/dev/null || true
  fi
  # XDP detach (debugging.md "Leftover XDP attachments")
  for i in \$(ip -br link show | awk '{print \$1}'); do
    ip link set dev "\$i" xdpgeneric off 2>/dev/null
    ip link set dev "\$i" xdpdrv off 2>/dev/null
    ip link set dev "\$i" xdp off 2>/dev/null
  done
  # Per-allocation network namespaces. The mTLS gate creates one
  # \`ovd-ns-NNNN\` per alloc on the production path, and this expectation
  # deliberately churns allocations, so it leaks them faster than anything else
  # in the catalogue. Same discipline as the cgroup and XDP sweeps: an orphan
  # here is state the next run inherits.
  #
  # ONLY empty namespaces are removed. This VM is shared across Conductor
  # workspaces, so a namespace still holding a PID may belong to somebody
  # else's live \`serve\` — deleting it would break their run. \`ip netns pids\`
  # is the guard.
  for ns in \$(ip netns list 2>/dev/null | awk '{print \$1}'); do
    case "\$ns" in
      ovd-ns-*)
        if [ -z "\$(ip netns pids "\$ns" 2>/dev/null)" ]; then
          ip netns delete "\$ns" 2>/dev/null
        fi
        ;;
    esac
  done
  return 0
}
on_exit() {
  sweep
  { echo "# probe: XDP attachments POST-TEARDOWN (after sweep)"; probe_xdp; }      > "\$EVID/probe_post_teardown_xdp.txt"      2>&1
  { echo "# probe: loopback POST-TEARDOWN (after sweep)";        probe_loopback; } > "\$EVID/probe_post_teardown_loopback.txt" 2>&1
  { echo "# probe: workload cgroups POST-TEARDOWN (after sweep)"; probe_cgroups; } > "\$EVID/probe_post_teardown_cgroups.txt"  2>&1
  {
    echo "# probe: per-alloc network namespaces POST-TEARDOWN (after sweep)"
    ip netns list 2>/dev/null | grep '^ovd-ns-' || echo "(no ovd-ns-* namespaces)"
  } > "\$EVID/probe_post_teardown_netns.txt" 2>&1
  return 0
}
trap on_exit EXIT

# ---- BEFORE probes (proof of clean start) -----------------------------------
{ echo "# probe: XDP attachments BEFORE run"; probe_xdp; }       > "\$EVID/probe_before_xdp.txt"      2>&1
{ echo "# probe: loopback BEFORE run";        probe_loopback; }  > "\$EVID/probe_before_loopback.txt" 2>&1
{ echo "# probe: workload cgroups BEFORE run"; probe_cgroups; }  > "\$EVID/probe_before_cgroups.txt"  2>&1
# Stale alloc scopes from a prior interrupted run make a fresh alloc fail EEXIST
# and read as a regression in whatever is under audit. Clean before serving.
if ! grep -q '(no alloc' "\$EVID/probe_before_cgroups.txt" \
   || grep -q 'HANG' "\$EVID/probe_before_loopback.txt"; then
  echo "PRE-EXISTING LEAK detected at runner start; cleaning before serve." >> "\$EVID/probe_before_cgroups.txt"
  sweep
fi

# ---- compile the binary once so serve + deploys share one build -------------
echo "# building the overdrive binary (single compile shared by serve+deploy+describe)"
if ! cargo build -q -p overdrive-cli --bin overdrive 2> "\$EVID/build.log"; then
  echo "BUILD_FAILED"; tail -40 "\$EVID/build.log"
  echo "INNER_DONE serve_status=build-failed"
  exit 0
fi
BIN="\$CARGO_TARGET_DIR/debug/overdrive"
[ -x "\$BIN" ] || { echo "BIN_MISSING: \$BIN"; echo "INNER_DONE serve_status=bin-missing"; exit 0; }

# ---- KEK via the PRODUCTION systemd-creds delivery path ---------------------
# \`SystemdCredsKeyring\` reads \$CREDENTIALS_DIRECTORY/<kek-id>; the root KEK id
# is \`overdrive-ca-root\`. Any byte string is accepted (folded to 256 bits when
# it is not already exactly 32 bytes), so a passphrase file is a legitimate
# production-shaped delivery — not a test seam.
printf 'o07-verification-kek-not-a-production-secret' > "\$CREDS_DIR/overdrive-ca-root"

# ---- background the ephemeral serve -----------------------------------------
echo "# starting ephemeral serve: bind=\$BIND cfg=\$CFG_DIR data=\$DATA_DIR creds=\$CREDS_DIR"
CREDENTIALS_DIRECTORY="\$CREDS_DIR" OVERDRIVE_CONFIG_DIR="\$CFG_DIR" \
  "\$BIN" serve --bind "\$BIND" --data-dir "\$DATA_DIR" > "\$EVID/serve.log" 2>&1 &
SERVE_PID=\$!

# serve writes the trust-triple config AFTER it binds the TLS listener.
CFG_FILE="\$CFG_DIR/.overdrive/config"
ready=0
for _ in \$(seq 1 60); do
  if [ -f "\$CFG_FILE" ]; then ready=1; break; fi
  if ! kill -0 "\$SERVE_PID" 2>/dev/null; then break; fi
  sleep 0.5
done
if [ "\$ready" -ne 1 ]; then
  echo "SERVE_NOT_READY: trust-triple config never appeared at \$CFG_FILE"
  echo "--- serve.log tail ---"; tail -30 "\$EVID/serve.log"
  echo "INNER_DONE serve_status=not-ready"
  exit 0   # the on_exit trap writes the post-teardown no-leak proof
fi
echo "# serve ready: trust triple written at \$CFG_FILE (pid \$SERVE_PID)"

# ---- deploy all three, capturing each accept render verbatim ----------------
deploy_one() {
  label="\$1"; spec="\$2"
  {
    echo "# command: overdrive deploy \$spec --detach"
    echo "# seed:    \$SEED"
    echo "# started: \$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  } > "\$EVID/deploy_\$label.meta"
  rc=0
  OVERDRIVE_CONFIG_DIR="\$CFG_DIR" "\$BIN" deploy "\$spec" --detach \
    > "\$EVID/deploy_\$label.out" 2>&1 || rc=\$?
  echo "# exit:    \$rc" >> "\$EVID/deploy_\$label.meta"
  echo "# deploy \$label exit: \$rc"
}
deploy_one absent "\$ABSENT_SPEC"
deploy_one fails  "\$FAILS_SPEC"
deploy_one holds  "\$HOLDS_SPEC"

# ---- namespace-reachability measurement (the diagnostic for the control) ----
# Recorded BEFORE the long observation sleep, while the first generation of
# backends is still up. Pure kernel surface (ip / ss / /dev/tcp) — this is what
# a probe runner in the control plane's namespace can and cannot see.
sleep 8
{
  echo "# What can a connect from the CONTROL PLANE's namespace reach?"
  echo "# Each fixture's workload binds 0.0.0.0:<its listener port> — 8091 / 8092 / 8093."
  echo
  echo "=== ip netns list ==="
  ip netns list 2>/dev/null || echo "(none)"
  echo
  echo "=== host-namespace TCP listeners on the fixture ports ==="
  ss -tlnp 2>/dev/null | grep -E ':(8091|8092|8093|9192) ' || echo "(none of 8091/8092/8093/9192 listening in the host namespace)"
  echo
  for ns in \$(ip netns list 2>/dev/null | awk '{print \$1}'); do
    echo "=== netns \$ns TCP listeners ==="
    ip netns exec "\$ns" ss -tlnp 2>/dev/null | grep -E ':(8091|8092|8093) ' || echo "  (none)"
  done
  echo
  echo "=== host-namespace connect attempts (what a TCP probe would do) ==="
  for p in 8091 8092 8093 9192; do
    if timeout 3 bash -c "echo > /dev/tcp/0.0.0.0/\$p" 2>/dev/null; then
      echo "  0.0.0.0:\$p CONNECTED"
    else
      echo "  0.0.0.0:\$p REFUSED-OR-UNREACHABLE"
    fi
  done
} > "\$EVID/namespace_reachability.txt" 2>&1

# ---- observe ----------------------------------------------------------------
# The 8s reachability measurement above already consumed part of the window;
# subtract it so the snapshot timestamps mean what they say.
snapshot() {
  suffix="\$1"
  for pair in "absent:liveness-absent" "fails:liveness-fails" "holds:liveness-holds"; do
    label="\${pair%%:*}"; wl="\${pair##*:}"
    rc=0
    OVERDRIVE_CONFIG_DIR="\$CFG_DIR" "\$BIN" workload describe "\$wl" \
      > "\$EVID/describe_\$label\$suffix.out" 2>&1 || rc=\$?
    echo "# workload describe \$wl (\$suffix) exit: \$rc"
  done
}

echo "# PRIMARY snapshot at T+\${OBSERVE_PRIMARY_S}s (gates sub-claims 2-4)"
sleep \$(( OBSERVE_PRIMARY_S - 8 ))
snapshot ""

echo "# CONTEXT snapshot at T+\${OBSERVE_CONTEXT_S}s (ungated; shows the startup cliff)"
sleep \$(( OBSERVE_CONTEXT_S - OBSERVE_PRIMARY_S ))
snapshot "_t\${OBSERVE_CONTEXT_S}s"

# ---- AFTER probes (steady state the run produced, pre-teardown) -------------
{ echo "# probe: XDP attachments AFTER observation (pre-teardown)"; probe_xdp; } > "\$EVID/probe_after_xdp.txt"      2>&1
{ echo "# probe: loopback AFTER observation";  probe_loopback; }                 > "\$EVID/probe_after_loopback.txt" 2>&1
{ echo "# probe: workload cgroups AFTER observation"; probe_cgroups; }           > "\$EVID/probe_after_cgroups.txt"  2>&1

# Keep the serve log bounded: liveness-fails restarts continuously, so the
# recovery lines dominate. Preserve the head (boot + first transitions) and the
# tail (steady state) rather than a multi-MB middle.
if [ -f "\$EVID/serve.log" ]; then
  {
    echo "# serve.log HEAD (boot + first transitions) — ANSI stripped"
    sed -e 's/\x1b\[[0-9;]*m//g' "\$EVID/serve.log" | head -60
    echo
    echo "# ... (\$(wc -l < "\$EVID/serve.log") total lines) ..."
    echo
    echo "# serve.log TAIL (steady state) — ANSI stripped"
    sed -e 's/\x1b\[[0-9;]*m//g' "\$EVID/serve.log" | tail -40
  } > "\$EVID/serve_log_excerpt.txt" 2>&1
  # Restart-recovery lines are the platform's own account of the churn.
  sed -e 's/\x1b\[[0-9;]*m//g' "\$EVID/serve.log" \
    | grep -c 'recovered from a terminal observation' \
    > "\$EVID/serve_restart_recovery_count.txt" 2>/dev/null || echo 0 > "\$EVID/serve_restart_recovery_count.txt"
fi

kill "\$SERVE_PID" 2>/dev/null
wait "\$SERVE_PID" 2>/dev/null
echo "INNER_DONE serve_status=ready"
exit 0
INNER_EOF
)

in_lima bash -c "$INNER" > "$EVIDENCE_DIR/serve_deploy.out" 2>&1 || true

echo "  --- serve_deploy.out (tail) ---"
tail -20 "$EVIDENCE_DIR/serve_deploy.out" || true

# ---- No-leak gate (HARD) ----------------------------------------------------
leak_rc=0
for p in probe_before_loopback probe_post_teardown_loopback; do
  if [[ -f "$EVIDENCE_DIR/$p.txt" ]] && grep -q 'HEALTHY' "$EVIDENCE_DIR/$p.txt"; then
    echo "  [PASS] no-leak: $p shows HEALTHY loopback"
  else
    echo "  [FAIL] no-leak: $p is not HEALTHY — possible leaked XDP on lo"
    leak_rc=1
  fi
done
if grep -q '(no alloc' "$EVIDENCE_DIR/probe_post_teardown_cgroups.txt" 2>/dev/null; then
  echo "  [PASS] no-leak: post-teardown shows no alloc-*.scope left behind"
else
  echo "  [FAIL] no-leak: post-teardown left workload cgroups behind"
  leak_rc=1
fi
if grep -q '(no ovd-ns' "$EVIDENCE_DIR/probe_post_teardown_netns.txt" 2>/dev/null; then
  echo "  [PASS] no-leak: post-teardown shows no ovd-ns-* namespaces left behind"
else
  echo "  [WARN] no-leak: ovd-ns-* namespaces remain — inspect probe_post_teardown_netns.txt (a namespace still holding a PID is deliberately NOT reclaimed; it may belong to another workspace's live serve)"
fi

serve_status="$(sed -n 's/.*serve_status=\([a-z-]*\).*/\1/p' "$EVIDENCE_DIR/serve_deploy.out" | tail -1)"
if [[ "$serve_status" != "ready" ]]; then
  echo "  [pending] black-box serve did not bind (serve_status='${serve_status:-unknown}')."
  echo "            Inspect evidence/serve.log for the cause. Sub-claims stay 'pending'."
  exit "$leak_rc"
fi

# ---- Sub-claims -------------------------------------------------------------
rc=0

# Sub-claim 1: all three deploys accepted.
for label in absent fails holds; do
  d_rc="$(sed -n 's/^# exit:[[:space:]]*//p' "$EVIDENCE_DIR/deploy_$label.meta" 2>/dev/null)"
  if [[ "$d_rc" == "0" ]]; then
    echo "  [PASS] sub-claim 1: deploy $label exited 0"
  else
    echo "  [FAIL] sub-claim 1: deploy $label exited '${d_rc:-<none>}'"
    rc=1
  fi
  evidence_contains "deploy_$label" "Accepted." || rc=1
done

# `Restarts` is the 3rd column of the Service per-alloc table
# (`Alloc / State / Restarts / Since`). Read it off the alloc row.
restarts_of() {
  awk '/^alloc-/ { print $3; exit }' "$EVIDENCE_DIR/describe_$1.out" 2>/dev/null
}
state_of() {
  awk '/^alloc-/ { print $2; exit }' "$EVIDENCE_DIR/describe_$1.out" 2>/dev/null
}

r_absent="$(restarts_of absent)"; s_absent="$(state_of absent)"
r_fails="$(restarts_of fails)";   s_fails="$(state_of fails)"
r_holds="$(restarts_of holds)";   s_holds="$(state_of holds)"
echo "  [observed] absent: state=${s_absent:-<none>} restarts=${r_absent:-<none>}"
echo "  [observed] fails:  state=${s_fails:-<none>} restarts=${r_fails:-<none>}"
echo "  [observed] holds:  state=${s_holds:-<none>} restarts=${r_holds:-<none>}"

# Confound guard for sub-claim 2: the baseline's `Restarts 0` is only
# attributable to the ABSENT liveness probe if the allocation is still Running.
# A terminal allocation might be excluded from restart for an unrelated reason,
# which would make the whole diff unreadable.
if [[ "$s_absent" == "Running" ]]; then
  echo "  [PASS] confound guard: baseline is still Running at the primary snapshot"
else
  echo "  [FAIL] confound guard: baseline state is '${s_absent:-<none>}', not Running — its Restarts 0 may be an artifact of terminality rather than of the absent liveness probe; sub-claim 2 is not attributable"
  rc=1
fi

# Sub-claim 2 (BASELINE): no liveness probe declared -> not restarted.
if [[ "$r_absent" == "0" ]]; then
  echo "  [PASS] sub-claim 2: liveness-absent was NOT restarted (Restarts 0)"
else
  echo "  [FAIL] sub-claim 2: liveness-absent shows Restarts='${r_absent:-<none>}' (expected 0) — the baseline does not hold, so nothing below is attributable to liveness"
  rc=1
fi

# Sub-claim 3 (POSITIVE): a declared, failing liveness probe -> restarted.
if [[ -n "$r_fails" && "$r_fails" =~ ^[0-9]+$ && "$r_fails" -gt 0 ]]; then
  echo "  [PASS] sub-claim 3: liveness-fails WAS restarted (Restarts $r_fails > 0)"
else
  echo "  [FAIL] sub-claim 3: liveness-fails shows Restarts='${r_fails:-<none>}' (expected > 0) — the liveness observation is not reaching the restart decision"
  rc=1
fi

# Sub-claim 4 (CONTROL): a declared liveness probe targeting the workload's OWN
# bound listener -> NOT restarted. This is the sub-claim that distinguishes
# "liveness is consulted" from "liveness is consulted AND correct".
if [[ "$r_holds" == "0" ]]; then
  echo "  [PASS] sub-claim 4: liveness-holds was NOT restarted (Restarts 0)"
else
  echo "  [FAIL] sub-claim 4: liveness-holds shows Restarts='${r_holds:-<none>}' (expected 0) — a HEALTHY workload is being restarted; see evidence/namespace_reachability.txt"
  rc=1
fi

# ---- context snapshot (ungated) --------------------------------------------
echo "  --- CONTEXT snapshot at T+40s (not gated; records the startup cliff) ---"
for label in absent fails holds; do
  cs="$(awk '/^alloc-/ { print $2; exit }' "$EVIDENCE_DIR/describe_${label}_t40s.out" 2>/dev/null)"
  cr="$(awk '/^alloc-/ { print $3; exit }' "$EVIDENCE_DIR/describe_${label}_t40s.out" 2>/dev/null)"
  echo "  [context] $label: state=${cs:-<none>} restarts=${cr:-<none>}"
done

[[ "$leak_rc" -eq 0 ]] || rc=1
echo "O07 sub-claim aggregate exit: $rc"
exit "$rc"
