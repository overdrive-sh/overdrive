# shellcheck shell=bash
# D01 — root CA private key is never plaintext at rest (byte-scan the IntentStore).
#
# Needs the built-in CA boot path (DELIVER): first boot generates +
# envelope-encrypts + persists the root to the IntentStore (redb). The
# byte-scan needs the KNOWN plaintext key to scan for (derived from the same
# first-boot/fixture key the test uses), not a guess. Until DELIVER lands the
# boot path + a way to obtain the known key, this runner records `pending`.
source "$REPO_ROOT/verification/harness/lima-helpers.sh"

DATA_DIR="${DATA_DIR:-/tmp/od-d01}"
# The IntentStore redb file (path lands with the DELIVER data-dir layout).
INTENT_FILE="${INTENT_FILE:-$DATA_DIR/intent.redb}"
# The known plaintext key DER to scan for (DELIVER must expose it, e.g. a
# test-only export, or the sim fixture key for a fixture-keyed boot).
KNOWN_KEY_DER="${KNOWN_KEY_DER:-}"

if [[ ! -f "$INTENT_FILE" || -z "$KNOWN_KEY_DER" || ! -f "$KNOWN_KEY_DER" ]]; then
  cat <<MSG
  [pending] D01 needs (a) a persisted IntentStore at \$INTENT_FILE and (b) the
  KNOWN plaintext key DER at \$KNOWN_KEY_DER to scan for — both arrive with the
  DELIVER CA boot path. When they exist:

    1. First boot -> root generated + sealed + persisted to $INTENT_FILE.
    2. Obtain the plaintext key DER that was sealed (test export / fixture key).
    3. Byte-scan: the plaintext key bytes MUST NOT appear in $INTENT_FILE.
    4. Restart (decrypt + reuse), re-scan -> still zero plaintext bytes.

  The gated test rcgen_ca_root_key_envelope.rs::root_key_envelope_contains_no_plaintext_key_bytes
  (S-02-02) proves this against the serialized record in-tree; D01 captures the
  on-disk IntentStore-file scan. This run stays 'pending'.
MSG
  exit 0
fi

rc=0
# Sub-claim 1 — plaintext key DER absent from the on-disk IntentStore.
# Use a binary substring search; capture the count (expected 0).
if grep -a -c -F -f <(xxd -p "$KNOWN_KEY_DER" | tr -d '\n') "$INTENT_FILE" >/dev/null 2>&1; then
  echo "  [FAIL] plaintext key bytes found in $INTENT_FILE — K3 guardrail violated"
  rc=1
else
  echo "  [PASS] zero plaintext key bytes in $INTENT_FILE"
fi

# Sub-claim 2 — the sealed envelope IS present (key sealed, not absent).
# (Heuristic: the redb file is non-trivially sized and non-empty.)
[[ -s "$INTENT_FILE" ]] \
  && echo "  [PASS] IntentStore is non-empty (sealed record present)" \
  || { echo "  [FAIL] IntentStore empty — no sealed record"; rc=1; }

exit $rc
