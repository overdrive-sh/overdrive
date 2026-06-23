# shellcheck shell=bash
# E04 — a mesh workload is reachable at its canonical address over mTLS, e2e.
#
# Tier-3 / black-box surface: this expectation needs a converged full-system
# deployment of TWO mesh workloads (server + client) on a real node, with the
# PRODUCTION workload-identity CA issuing the leg-C/leg-B SVIDs (no in-process
# `mtls_identity_override` test seam), driven through the BUILT `overdrive`
# binary (`overdrive serve` + two `overdrive deploy`). That full-system harness
# is #227 (disposable full-system Lima VM on the immutable OS) on #75 (the OS
# image). NEITHER has landed, so this runner self-reports `pending` rather than
# narrate a capture it cannot execute.
#
# Black-box only: surfaces are the `od` CLI + what the kernel exposes
# (`ss`, `nft list`, `tcpdump`). This runner does NOT link any `overdrive-*`
# crate — when the precondition lands it drives the `od serve` + `od deploy ×2`
# shape sketched below and observes the wire/CLI, nothing more.
#
# The in-process Tier-3 keystone
# `crates/overdrive-control-plane/tests/integration/canonical_address_inbound_walking_skeleton.rs`
# is the `what, forever` witness for the round-trip through the production-
# installed inbound rule (with a test PKI seam); E04 is the black-box operator-
# observable `why` those tiers under-serve.

source "$REPO_ROOT/verification/harness/lima-helpers.sh"

SERVER_SPEC="examples/canonical-addr-server.toml"
CLIENT_SPEC="examples/canonical-addr-client.toml"

# --- Precondition gate: the full-system #227 harness + #75 OS image -----------
# Until #227 (full-system Lima VM on the immutable OS) and #75 (Image Factory
# MVP) land, the production CA → SVID → leg-C mTLS path cannot be driven
# black-box with two converged workloads. Leave every sub-claim `pending`.
echo "  [pending] E04 needs the full-system EDD harness (#227) on the OS image (#75):"
echo "            a converged two-workload deploy with the PRODUCTION workload-identity"
echo "            CA issuing leg-C/leg-B SVIDs (no in-process test seam), driven through"
echo "            the built 'overdrive serve' + 'overdrive deploy ×2'. Neither landed yet,"
echo "            so this runner reports pending and captures no fabricated evidence."
echo "            The in-process 'what, forever' witness is"
echo "            crates/overdrive-control-plane/tests/integration/canonical_address_inbound_walking_skeleton.rs."
exit 0

# === SHAPE WHEN #227 + #75 LAND (not executed today) =========================
# In a #227 full-system Lima VM, as root, with the built binary:
#
#   # 0. boot the node (separate Lima-routed terminal):
#   #      overdrive serve --bind 127.0.0.1:7443 --data-dir /tmp/od-e04
#   #
#   # 1. deploy server + client mesh workloads; both must Accept.
#   capture deploy_server od deploy "$SERVER_SPEC"
#   capture deploy_client od deploy "$CLIENT_SPEC"
#   #    assert both .meta exit 0 and .out contains "Accepted."
#   #
#   # 2. discover the server's canonical workload_addr via `od alloc status`,
#   #    then from the client workload's netns dial workload_addr:service_port
#   #    DIRECTLY (no DNS) and assert the byte-exact application round-trip.
#   capture server_alloc_status od alloc status <server-alloc>
#   #
#   # 3. confidentiality: capture the leg-C/leg-B wire and assert TLS-1.3
#   #    application_data (0x17) records in both directions; plaintext markers
#   #    never appear on the encrypted wire.
#   capture wire_legc in_lima tcpdump -ni lo -c 80 'tcp port <service_port>'
#
# Then set Status: satisfied ONLY after a different-fox adversarial audit of the
# captured evidence/ (per .claude/rules/verification.md). Do NOT self-stamp.
