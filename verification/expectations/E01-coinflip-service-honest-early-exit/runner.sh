# shellcheck shell=bash
# E01 — coinflip-as-Service honest EarlyExit, end-to-end.
# Needs a reachable control plane. Does NOT background-spawn serve itself
# (leaked cgroups/XDP across runs are a documented hazard); it checks
# reachability and tells the operator the exact command if absent.
source "$REPO_ROOT/verification/harness/lima-helpers.sh"

SPEC="crates/overdrive-cli/examples/coinflip-as-service.toml"
if [[ ! -f "$REPO_ROOT/$SPEC" ]]; then
  echo "  [pending] fixture missing: $SPEC"
  exit 0
fi

# Precondition: control plane reachable?
if ! capture preflight_cluster od cluster status; then
  cat <<MSG
  [pending] control plane not reachable — E01 needs a running deployment.
  In a separate Lima-routed terminal, start one, then re-run E01:

      cargo overdrive serve --bind 127.0.0.1:7443 --data-dir /tmp/od-e01

  (Adjust bind/data-dir to your trust-triple config.) This run captured the
  preflight failure as evidence and is leaving status 'pending'.
MSG
  exit 0
fi

rc=0
# Deploy the coinflip Service and capture the streaming terminal event.
capture deploy_coinflip od deploy "$SPEC" || true

evidence_contains deploy_coinflip "EarlyExit" || rc=1
evidence_contains deploy_coinflip "exit_code" || rc=1   # tolerate exit_code: 1 / "exit_code":1
evidence_absent   deploy_coinflip "(took live)" || rc=1  # RCA-A guard
evidence_absent   deploy_coinflip "Stable"      || rc=1

echo "E01 sub-claim aggregate exit: $rc"
exit "$rc"
