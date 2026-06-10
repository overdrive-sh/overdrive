# shellcheck shell=bash
# D01 — root CA private key is never plaintext at rest (byte-scan the IntentStore).
#
# Black-box: drives the BUILT `overdrive serve` binary inside Lima (real kernel,
# real redb IntentStore, production `SystemdCredsKeyring` KEK provider reading
# `$CREDENTIALS_DIRECTORY/overdrive-ca-root`). No `overdrive-*` crate is linked.
# #215 wired `boot_ca` into `run_server`, so first boot now generates +
# KEK-seals + persists the root to the on-disk IntentStore.
#
# WHY a STRUCTURAL byte-scan (not a known-key content scan):
# ---------------------------------------------------------------------------
# The first-boot root key is randomly generated (OsEntropy), so this black-box
# runner CANNOT know the specific key`s bytes a priori — and MUST NOT: a binary
# that leaked the generated key to the runner would itself violate D01. A
# faithful "scan the on-disk file for THIS key`s plaintext" therefore cannot be
# expressed black-box without inventing serve-binary surface (a fixture-key boot
# flag or a test-only key export), which CLAUDE.md forbids.
#
# The honest black-box proxy is to scan for the STRUCTURAL markers that ANY
# plaintext P-256 private-key serialization MUST carry in the clear, regardless
# of the key`s random scalar:
#
#   * PEM armor (ASCII):  `EC PRIVATE KEY` (SEC1) and `PRIVATE KEY` (PKCS#8).
#   * Raw DER OID runs (binary), present in every unencrypted EC key`s
#     AlgorithmIdentifier / ECPrivateKey body:
#       - id-ecPublicKey  1.2.840.10045.2.1  -> 06 07 2A 86 48 CE 3D 02 01
#       - prime256v1      1.2.840.10045.3.1.7 -> 06 08 2A 86 48 CE 3D 03 01 07
#     A SEC1 `ECPrivateKey` carries the curve OID in its `parameters [0]`
#     context tag; a PKCS#8 `PrivateKeyInfo` carries id-ecPublicKey +
#     prime256v1 in its `privateKeyAlgorithm`. Either serialization emits at
#     least one of these OID byte runs in the clear.
#
# A sealed AES-256-GCM envelope is high-entropy ciphertext: the probability of
# either fixed 9/10-byte OID run appearing by chance in the sealed blob is
# ~2^-72 / 2^-80, i.e. zero in practice. So:
#   - a leaked plaintext EC private key (armored OR unarmored raw DER) WILL
#     match one of these markers;
#   - the sealed envelope WILL NOT.
# This closes the raw-DER vacuous-pass gap the prior PEM-only scan left open
# (an unarmored DER key carries no `-----BEGIN`).
#
# NOTE: the persisted PUBLIC root certificate (X.509) ALSO carries the
# `prime256v1` / `id-ecPublicKey` OIDs in its SubjectPublicKeyInfo — a P-256
# cert legitimately advertises its curve in the clear. So the OID-run scan is
# applied to the IntentStore file with the CERTIFICATE PEM blocks EXCISED first
# (a public cert is allowed to carry the curve OID; a private key is not). The
# excision keeps the scan honest: it must not fire on the legitimate public
# cert, only on a private-key body.
source "$REPO_ROOT/verification/harness/lima-helpers.sh"

in_lima bash -lc '
set -uo pipefail
WORK="$(mktemp -d)"
DATA="$WORK/data"; CONF="$WORK/conf"; CREDS="$WORK/creds"
mkdir -p "$DATA" "$CONF" "$CREDS"
KEK_ID="overdrive-ca-root"
INTENT="$DATA/intent.redb"
head -c 32 /dev/zero | tr "\0" "\\377" > "$CREDS/$KEK_ID"   # 32 raw bytes (0xFF)

serve() { # <timeout_secs>
  CREDENTIALS_DIRECTORY="$CREDS" OVERDRIVE_CONFIG_DIR="$CONF" \
    timeout --preserve-status -s INT "${1}s" \
    cargo run -q -p overdrive-cli --bin overdrive -- serve --bind 127.0.0.1:0 --data-dir "$DATA" \
    >> "$WORK/serve.out" 2>&1 || true
}

rc=0

# The scan function the whole expectation rests on. Defined ONCE here so the
# inline self-test below and the production scan further down exercise the
# SAME code. Binary-safe single pass in Python (grep -c on a binary blob
# mis-counts). Excises CERTIFICATE PEM blocks FIRST — a P-256 public cert
# legitimately carries the curve OID in its SubjectPublicKeyInfo, so the OID
# scan must not fire on it; a leaked raw-DER private key lives OUTSIDE any
# CERTIFICATE block, so excision never hides it. Echoes "<armor> <der_oid_runs>".
scan_plaintext_key() { # <path>
  python3 - "$1" <<"PY"
import sys, re
b = open(sys.argv[1], "rb").read()
# Excise public certs (a P-256 cert legitimately carries the curve OID).
b = re.sub(rb"-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----", b"", b, flags=re.S)
# PEM armor: SEC1 (EC PRIVATE KEY) and PKCS#8 (PRIVATE KEY). The PRIVATE KEY
# substring is a superset covering both, so either serialization trips it.
armor = b.count(b"PRIVATE KEY")
# Raw-DER OID byte runs: id-ecPublicKey (1.2.840.10045.2.1) and
# prime256v1 (1.2.840.10045.3.1.7). Every unencrypted EC private key emits at
# least one in the clear; sealed AEAD ciphertext (high-entropy) will not.
oid_pub   = b.count(bytes.fromhex("06072a8648ce3d0201"))
oid_curve = b.count(bytes.fromhex("06082a8648ce3d030107"))
print("%d %d" % (armor, oid_pub + oid_curve))
PY
}

# scan_hits <path> -> echoes total marker count (armor + der-oid-runs), 0 = clean.
scan_hits() { # <path>
  read -r a o <<<"$(scan_plaintext_key "$1")"
  echo $(( a + o ))
}

# === INLINE NON-VACUITY SELF-TEST (printed to run.log) =======================
# Before trusting a CLEAN result on the production IntentStore, PROVE the scan
# is non-vacuous RIGHT HERE: it must DETECT a real plaintext P-256 key in all
# four serializations, and stay CLEAN on benign inputs (a real public cert
# post-excision, and high-entropy random bytes). A throwaway key is generated
# in this temp space only and NEVER touches the real IntentStore. openssl is
# available in Lima. If ANY positive control fails to detect, or ANY negative
# control falsely matches, the self-test FAILS the runner (hard gate).
echo "  --- non-vacuity self-test: prove the byte-scan detects real keys ---"
ST="$WORK/selftest"; mkdir -p "$ST"
selftest_fail=0

# Throwaway P-256 key (SEC1 PEM), then derive the other three serializations.
openssl ecparam -name prime256v1 -genkey -noout -out "$ST/sec1.pem" 2>/dev/null
openssl pkcs8 -topk8 -nocrypt -in "$ST/sec1.pem" -out "$ST/pkcs8.pem" 2>/dev/null
openssl ec    -in "$ST/sec1.pem"  -outform DER -out "$ST/sec1.der"  2>/dev/null
openssl pkcs8 -topk8 -nocrypt -in "$ST/sec1.pem" -outform DER -out "$ST/pkcs8.der" 2>/dev/null

# Positive controls — the scan MUST report a non-zero marker count for each form.
for form in sec1.pem:SEC1-PEM pkcs8.pem:PKCS8-PEM sec1.der:raw-SEC1-DER pkcs8.der:raw-PKCS8-DER; do
  file="${form%%:*}"; name="${form##*:}"
  hits="$(scan_hits "$ST/$file")"
  if [ "$hits" -gt 0 ]; then
    echo "  [selftest PASS] detects $name (markers=$hits)"
  else
    echo "  [selftest FAIL] scan BLIND to $name (markers=0) — scan is vacuous"; selftest_fail=1
  fi
done

# Negative control (i): a REAL P-256 PUBLIC cert, AFTER cert excision, must scan
# CLEAN — proving excision does not blind the scan to a key (the key is outside
# CERT blocks; the cert merely advertises its curve and is legitimately excised).
openssl req -x509 -new -key "$ST/sec1.pem" -days 1 -subj "/CN=selftest" \
  -out "$ST/cert.pem" 2>/dev/null
cert_hits="$(scan_hits "$ST/cert.pem")"
if [ "$cert_hits" -eq 0 ]; then
  echo "  [selftest PASS] clean on public-cert-post-excision (markers=0)"
else
  echo "  [selftest FAIL] scan fired on a public cert (markers=$cert_hits) — excision broken / false-positive"; selftest_fail=1
fi

# Negative control (ii): high-entropy random bytes (ciphertext-sized) must scan
# CLEAN — proving the OID runs do not appear by accident in sealed-envelope-like
# data.
openssl rand 4096 > "$ST/random.bin" 2>/dev/null
rand_hits="$(scan_hits "$ST/random.bin")"
if [ "$rand_hits" -eq 0 ]; then
  echo "  [selftest PASS] clean on random-ciphertext-sized-bytes (markers=0)"
else
  echo "  [selftest FAIL] scan fired on random bytes (markers=$rand_hits) — false-positive"; selftest_fail=1
fi

if [ "$selftest_fail" -ne 0 ]; then
  echo "  [FAIL] non-vacuity self-test FAILED — scan cannot be trusted; aborting"
  rm -rf "$WORK"; exit 1
fi
echo "  [PASS] scan proven non-vacuous (4/4 detected, 2/2 benign clean)"
rm -rf "$ST"
# === end self-test ===========================================================

# --- First boot: generate + seal + persist the root. -------------------------
serve 25
if [ ! -f "$INTENT" ]; then
  echo "  [FAIL] first boot did not persist $INTENT"
  sed -n "1,40p" "$WORK/serve.out"
  rm -rf "$WORK"; exit 1
fi
echo "  [PASS] first boot persisted the IntentStore at $INTENT"

# Production byte-scan: now that the scan is proven non-vacuous above, scan the
# REAL on-disk IntentStore for plaintext-private-key markers.
echo "  --- production IntentStore scan (scan proven non-vacuous above) ---"

# Sub-claim 1 — zero plaintext private-key markers (armored OR raw-DER) on disk.
read -r armor1 oid1 <<<"$(scan_plaintext_key "$INTENT")"
if [ "$armor1" -gt 0 ] || [ "$oid1" -gt 0 ]; then
  echo "  [FAIL] sub-claim 1: plaintext private-key markers found on disk (PEM-armor=$armor1 DER-OID-runs=$oid1)"; rc=1
else
  echo "  [PASS] sub-claim 1: zero plaintext private-key markers in $INTENT (PEM-armor=$armor1 DER-OID-runs=$oid1; certs excised before scan)"
fi

# Sub-claim 2 — the sealed envelope IS present (key sealed, not absent): the
# IntentStore carries the envelope key marker and is non-trivially sized.
if grep -aq "key-envelope" "$INTENT" 2>/dev/null && [ -s "$INTENT" ]; then
  echo "  [PASS] sub-claim 2: sealed root-key envelope present in the IntentStore"
else
  echo "  [FAIL] sub-claim 2: no sealed root-key envelope present"; rc=1
fi

# --- Restart (decrypt + adopt the SAME root), re-scan. -----------------------
serve 25
read -r armor2 oid2 <<<"$(scan_plaintext_key "$INTENT")"
if [ "$armor2" -gt 0 ] || [ "$oid2" -gt 0 ]; then
  echo "  [FAIL] sub-claim 3: plaintext private-key markers appeared after restart (PEM-armor=$armor2 DER-OID-runs=$oid2)"; rc=1
else
  echo "  [PASS] sub-claim 3: still zero plaintext private-key markers after restart (PEM-armor=$armor2 DER-OID-runs=$oid2)"
fi

rm -rf "$WORK"
exit $rc
'
