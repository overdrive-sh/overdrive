#!/usr/bin/env bash
set -euo pipefail

[[ "$#" -eq 2 ]] || {
  echo "usage: $0 <absolute-scratch-dir> <hostname>" >&2
  exit 2
}

readonly scratch_dir="$1"
readonly hostname="$2"
[[ "$scratch_dir" = /* ]] || {
  echo "prepare-test-ca: scratch directory must be absolute" >&2
  exit 2
}

install -d -m 0700 "$scratch_dir"
readonly ca_key="$scratch_dir/test-ca-key.pem"
readonly ca_cert="$scratch_dir/test-ca.pem"
readonly leaf_key="$scratch_dir/api-origin-key.pem"
readonly leaf_csr="$scratch_dir/api-origin.csr"
readonly leaf_cert="$scratch_dir/api-origin.pem"
readonly chain="$scratch_dir/api-origin-chain.pem"
readonly extensions="$scratch_dir/api-origin.ext"

openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "$ca_key"
openssl req -x509 -new -sha256 -key "$ca_key" -days 2 \
  -subj "/CN=Public Ingress Test CA" -out "$ca_cert"
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "$leaf_key"
openssl req -new -sha256 -key "$leaf_key" -subj "/CN=$hostname" -out "$leaf_csr"
{
  echo "basicConstraints=critical,CA:FALSE"
  echo "keyUsage=critical,digitalSignature"
  echo "extendedKeyUsage=serverAuth"
  echo "subjectAltName=DNS:$hostname"
} >"$extensions"
openssl x509 -req -sha256 -in "$leaf_csr" -CA "$ca_cert" -CAkey "$ca_key" \
  -CAcreateserial -days 2 -extfile "$extensions" -out "$leaf_cert"

{
  sed -n '/-----BEGIN CERTIFICATE-----/,/-----END CERTIFICATE-----/p' "$leaf_cert"
  sed -n '/-----BEGIN CERTIFICATE-----/,/-----END CERTIFICATE-----/p' "$ca_cert"
} >"$chain"
chmod 0600 "$ca_key" "$leaf_key"
chmod 0644 "$ca_cert" "$leaf_cert" "$chain"
rm -f -- "$leaf_csr" "$extensions" "$scratch_dir/test-ca.srl"

printf 'test_ca=%s\ncertificate_chain=%s\nprivate_key=%s\n' \
  "$ca_cert" "$chain" "$leaf_key"
