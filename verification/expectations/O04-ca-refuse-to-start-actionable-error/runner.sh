# shellcheck shell=bash
# O04 — control plane refuses to start on root-key decrypt failure with an
# actionable error, distinctly per cause, with no silent re-mint.
#
# Needs the built-in CA boot path (DELIVER): root-key envelope persistence in
# the IntentStore + keyring/systemd-creds KEK provider + Earned-Trust probe.
# Until that lands, this runner records `pending` and prints the manual path.
source "$REPO_ROOT/verification/harness/lima-helpers.sh"

DATA_DIR="${DATA_DIR:-/tmp/od-o04}"

# Probe: does `overdrive serve --help` reveal a CA/keyring boot surface yet?
# (Heuristic gate — replace with the real first-boot/tamper setup in DELIVER.)
if ! capture serve_help od serve --help; then
  cat <<MSG
  [pending] cannot reach the overdrive serve surface — O04 needs the built-in
  CA boot path (DELIVER). When it lands, the capture sequence is:

    1. First boot (correct KEK) -> root persisted to \$DATA_DIR's IntentStore.
    2. Tamper one byte of the persisted root-key blob; `overdrive serve` again
       -> expect refuse-to-start naming a corrupt/tampered envelope.
    3. Restart with the WRONG KEK -> expect refuse-to-start naming a wrong-KEK
       cause, DISTINCT from (2).
    4. Restart with an ABSENT keyring KEK and no OVERDRIVE_CA_KEK -> refuse
       before any issuance; no throwaway KEK generated.
    5. Restore the correct KEK + untampered blob -> SAME root reused (no re-mint).

  This run stays 'pending'.
MSG
  exit 0
fi

cat <<MSG
  [pending] O04 capture sequence is defined above but not yet automated — the
  tamper/wrong-KEK/absent-KEK boot fixtures require the DELIVER CA boot path.
  The gated integration tests ca_boot_and_audit.rs::{boot_refuses_to_start_*}
  prove the behaviour in-tree; this expectation captures the operator-visible
  stderr quality once the serve-with-CA surface exists. Staying 'pending'.
MSG
exit 0
