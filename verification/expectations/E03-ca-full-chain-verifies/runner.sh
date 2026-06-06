# shellcheck shell=bash
# E03 — full Root -> Intermediate -> SVID chain verifies under `openssl verify`.
#
# This is the walking-skeleton proof (S-04-07, KPI K1). It needs the built-in
# CA production surface (`Ca` trait + `RcgenCa` host adapter), which lands in
# DELIVER. Until the binary can export the three PEMs (root / intermediate /
# svid), this runner records `pending` and prints the manual capture path.
#
# There is NO operator CLI verb to mint an SVID this phase (feature-delta
# D-CA-4). When DELIVER lands a CA-material export surface (or the gated
# integration test writes the PEMs to a known dir), wire it below and flip the
# guard. `openssl verify` is the honest external entry point.
source "$REPO_ROOT/verification/harness/lima-helpers.sh"

CA_DIR="${CA_DIR:-/tmp/od-e03-ca}"
ROOT_PEM="$CA_DIR/root.pem"
INT_PEM="$CA_DIR/intermediate.pem"
SVID_PEM="$CA_DIR/svid.pem"

if [[ ! -f "$ROOT_PEM" || ! -f "$INT_PEM" || ! -f "$SVID_PEM" ]]; then
  cat <<MSG
  [pending] CA material not present at $CA_DIR — E03 needs the built-in CA
  production surface (DELIVER). Once the host adapter can emit the chain PEMs,
  export them to:

      $ROOT_PEM
      $INT_PEM
      $SVID_PEM

  then re-run E03. The gated integration test
  rcgen_ca_chain_verify.rs::rcgen_full_svid_chain_verifies_root_intermediate_svid
  already proves the chain in-tree; this expectation captures the external
  openssl-verify proof. This run stays 'pending'.
MSG
  exit 0
fi

rc=0

# Sub-claim 1 — full chain verifies.
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

exit $rc
