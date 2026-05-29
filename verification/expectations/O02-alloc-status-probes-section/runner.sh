# shellcheck shell=bash
# O02 — alloc status Probes section render. Needs a reachable control plane.
source "$REPO_ROOT/verification/harness/lima-helpers.sh"

SVC_SPEC="crates/overdrive-cli/examples/quick-bind-service.toml"
JOB_SPEC="crates/overdrive-cli/examples/coinflip.toml"
SVC_JOB="payments"   # job name to query; confirm against the spec's [service].name

if ! capture preflight_cluster od cluster status; then
  cat <<MSG
  [pending] control plane not reachable — O02 needs a running deployment.
  In a separate Lima-routed terminal:

      cargo overdrive serve --bind 127.0.0.1:7443 --data-dir /tmp/od-o02
      cargo overdrive deploy $SVC_SPEC --detach

  then re-run O02. This run captured the preflight failure and stays 'pending'.
MSG
  exit 0
fi

rc=0

# Ensure the Service is deployed (detached), then read its alloc status.
capture deploy_service od deploy "$SVC_SPEC" --detach || true
# Confirm the queried job name; the spec's [service].name is authoritative.
svc_name="$(grep -E '^name[[:space:]]*=' "$REPO_ROOT/$SVC_SPEC" | head -1 | sed -E 's/.*"(.*)".*/\1/')"
[[ -n "$svc_name" ]] && SVC_JOB="$svc_name"

capture svc_status od alloc status "$SVC_JOB" || true
evidence_contains svc_status "Probes" || rc=1
grep -qE 'tcp |http |exec ' "$EVIDENCE_DIR/svc_status.out" \
  && echo "  [PASS] svc_status.out shows a mechanic summary" \
  || { echo "  [FAIL] svc_status.out shows no mechanic summary"; rc=1; }

# Negative case: a Job-kind alloc has NO Probes section.
capture deploy_job od deploy "$JOB_SPEC" --detach || true
job_name="$(grep -E '^name[[:space:]]*=' "$REPO_ROOT/$JOB_SPEC" | head -1 | sed -E 's/.*"(.*)".*/\1/')"
capture job_status od alloc status "${job_name:-coinflip}" || true
evidence_absent job_status "Probes" || rc=1

echo "O02 sub-claim aggregate exit: $rc"
exit "$rc"
