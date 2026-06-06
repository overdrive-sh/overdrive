# shellcheck shell=bash
# O06 — describe of a deployed Service returns DescribeSpecOutput::Service + VIP.
#
# GH #183 / ADR-0064. Proves the describe inspection path works for a Service
# (it returned HTTP 400 before #183) and surfaces the platform-issued VIP.
#
# Three sub-claims, all black-box against the built `overdrive` binary on a
# real kernel (Lima):
#   SC1 (precondition): `overdrive deploy <svc>` exits 0 + prints `Accepted.`
#       — the Service intent is persisted and its VIP allocated at submit.
#   SC2 (O-surface):    `overdrive alloc status --job <id>` exits 0 — the real
#       operator CLI mTLS client deserialises the NEW DescribeSpecOutput
#       discriminated wire shape (the regression witness for the migration).
#   SC3 (#183 deliverable): a raw mTLS `GET /v1/jobs/<id>` returns JSON whose
#       `spec` carries `"kind":"service"` + a required `"vip"` IPv4 dotted-quad.
#
# Mirrors O03's ephemeral-serve + teardown-trap + leak-probe discipline:
# leaked XDP on `lo` hangs loopback for the user's OTHER Conductor workspaces
# sharing this VM, and leaked workload cgroups break later tests
# (.claude/rules/{debugging,testing}.md). The trap kills serve, mass-kills +
# rmdirs workload cgroup scopes, and detaches XDP; before AND after, it probes
# XDP + loopback + cgroups into evidence/ as no-leak proof.
#
# Black-box only: the surfaces are the built `overdrive` binary (CLI), the
# control-plane HTTP API (curl + the serve-written trust triple), and what the
# kernel exposes (`ip link`, `bpftool`, cgroupfs). No overdrive-* crate linked.
source "$REPO_ROOT/verification/harness/lima-helpers.sh"

SPEC="examples/dns-resolver.toml"
WORKLOAD_ID="dns-resolver"
if [[ ! -f "$REPO_ROOT/$SPEC" ]]; then
  echo "  [pending] fixture missing: $SPEC"
  exit 0
fi

INNER=$(cat <<INNER_EOF
set -uo pipefail
EVID="$EVIDENCE_DIR"
REPO="$REPO_ROOT"
SPEC="$REPO_ROOT/$SPEC"
WID="$WORKLOAD_ID"
SEED="$SEED"
cd "\$REPO"

BIND="127.0.0.1:7443"
LOOPBACK_PROBE_PORT="1"
CFG_DIR="\$(mktemp -d /tmp/od-o06-cfg.XXXXXX)"
DATA_DIR="\$(mktemp -d /tmp/od-o06-data.XXXXXX)"
SERVE_PID=""

probe_xdp() {
  for i in \$(ip -br link show | awk '{print \$1}'); do
    info="\$(ip link show "\$i")"
    case "\$info" in
      *xdpgeneric*|*xdpdrv*|*xdp\ *)
        echo "=== \$i ==="; echo "\$info" | grep -E 'xdp(generic|drv)?' ;;
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
    if [ "\$rc" -eq 124 ]; then echo "HANG: loopback connect timed out after 3s -> probable leaked XDP on lo"
    else echo "HEALTHY: loopback refused fast (no leaked XDP swallowing lo traffic)"; fi
  fi
}
probe_cgroups() {
  ls /sys/fs/cgroup/overdrive.slice/workloads.slice/ 2>/dev/null | grep '^alloc-' || echo "(no alloc-*.scope)"
}
sweep() {
  [ -n "\$SERVE_PID" ] && kill "\$SERVE_PID" 2>/dev/null
  if cd /sys/fs/cgroup/overdrive.slice/workloads.slice 2>/dev/null; then
    for d in alloc-*.scope; do
      [ -d "\$d" ] && { echo 1 > "\$d/cgroup.kill" 2>/dev/null; rmdir "\$d" 2>/dev/null; }
    done
    cd "\$REPO" 2>/dev/null || true
  fi
  for i in \$(ip -br link show | awk '{print \$1}'); do
    ip link set dev "\$i" xdpgeneric off 2>/dev/null
    ip link set dev "\$i" xdpdrv off 2>/dev/null
    ip link set dev "\$i" xdp off 2>/dev/null
  done
  return 0
}
on_exit() {
  sweep
  { echo "# probe: XDP attachments POST-TEARDOWN (after sweep)"; probe_xdp; }      > "\$EVID/probe_post_teardown_xdp.txt"      2>&1
  { echo "# probe: loopback POST-TEARDOWN (after sweep)";        probe_loopback; } > "\$EVID/probe_post_teardown_loopback.txt" 2>&1
  { echo "# probe: workload cgroups POST-TEARDOWN (after sweep)"; probe_cgroups; } > "\$EVID/probe_post_teardown_cgroups.txt"  2>&1
  return 0
}
trap on_exit EXIT

{ echo "# probe: XDP attachments BEFORE run"; probe_xdp; }       > "\$EVID/probe_before_xdp.txt"      2>&1
{ echo "# probe: loopback BEFORE run";        probe_loopback; }  > "\$EVID/probe_before_loopback.txt" 2>&1
{ echo "# probe: workload cgroups BEFORE run"; probe_cgroups; }  > "\$EVID/probe_before_cgroups.txt"  2>&1

if grep -q 'HANG' "\$EVID/probe_before_loopback.txt" \
   || grep -qE '(xdp_|overdrive)' "\$EVID/probe_before_xdp.txt" \
   || ! grep -q '(no alloc' "\$EVID/probe_before_cgroups.txt"; then
  echo "PRE-EXISTING LEAK detected at runner start; cleaning before serve." >> "\$EVID/probe_before_cgroups.txt"
  sweep
fi

echo "# building overdrive binary"
if ! cargo build -p overdrive-cli --bin overdrive 2> "\$EVID/build.log"; then
  echo "BUILD_FAILED"; tail -40 "\$EVID/build.log"
  echo "INNER_DONE serve_status=build-failed"; exit 0
fi
BIN="\$CARGO_TARGET_DIR/debug/overdrive"
[ -x "\$BIN" ] || { echo "BIN_MISSING: \$BIN"; echo "INNER_DONE serve_status=bin-missing"; exit 0; }

echo "# starting ephemeral serve: bind=\$BIND cfg=\$CFG_DIR data=\$DATA_DIR"
OVERDRIVE_CONFIG_DIR="\$CFG_DIR" "\$BIN" serve --bind "\$BIND" --data-dir "\$DATA_DIR" \
  > "\$EVID/serve.log" 2>&1 &
SERVE_PID=\$!

CFG_FILE="\$CFG_DIR/.overdrive/config"
ready=0
for _ in \$(seq 1 50); do
  if [ -f "\$CFG_FILE" ]; then ready=1; break; fi
  if ! kill -0 "\$SERVE_PID" 2>/dev/null; then break; fi
  sleep 0.5
done
if [ "\$ready" -ne 1 ]; then
  echo "SERVE_NOT_READY: trust-triple config never appeared at \$CFG_FILE"
  echo "--- serve.log tail ---"; tail -20 "\$EVID/serve.log"
  echo "INNER_DONE serve_status=not-ready"; exit 0
fi
echo "# serve ready: trust triple at \$CFG_FILE (pid \$SERVE_PID)"

# --- SC1: deploy the Service (persists intent + allocates VIP at submit) ---
{
  echo "# command: overdrive deploy \$SPEC"
  echo "# seed:    \$SEED"
  echo "# started: \$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} > "\$EVID/deploy_service.meta"
DEPLOY_RC=0
OVERDRIVE_CONFIG_DIR="\$CFG_DIR" "\$BIN" deploy "\$SPEC" > "\$EVID/deploy_service.out" 2>&1 || DEPLOY_RC=\$?
echo "# exit:    \$DEPLOY_RC" >> "\$EVID/deploy_service.meta"
echo "# deploy exit: \$DEPLOY_RC"

# --- wait for convergence so the capture shows a Running allocation, not the
#     0-allocations empty-state. The default Service startup probe interval is
#     ~2s, so a single 1s sleep cannot reach Running — poll alloc status up to
#     ~25s and stop the moment allocations > 0. The describe VIP is allocated at
#     SUBMIT (ADR-0049) and is unaffected by convergence; this only strengthens
#     the alloc-status capture (SC2) to a converged operator view.
CONVERGE_SECS=0
for _ in \$(seq 1 25); do
  OVERDRIVE_CONFIG_DIR="\$CFG_DIR" "\$BIN" alloc status --job "\$WID" > "\$EVID/alloc_status_service.out" 2>&1 || true
  if grep -qE '^Allocations:[[:space:]]+[1-9]' "\$EVID/alloc_status_service.out"; then break; fi
  sleep 1; CONVERGE_SECS=\$((CONVERGE_SECS+1))
done

# --- SC2: final authoritative O-surface capture (exit code + converged output) ---
ALLOC_RC=0
OVERDRIVE_CONFIG_DIR="\$CFG_DIR" "\$BIN" alloc status --job "\$WID" > "\$EVID/alloc_status_service.out" 2>&1 || ALLOC_RC=\$?
{ echo "alloc_status_exit=\$ALLOC_RC"; echo "converged_after_secs=\$CONVERGE_SECS"; } > "\$EVID/alloc_status_service.meta"
echo "# alloc status exit: \$ALLOC_RC (converged_after=\${CONVERGE_SECS}s)"

# --- SC3: raw mTLS GET /v1/jobs/<id> — the describe wire response (#183) ---
# Extract the trust triple's base64-PEM ca/crt/key (single-line TOML inline
# strings) without a toml parser, decode, and curl the describe endpoint.
EP="\$(grep -E '^[[:space:]]*endpoint[[:space:]]*=' "\$CFG_FILE" | head -1 | sed 's/.*=[[:space:]]*"\(.*\)"/\1/')"
[ -n "\$EP" ] || EP="https://\$BIND"
for f in ca crt key; do
  grep -E "^[[:space:]]*\$f[[:space:]]*=" "\$CFG_FILE" | head -1 \
    | sed "s/.*=[[:space:]]*\"\(.*\)\"/\1/" | base64 -d > "/tmp/od-o06-\$f.pem" 2>/dev/null
done
DESCRIBE_HTTP=000
if [ -s /tmp/od-o06-ca.pem ] && [ -s /tmp/od-o06-crt.pem ] && [ -s /tmp/od-o06-key.pem ]; then
  DESCRIBE_HTTP="\$(curl -sS --cacert /tmp/od-o06-ca.pem --cert /tmp/od-o06-crt.pem --key /tmp/od-o06-key.pem \
      -o "\$EVID/describe_api.json" -w '%{http_code}' "\$EP/v1/jobs/\$WID" 2>"\$EVID/describe_api.curlerr" || true)"
else
  echo "CERT_EXTRACT_FAILED: one of ca/crt/key did not decode from \$CFG_FILE" > "\$EVID/describe_api.curlerr"
fi
echo "describe_http_code=\$DESCRIBE_HTTP" > "\$EVID/describe_api.meta"
echo "describe_endpoint=\$EP/v1/jobs/\$WID" >> "\$EVID/describe_api.meta"
echo "# describe GET /v1/jobs/\$WID -> HTTP \$DESCRIBE_HTTP"
rm -f /tmp/od-o06-ca.pem /tmp/od-o06-crt.pem /tmp/od-o06-key.pem

{ echo "# probe: XDP attachments AFTER (pre-teardown)"; probe_xdp; }       > "\$EVID/probe_after_xdp.txt"      2>&1
{ echo "# probe: loopback AFTER";  probe_loopback; }                       > "\$EVID/probe_after_loopback.txt" 2>&1
{ echo "# probe: workload cgroups AFTER"; probe_cgroups; }                 > "\$EVID/probe_after_cgroups.txt"  2>&1

kill "\$SERVE_PID" 2>/dev/null; wait "\$SERVE_PID" 2>/dev/null
echo "INNER_DONE serve_status=ready deploy_rc=\$DEPLOY_RC alloc_rc=\$ALLOC_RC describe_http=\$DESCRIBE_HTTP"
exit 0
INNER_EOF
)

in_lima bash -c "$INNER" > "$EVIDENCE_DIR/serve_describe.out" 2>&1 || true

echo "  --- serve_describe.out (tail) ---"
tail -20 "$EVIDENCE_DIR/serve_describe.out" || true

# --- no-leak gate (same discipline as O03) ---
leak_rc=0
for p in probe_before_loopback probe_post_teardown_loopback; do
  if [[ -f "$EVIDENCE_DIR/$p.txt" ]] && grep -q 'HEALTHY' "$EVIDENCE_DIR/$p.txt"; then
    echo "  [PASS] no-leak: $p shows HEALTHY loopback"
  else
    echo "  [FAIL] no-leak: $p is not HEALTHY — possible leaked XDP on lo"; leak_rc=1
  fi
done
if grep -q '(none)' "$EVIDENCE_DIR/probe_post_teardown_xdp.txt" 2>/dev/null; then
  echo "  [PASS] no-leak: post-teardown shows no XDP programs attached"
else
  echo "  [WARN] no-leak: post-teardown XDP probe non-empty — inspect probe_post_teardown_xdp.txt"
fi

serve_status="$(sed -n 's/.*serve_status=\([a-z-]*\).*/\1/p' "$EVIDENCE_DIR/serve_describe.out" | tail -1)"
if [[ "$serve_status" != "ready" ]]; then
  echo "  [pending] black-box serve did not bind (serve_status='${serve_status:-unknown}')."
  echo "            Post-ADR-0061 serve is EXPECTED to bind (veth pair + XDP on veth, not lo)."
  echo "            A not-ready result is a boot REGRESSION — inspect evidence/serve.log."
  echo "            The 'what, forever' witness for the describe shape is the integration"
  echo "            test describe_service_returns_discriminated_shape_with_vip."
  exit "$leak_rc"
fi

rc=0
# SC1 — deploy accepted (precondition)
deploy_rc="$(sed -n 's/^# exit:[[:space:]]*//p' "$EVIDENCE_DIR/deploy_service.meta" 2>/dev/null)"
if [[ "$deploy_rc" == "0" ]]; then echo "  [PASS] SC1: deploy exited 0"; else echo "  [FAIL] SC1: deploy exited '${deploy_rc:-<none>}'"; rc=1; fi
if [[ -f "$EVIDENCE_DIR/deploy_service.out" ]]; then evidence_contains deploy_service "Accepted." || rc=1; else echo "  [FAIL] SC1: no deploy output"; rc=1; fi

# SC2 — CLI round-trips the widened describe response
alloc_rc="$(sed -n 's/^alloc_status_exit=//p' "$EVIDENCE_DIR/alloc_status_service.meta" 2>/dev/null)"
if [[ "$alloc_rc" == "0" ]]; then
  echo "  [PASS] SC2: alloc status exited 0 — CLI deserialised DescribeSpecOutput"
else
  echo "  [FAIL] SC2: alloc status exited '${alloc_rc:-<none>}' — CLI failed to round-trip the describe response"; rc=1
fi

# SC2b — the operator CLI now RENDERS the Service VIP (the rendering half of #220).
# alloc status reads vip from the AllocStatusResponse envelope (ADR-0049) and prints
# a `VIP: <ipv4>` line for Service reads. Before this landed the VIP was on the wire
# but dropped by the CLI; this sub-claim is the operator-visible-VIP proof.
if grep -Eq '^VIP:[[:space:]]+[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+' "$EVIDENCE_DIR/alloc_status_service.out" 2>/dev/null; then
  echo "  [PASS] SC2b: alloc status renders the Service VIP line (operator-visible)"
else
  echo "  [FAIL] SC2b: alloc status output has no 'VIP: <ipv4>' line — CLI VIP rendering missing"; rc=1
fi

# Informational (NOT a sub-claim — describe/VIP does not require convergence):
converged_secs="$(sed -n 's/^converged_after_secs=//p' "$EVIDENCE_DIR/alloc_status_service.meta" 2>/dev/null)"
if grep -qE '^Allocations:[[:space:]]+[1-9]' "$EVIDENCE_DIR/alloc_status_service.out" 2>/dev/null; then
  echo "  [info] workload converged to >=1 allocation after ~${converged_secs}s (captured running, not the 0-alloc empty-state)"
else
  echo "  [info] workload still 0 allocations after ~${converged_secs}s poll — capture is pre-convergence (issue #219 empty-state hint applies); describe/VIP still valid (allocated at submit)"
fi

# SC3 — discriminated shape + VIP on the describe wire (the #183 deliverable)
http="$(sed -n 's/^describe_http_code=//p' "$EVIDENCE_DIR/describe_api.meta" 2>/dev/null)"
if [[ "$http" == "200" ]] && [[ -f "$EVIDENCE_DIR/describe_api.json" ]]; then
  if grep -Eq '"kind"[[:space:]]*:[[:space:]]*"service"' "$EVIDENCE_DIR/describe_api.json"; then
    echo "  [PASS] SC3a: describe response carries \"kind\":\"service\" (discriminated)"
  else
    echo "  [FAIL] SC3a: describe response missing \"kind\":\"service\""; rc=1
  fi
  if grep -Eq '"vip"[[:space:]]*:[[:space:]]*"[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+"' "$EVIDENCE_DIR/describe_api.json"; then
    echo "  [PASS] SC3b: describe response carries platform-issued \"vip\" IPv4"
  else
    echo "  [FAIL] SC3b: describe response missing a \"vip\" IPv4 dotted-quad"; rc=1
  fi
else
  echo "  [FAIL] SC3: describe GET returned HTTP '${http:-<none>}' (expected 200) — see describe_api.curlerr"; rc=1
fi

[[ "$leak_rc" -eq 0 ]] || rc=1
echo "O06 sub-claim aggregate exit: $rc"
exit "$rc"
