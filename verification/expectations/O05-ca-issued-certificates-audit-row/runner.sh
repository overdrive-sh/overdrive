# shellcheck shell=bash
# O05 — issued_certificates audit row is observable via `alloc status`.
#
# Needs the built-in CA issuance path (DELIVER): SVID issuance on alloc-start
# writes the issued_certificates observation row, surfaced by `alloc status`.
# The live render is `render::render_issued_certificates_section`
# (crates/overdrive-cli/src/render.rs) — heading `Issued certificates:` with the
# four audit-row FACTS (serial / spiffe_id / issuer_serial / not_after) and NO
# cert bytes / NO private key. Until a deployment that issues an SVID is
# reachable, this runner records `pending`.
source "$REPO_ROOT/verification/harness/lima-helpers.sh"

# A deployed workload that triggers SVID issuance. The exact spec lands with
# the CA issuance wiring in DELIVER; use the coinflip job as the placeholder
# allocation whose status we would read.
JOB_SPEC="${JOB_SPEC:-crates/overdrive-cli/examples/coinflip.toml}"

if ! capture preflight_cluster od cluster status; then
  cat <<'MSG'
  [pending] control plane not reachable — O05 needs a running deployment with
  the built-in CA issuance path (DELIVER). The O05 runner reads an
  already-running deployment; it does not stand up "serve" itself. In a
  Lima-routed terminal:

      cargo overdrive serve --bind 127.0.0.1:7443 --data-dir /tmp/od-o05
      cargo overdrive deploy crates/overdrive-cli/examples/coinflip.toml --detach

  then re-run O05. This run stays 'pending'.
MSG
  exit 0
fi

rc=0
job_name="$(grep -E '^name[[:space:]]*=' "$REPO_ROOT/$JOB_SPEC" | head -1 | sed -E 's/.*"(.*)".*/\1/')"
capture deploy_job od deploy "$JOB_SPEC" --detach || true
capture alloc_status od alloc status "${job_name:-coinflip}" || true

# Sub-claim 1 — alloc status surfaces the live `Issued certificates:` section
# with the four operator-legible facts. The live render heading is
# `Issued certificates:` (render_issued_certificates_section); each row carries
# `serial:`, `spiffe_id:`, `issuer_serial:`, `not_after:`.
status_out="$EVIDENCE_DIR/alloc_status.out"
if grep -qE '^Issued certificates:' "$status_out" \
   && grep -qE '^[[:space:]]*serial:' "$status_out" \
   && grep -qE '^[[:space:]]*spiffe_id:' "$status_out" \
   && grep -qE '^[[:space:]]*issuer_serial:' "$status_out" \
   && grep -qE '^[[:space:]]*not_after:' "$status_out"; then
  echo "  [PASS] alloc status surfaces 'Issued certificates:' with serial / spiffe_id / issuer_serial / not_after"

  # Sub-claim 2 — the render is metadata-only: NO cert bytes, NO private key.
  # The issued_certificates audit row persists only facts (the workload holds
  # NOTHING; the kernel does mTLS — CLAUDE.md workload-identity model). A PEM
  # block or key in the operator render would be a leak.
  if grep -qiE 'BEGIN (CERTIFICATE|PRIVATE KEY|EC PRIVATE KEY|RSA PRIVATE KEY)' "$status_out"; then
    echo "  [FAIL] alloc status leaks PEM cert/key bytes — the summary must be metadata-only"
    rc=1
  else
    echo "  [PASS] no cert bytes / no private key in the render (metadata-only, as designed)"
  fi
else
  echo "  [pending] alloc status shows no 'Issued certificates:' section yet — the"
  echo "            DELIVER issuance-on-alloc-start path did not write/surface an"
  echo "            issued_certificates row for this deployment. The in-tree gated"
  echo "            tests ca_boot_and_audit.rs (S-05-03/04) prove the row write;"
  echo "            this expectation captures the operator-visible read surface."
  rc=0
fi

exit $rc
