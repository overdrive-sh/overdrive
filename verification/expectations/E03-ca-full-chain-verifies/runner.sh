# shellcheck shell=bash
# E03 — full Root -> Intermediate -> SVID chain verifies under `openssl verify`,
# AND the pathLen=0 negative anchor FAILS `openssl verify` (S-OC-13/14/15).
#
# Walking-skeleton proof (S-04-07, KPI K1). There is NO operator CLI verb to
# mint/export an SVID this phase (feature-delta D-CA-4) — so this runner produces
# its PEMs as a SIDE-EFFECT of the gated `rcgen_ca_chain_verify.rs` integration
# tests under an `OD_E03_CA_DIR` env-gate, then does its black-box `openssl`
# checks over the exported `$CA_DIR/{positive,negative}/*.pem`. `cargo nextest`
# is invoked ONLY as the PEM producer; the runner stays bash + openssl +
# file-observation and links no `overdrive-*` crate (verification/README.md).
#
# Three sub-claims, ALL required before any `satisfied`:
#   1. positive chain: `openssl verify -CAfile root.pem -untrusted intermediate.pem svid.pem` -> exit 0 ("OK").
#   2. leaf profile: exactly one spiffe:// URI SAN, CA:FALSE, digitalSignature critical.
#   3. negative anchor (MANDATORY): a further-CA under the pathLen=0 intermediate
#      FAILS `openssl verify` (non-zero exit) — pathLen ENFORCED, not merely set.
source "$REPO_ROOT/verification/harness/lima-helpers.sh"

# $CA_DIR must be a path SHARED between the macOS host (where this runner + its
# openssl checks run) and the Lima guest (where the gated test writes the PEMs).
# The repo working tree is virtiofs-mounted into Lima at the same absolute path,
# so anchoring under $EVIDENCE_DIR (inside the repo) makes the test's writes
# visible here AND captures the PEMs as pinned evidence.
CA_DIR="${CA_DIR:-$EVIDENCE_DIR/ca}"
POS_DIR="$CA_DIR/positive"
NEG_DIR="$CA_DIR/negative"
rm -rf "$CA_DIR"
mkdir -p "$CA_DIR"

ROOT_PEM="$POS_DIR/root.pem"
INT_PEM="$POS_DIR/intermediate.pem"
SVID_PEM="$POS_DIR/svid.pem"
NEG_ROOT_PEM="$NEG_DIR/root.pem"
NEG_INT_PEM="$NEG_DIR/intermediate.pem"
NEG_FURTHER_PEM="$NEG_DIR/furtherca.pem"
NEG_LEAF_PEM="$NEG_DIR/leaf.pem"

# --- PEM producer (in Lima, real ring/rcgen crypto) --------------------------
# Run BOTH chain-verify tests with OD_E03_CA_DIR set. The
# `rcgen_full_svid_chain_verifies_root_intermediate_svid` test exports the
# positive triple (sub-claims 1+2); the
# `rcgen_intermediate_cannot_sign_a_further_ca_path_len_enforced` test exports
# the negative triple (sub-claim 3) to the DISTINCT negative/ sub-dir.
capture produce_pems \
  in_lima env "OD_E03_CA_DIR=$CA_DIR" \
  cargo nextest run -p overdrive-host --features integration-tests \
  -E 'test(rcgen_full_svid_chain_verifies_root_intermediate_svid) + test(rcgen_intermediate_cannot_sign_a_further_ca_path_len_enforced)' \
  || true

if [[ ! -f "$ROOT_PEM" || ! -f "$INT_PEM" || ! -f "$SVID_PEM" \
   || ! -f "$NEG_ROOT_PEM" || ! -f "$NEG_INT_PEM" || ! -f "$NEG_FURTHER_PEM" \
   || ! -f "$NEG_LEAF_PEM" ]]; then
  cat <<MSG
  [pending] CA material not present under $CA_DIR — the gated
  rcgen_ca_chain_verify.rs export did not run (OD_E03_CA_DIR producer step).
  Expected, written by the gated integration tests in Lima:

      $ROOT_PEM   $INT_PEM   $SVID_PEM          (positive, sub-claims 1+2)
      $NEG_ROOT_PEM
      $NEG_INT_PEM
      $NEG_FURTHER_PEM
      $NEG_LEAF_PEM   (negative pathLen=0 anchor, sub-claim 3)

  The gated tests prove the chain in-tree; this expectation captures the
  external openssl-verify proof. This run stays 'pending'.
MSG
  exit 0
fi

rc=0

# Sub-claim 1 — positive full chain verifies (exit 0, "OK").
capture chain_verify openssl verify -CAfile "$ROOT_PEM" -untrusted "$INT_PEM" "$SVID_PEM" || rc=1
evidence_contains chain_verify "OK" || rc=1

# Sub-claim 2 — leaf profile: exactly one spiffe URI SAN, CA:FALSE, digitalSignature critical.
capture svid_text openssl x509 -in "$SVID_PEM" -noout -text || rc=1
grep -qE 'URI:spiffe://overdrive\.local/job/.*/alloc/' "$EVIDENCE_DIR/svid_text.out" \
  && echo "  [PASS] svid carries a spiffe URI SAN" \
  || { echo "  [FAIL] svid missing spiffe URI SAN"; rc=1; }
# exactly one URI SAN (no second URI:)
[[ "$(grep -cE 'URI:' "$EVIDENCE_DIR/svid_text.out")" == "1" ]] \
  && echo "  [PASS] exactly one URI SAN" \
  || { echo "  [FAIL] expected exactly one URI SAN"; rc=1; }
# Not a CA: basicConstraints is either ABSENT (a cert with no basicConstraints
# is CA:FALSE per X.509) OR present with CA:FALSE — and never CA:TRUE. The
# falsifying observation is CA:TRUE; absence and explicit CA:FALSE both pass.
if grep -qE 'CA:TRUE' "$EVIDENCE_DIR/svid_text.out"; then
  echo "  [FAIL] leaf is CA:TRUE — a leaf must not be a CA"; rc=1
elif grep -qE 'CA:FALSE' "$EVIDENCE_DIR/svid_text.out"; then
  echo "  [PASS] leaf basicConstraints explicitly CA:FALSE"
else
  echo "  [PASS] leaf carries no basicConstraints (CA:FALSE by X.509 default — never a CA)"
fi
# digitalSignature keyUsage marked critical
grep -A1 -E 'X509v3 Key Usage:.*critical' "$EVIDENCE_DIR/svid_text.out" \
  | grep -qE 'Digital Signature' \
  && echo "  [PASS] keyUsage digitalSignature marked critical" \
  || { echo "  [FAIL] keyUsage digitalSignature not marked critical"; rc=1; }

# Sub-claim 3 (MANDATORY negative anchor) — a leaf signed by a further-CA that
# is itself signed by the pathLen=0 intermediate FAILS openssl verify because
# the further-CA exceeds the intermediate's pathLen=0 budget. The leaf below
# the further-CA is load-bearing: openssl only counts the pathLen budget for
# CAs that sit as INTERMEDIATES on the path to an end-entity, so the further-CA
# must have a leaf below it to be counted. A non-zero exit here is the PASS
# condition, AND the failure must cite the pathLen/depth constraint — not a
# signature mismatch — to prove pathLen is the thing enforced.
neg_rc=0
capture pathlen_negative \
  openssl verify -CAfile "$NEG_ROOT_PEM" \
  -untrusted "$NEG_INT_PEM" -untrusted "$NEG_FURTHER_PEM" "$NEG_LEAF_PEM" \
  || neg_rc=$?
if [[ "$neg_rc" == "0" ]]; then
  echo "  [FAIL] pathLen=0 anchor: leaf-under-further-CA chain VERIFIED (exit 0) — pathLen NOT enforced"
  rc=1
elif grep -qiE 'path length|pathlen|basic constraints|excluded|exceeded' "$EVIDENCE_DIR/pathlen_negative.out"; then
  echo "  [PASS] pathLen=0 anchor: chain FAILED openssl verify (exit $neg_rc) on the pathLen/basicConstraints check — pathLen ENFORCED"
else
  # Failed, but not visibly on pathLen. Surface the actual reason so the
  # different-fox audit can judge it (a signature failure would prove the
  # WRONG thing — it would mean the chain did not even link).
  reason="$(grep -iE 'error|fail' "$EVIDENCE_DIR/pathlen_negative.out" | head -3 | tr '\n' ';')"
  echo "  [WARN] pathLen=0 anchor: chain FAILED openssl verify (exit $neg_rc) but not on a"
  echo "         recognised pathLen message — actual reason: ${reason}"
  echo "         (a signature failure here would prove the wrong thing; review the evidence)"
  rc=1
fi

exit $rc
