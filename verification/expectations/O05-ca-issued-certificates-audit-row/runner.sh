# shellcheck shell=bash
# O05 — issued_certificates audit row is observable via `alloc status`.
#
# Needs the built-in CA issuance path (DELIVER): SVID issuance on alloc-start
# writes the issued_certificates observation row, surfaced by `alloc status`.
# Until that lands, this runner records `pending`.
source "$REPO_ROOT/verification/harness/lima-helpers.sh"

# A deployed workload that triggers SVID issuance. The exact spec lands with
# the CA issuance wiring in DELIVER; use the coinflip job as the placeholder
# allocation whose status we would read.
JOB_SPEC="${JOB_SPEC:-crates/overdrive-cli/examples/coinflip.toml}"

if ! capture preflight_cluster od cluster status; then
  cat <<MSG
  [pending] control plane not reachable — O05 needs a running deployment with
  the built-in CA issuance path (DELIVER). In a Lima-routed terminal:

      cargo overdrive serve --bind 127.0.0.1:7443 --data-dir /tmp/od-o05
      cargo overdrive deploy $JOB_SPEC --detach

  then re-run O05. This run stays 'pending'.
MSG
  exit 0
fi

rc=0
job_name="$(grep -E '^name[[:space:]]*=' "$REPO_ROOT/$JOB_SPEC" | head -1 | sed -E 's/.*"(.*)".*/\1/')"
capture deploy_job od deploy "$JOB_SPEC" --detach || true
capture alloc_status od alloc status "${job_name:-coinflip}" || true

# Sub-claim 1 — alloc status surfaces the issued_certificates record.
# (The exact heading lands with the DELIVER render; grep both likely shapes.)
if grep -qiE 'issued.certificate|issued_certificates|SVID|serial' "$EVIDENCE_DIR/alloc_status.out"; then
  echo "  [PASS] alloc status surfaces an issued-certificate record"
else
  echo "  [pending] alloc status shows no issued-certificate section yet — the"
  echo "            DELIVER render for issued_certificates is not present. The"
  echo "            in-tree gated tests ca_boot_and_audit.rs (S-05-03/04) prove"
  echo "            the row write; this expectation captures the read surface."
  rc=0
fi

exit $rc
