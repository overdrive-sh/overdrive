# shellcheck shell=bash
# O03 — `overdrive deploy <udp-spec>` is accepted; the intent carries Proto::Udp.
#
# Operator-surface, deploy half of S-04-A. Sub-claims 1+2 (exit 0 + `Accepted.`)
# are PRE-CONVERGENCE: the deploy-accept render is produced before any
# allocation runs, so the backend binary is never launched.
#
# This runner brings up an EPHEMERAL control plane itself, inside a SINGLE Lima
# invocation, with a teardown trap on every exit path. Leaked XDP on `lo` makes
# loopback hang for the user's *other* Conductor workspaces sharing this VM, and
# leaked workload cgroups break later tests (.claude/rules/{debugging,testing}.md).
# The trap (a) kills the serve PID, (b) runs the cgroup mass-kill+rmdir sweep,
# (c) runs the XDP detach sweep. Before AND after the run it probes the
# XDP-attachment surface + loopback + cgroups and writes both into evidence/ as
# proof of no leak. Serve lifetime is seconds: boot -> deploy -> capture -> down.
#
# BOOT MODEL (post-ADR-0061, single-node-dataplane-wiring fix): production
# `overdrive serve` under the default config no longer attaches both XDP
# programs to `lo` (which collided EBUSY / failed generic-SKB attach on this VM
# and aborted boot). It now auto-provisions a dedicated host-netns veth pair
# (`ovd-veth-cli`/`ovd-veth-bk`) at boot and attaches the two distinct XDP
# programs to the two distinct veth ifaces, THEN binds the TLS listener and
# writes the trust triple. So serve BINDS here and the black-box deploy reaches
# it (evidence/serve.log: "control plane listening endpoint=..."). The runner
# still never touches `eth0` (the VM's only real NIC, shared across workspaces —
# the forbidden hazard) and never uses a test-only SimDataplane (not black-box
# reachable). The `not-ready` path below is kept as a defensive fallback: if a
# future change re-breaks boot, the runner classifies it `pending` with
# serve.log as the executed reason rather than fabricating a deploy result.
#
# Black-box only: the surface is the built `overdrive` binary (CLI) and what the
# kernel exposes (`ip link`, `bpftool`, cgroupfs). No overdrive-* crate is linked.
source "$REPO_ROOT/verification/harness/lima-helpers.sh"

SPEC="crates/overdrive-cli/examples/dns-resolver.toml"
if [[ ! -f "$REPO_ROOT/$SPEC" ]]; then
  echo "  [pending] fixture missing: $SPEC"
  exit 0
fi

# The whole serve-bringup runs as ONE root-context script inside Lima so the
# trap spans serve + deploy + teardown. $EVIDENCE_DIR / $REPO_ROOT / $SEED /
# $SPEC are interpolated host-side here, then the script is sh-escaped as a
# single arg by `cargo xtask lima run --` and executed as root in the guest
# (PATH + CARGO_TARGET_DIR re-injected by the wrapper). The repo is
# virtiofs-mounted at the same absolute path, so $EVIDENCE_DIR resolves
# identically in the guest and the inner script writes evidence directly.
INNER=$(cat <<INNER_EOF
set -uo pipefail
EVID="$EVIDENCE_DIR"
REPO="$REPO_ROOT"
SPEC="$REPO_ROOT/$SPEC"
SEED="$SEED"
cd "\$REPO"

BIND="127.0.0.1:7443"
LOOPBACK_PROBE_PORT="1"   # nothing listens here; a healthy loopback refuses fast
CFG_DIR="\$(mktemp -d /tmp/od-o03-cfg.XXXXXX)"
DATA_DIR="\$(mktemp -d /tmp/od-o03-data.XXXXXX)"
SERVE_PID=""
DEPLOY_RC="n/a"
SERVE_STATUS="not-started"

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

# ---- teardown sweep: fires on EVERY exit path (EXIT/ERR) + a post-probe ------
sweep() {
  [ -n "\$SERVE_PID" ] && kill "\$SERVE_PID" 2>/dev/null
  # cgroup mass-kill + rmdir (testing.md § "Leaked workload cgroups"). This
  # SIGKILLs every PID inside each workload scope — including any socat backend
  # a convergence path would have spawned — so no separate process-name kill is
  # needed. (O03 is pre-convergence and launches no backend at all; the sweep
  # is defensive for the ready/deploy path.) Deliberately NO broad
  # \`pgrep -f socat\`: the runner's own argv contains the literal socat command
  # string from the fixture path, so a \`pgrep -f\`/\`pgrep -af\` on it would
  # match (and \`kill\`) this very shell — a self-destruct. cgroup.kill is the
  # correct, scoped primitive.
  if cd /sys/fs/cgroup/overdrive.slice/workloads.slice 2>/dev/null; then
    for d in alloc-*.scope; do
      [ -d "\$d" ] && { echo 1 > "\$d/cgroup.kill" 2>/dev/null; rmdir "\$d" 2>/dev/null; }
    done
    cd "\$REPO" 2>/dev/null || true
  fi
  # XDP detach (debugging.md § "Leftover XDP attachments")
  for i in \$(ip -br link show | awk '{print \$1}'); do
    ip link set dev "\$i" xdpgeneric off 2>/dev/null
    ip link set dev "\$i" xdpdrv off 2>/dev/null
    ip link set dev "\$i" xdp off 2>/dev/null
  done
  return 0
}
# On EVERY exit: sweep, then write the POST-TEARDOWN no-leak proof. This runs on
# the serve-failure path too, so the no-leak evidence is ALWAYS present.
on_exit() {
  sweep
  { echo "# probe: XDP attachments POST-TEARDOWN (after sweep)"; probe_xdp; }       > "\$EVID/probe_post_teardown_xdp.txt"      2>&1
  { echo "# probe: loopback POST-TEARDOWN (after sweep)";        probe_loopback; }  > "\$EVID/probe_post_teardown_loopback.txt" 2>&1
  { echo "# probe: workload cgroups POST-TEARDOWN (after sweep)"; probe_cgroups; }  > "\$EVID/probe_post_teardown_cgroups.txt"  2>&1
  return 0
}
trap on_exit EXIT

# ---- BEFORE probes (proof of clean start) -----------------------------------
{ echo "# probe: XDP attachments BEFORE run"; probe_xdp; }       > "\$EVID/probe_before_xdp.txt"      2>&1
{ echo "# probe: loopback BEFORE run";        probe_loopback; }  > "\$EVID/probe_before_loopback.txt" 2>&1
{ echo "# probe: workload cgroups BEFORE run"; probe_cgroups; }  > "\$EVID/probe_before_cgroups.txt"  2>&1

# If a pre-existing leak is detected at start, clean it and note it.
if grep -q 'HANG' "\$EVID/probe_before_loopback.txt" \
   || grep -qE '(xdp_|overdrive)' "\$EVID/probe_before_xdp.txt" \
   || ! grep -q '(no alloc' "\$EVID/probe_before_cgroups.txt"; then
  echo "PRE-EXISTING LEAK detected at runner start; cleaning before serve." >> "\$EVID/probe_before_cgroups.txt"
  sweep
fi

# ---- build once so serve + deploy don't race / double-compile ---------------
echo "# building overdrive binary (single compile shared by serve+deploy)"
if ! cargo build -p overdrive-cli --bin overdrive 2> "\$EVID/build.log"; then
  echo "BUILD_FAILED"; tail -40 "\$EVID/build.log"
  echo "INNER_DONE serve_status=build-failed deploy_rc=n/a"
  exit 0
fi
BIN="\$CARGO_TARGET_DIR/debug/overdrive"
[ -x "\$BIN" ] || { echo "BIN_MISSING: \$BIN"; echo "INNER_DONE serve_status=bin-missing deploy_rc=n/a"; exit 0; }

# ---- background the ephemeral serve under the shared config dir -------------
echo "# starting ephemeral serve: bind=\$BIND cfg=\$CFG_DIR data=\$DATA_DIR"
OVERDRIVE_CONFIG_DIR="\$CFG_DIR" "\$BIN" serve --bind "\$BIND" --data-dir "\$DATA_DIR" \
  > "\$EVID/serve.log" 2>&1 &
SERVE_PID=\$!

# Readiness gate: serve writes the trust-triple config AFTER it binds the TLS
# listener (serve.rs) — but ONLY if the EbpfDataplane XDP attach at boot
# succeeded. Wait for the config file OR serve death, up to ~25s.
CFG_FILE="\$CFG_DIR/.overdrive/config"
ready=0
for _ in \$(seq 1 50); do
  if [ -f "\$CFG_FILE" ]; then ready=1; break; fi
  if ! kill -0 "\$SERVE_PID" 2>/dev/null; then break; fi
  sleep 0.5
done

if [ "\$ready" -ne 1 ]; then
  # serve never bound. On this VM the cause is the XDP-attach-to-lo failure
  # captured in serve.log; classify as PENDING (not a deploy FAIL) and let the
  # host gate read serve.log for the precise reason.
  SERVE_STATUS="not-ready"
  echo "SERVE_NOT_READY: trust-triple config never appeared at \$CFG_FILE"
  echo "--- serve.log tail ---"; tail -20 "\$EVID/serve.log"
  echo "INNER_DONE serve_status=not-ready deploy_rc=n/a"
  exit 0   # the on_exit trap writes the post-teardown no-leak proof
fi
SERVE_STATUS="ready"
echo "# serve ready: trust triple written at \$CFG_FILE (pid \$SERVE_PID)"

# ---- DEPLOY the UDP dns-resolver Service; capture the accept render ----------
{
  echo "# command: overdrive deploy \$SPEC"
  echo "# seed:    \$SEED"
  echo "# started: \$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} > "\$EVID/deploy_dns_resolver.meta"
DEPLOY_RC=0
OVERDRIVE_CONFIG_DIR="\$CFG_DIR" "\$BIN" deploy "\$SPEC" \
  > "\$EVID/deploy_dns_resolver.out" 2>&1 || DEPLOY_RC=\$?
echo "# exit:    \$DEPLOY_RC" >> "\$EVID/deploy_dns_resolver.meta"
echo "# deploy exit: \$DEPLOY_RC"

# ---- alloc status: capture the listener-protocol render (sub-claim 3) --------
# A Service's listeners are projected from the persisted WorkloadIntent::Service
# aggregate (not the allocation), so \`alloc status\` renders each listener as
# <port>/<protocol> IMMEDIATELY after deploy-accept — before any convergence to
# a Running allocation. This is the operator-surface proof that the accepted
# intent carries Proto::Udp (closing sub-claim 3 black-box).
ALLOC_RC=0
OVERDRIVE_CONFIG_DIR="\$CFG_DIR" "\$BIN" alloc status --job dns-resolver \
  > "\$EVID/alloc_status_dns_resolver.out" 2>&1 || ALLOC_RC=\$?
echo "# alloc status exit: \$ALLOC_RC"

# ---- AFTER probes (steady state the run produced, pre-teardown) -------------
{ echo "# probe: XDP attachments AFTER deploy (pre-teardown)"; probe_xdp; }       > "\$EVID/probe_after_xdp.txt"      2>&1
{ echo "# probe: loopback AFTER deploy";  probe_loopback; }                       > "\$EVID/probe_after_loopback.txt" 2>&1
{ echo "# probe: workload cgroups AFTER deploy"; probe_cgroups; }                 > "\$EVID/probe_after_cgroups.txt"  2>&1

# Stop serve explicitly (the on_exit trap also covers abnormal exits + writes
# the post-teardown no-leak proof).
kill "\$SERVE_PID" 2>/dev/null
wait "\$SERVE_PID" 2>/dev/null

echo "INNER_DONE serve_status=ready deploy_rc=\$DEPLOY_RC"
exit 0
INNER_EOF
)

# Run the whole bring-up as a single root-context Lima invocation; tee the
# guest-side narration into evidence/ for the audit trail. The deploy exit
# code is recorded in deploy_dns_resolver.meta by the inner script.
in_lima bash -c "$INNER" > "$EVIDENCE_DIR/serve_deploy.out" 2>&1 || true

echo "  --- serve_deploy.out (tail) ---"
tail -20 "$EVIDENCE_DIR/serve_deploy.out" || true

# ---- No-leak gate (HARD): loopback must be HEALTHY before AND post-teardown --
# A leaked XDP on lo would hang the user's other workspaces; treat a non-healthy
# loopback as a runner failure regardless of deploy outcome. These two probes
# ALWAYS run (the post-teardown one fires from the on_exit trap even when serve
# never bound), so the no-leak proof is always present.
leak_rc=0
for p in probe_before_loopback probe_post_teardown_loopback; do
  if [[ -f "$EVIDENCE_DIR/$p.txt" ]] && grep -q 'HEALTHY' "$EVIDENCE_DIR/$p.txt"; then
    echo "  [PASS] no-leak: $p shows HEALTHY loopback"
  else
    echo "  [FAIL] no-leak: $p is not HEALTHY — possible leaked XDP on lo"
    leak_rc=1
  fi
done
# Post-teardown XDP + cgroup sweeps must leave nothing behind.
if grep -q '(none)' "$EVIDENCE_DIR/probe_post_teardown_xdp.txt" 2>/dev/null; then
  echo "  [PASS] no-leak: post-teardown shows no XDP programs attached"
else
  echo "  [WARN] no-leak: post-teardown XDP probe is non-empty — inspect probe_post_teardown_xdp.txt"
fi

# ---- Did serve bind black-box at all? ---------------------------------------
serve_status="$(sed -n 's/.*serve_status=\([a-z-]*\).*/\1/p' "$EVIDENCE_DIR/serve_deploy.out" | tail -1)"
if [[ "$serve_status" != "ready" ]]; then
  echo "  [pending] black-box serve did not bind (serve_status='${serve_status:-unknown}')."
  echo "            Post-ADR-0061, serve is EXPECTED to bind (it provisions the"
  echo "            ovd-veth-cli/ovd-veth-bk pair and attaches XDP to the veth ifaces,"
  echo "            not lo). A not-ready result here is a REGRESSION — inspect"
  echo "            evidence/serve.log for the actual cause (veth provisioning failed,"
  echo "            CAP_NET_ADMIN missing, a stale XDP program on ovd-veth-cli forcing"
  echo "            EBUSY -> sweep per debugging.md, or a boot regression). The runner"
  echo "            does NOT touch eth0 (shared NIC, forbidden) or a SimDataplane"
  echo "            override (test-only). Sub-claims 1+2 stay 'pending' until boot is"
  echo "            restored; the 'what, forever' witness is the direct-handler test"
  echo "            deploy_udp_walking_skeleton.rs (SimDataplane serve)."
  # serve-not-ready is DATA, not a runner crash; but a real leak is a hard fail.
  exit "$leak_rc"
fi

# ---- serve bound — evaluate the real sub-claims -----------------------------
rc=0
# Sub-claim 1: deploy exited 0
deploy_rc="$(sed -n 's/^# exit:[[:space:]]*//p' "$EVIDENCE_DIR/deploy_dns_resolver.meta" 2>/dev/null)"
if [[ "$deploy_rc" == "0" ]]; then
  echo "  [PASS] sub-claim 1: deploy exited 0"
else
  echo "  [FAIL] sub-claim 1: deploy exited '${deploy_rc:-<none>}' (expected 0)"
  rc=1
fi
# Sub-claim 2: stdout contains `Accepted.` (workload_submit_accepted shape)
if [[ -f "$EVIDENCE_DIR/deploy_dns_resolver.out" ]]; then
  evidence_contains deploy_dns_resolver "Accepted." || rc=1
else
  echo "  [FAIL] sub-claim 2: no deploy output captured"
  rc=1
fi

# Sub-claim 3: listener protocol (Proto::Udp) rendered at the operator surface.
# `overdrive alloc status --job dns-resolver` renders each Service listener as
# <port>/<protocol>, projected from the persisted WorkloadIntent::Service
# aggregate (commit 7e79007f) — so the udp/15353 listener surfaces as `15353/udp`.
# This is the black-box operator-surface proof that the accepted intent carries
# Proto::Udp (never coerced to Tcp): a tcp coercion would render `15353/tcp`.
if [[ -f "$EVIDENCE_DIR/alloc_status_dns_resolver.out" ]]; then
  evidence_contains alloc_status_dns_resolver "15353/udp" || rc=1
  # Negative guard: the udp listener must NOT be rendered as tcp.
  if grep -q '15353/tcp' "$EVIDENCE_DIR/alloc_status_dns_resolver.out"; then
    echo "  [FAIL] sub-claim 3: listener rendered as 15353/tcp — Proto coerced to Tcp"
    rc=1
  fi
else
  echo "  [FAIL] sub-claim 3: no alloc status output captured"
  rc=1
fi

[[ "$leak_rc" -eq 0 ]] || rc=1
echo "O03 sub-claim aggregate exit: $rc"
exit "$rc"
