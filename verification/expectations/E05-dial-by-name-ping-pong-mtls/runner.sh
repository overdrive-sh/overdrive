# shellcheck shell=bash
# E05 — two services dial each other by name; counters advance; each hop mTLS'd.
#
# Tier-3 / black-box surface: this expectation needs a converged full-system
# deployment of TWO mesh workloads (a + b) on a real node, with the PRODUCTION
# workload-identity CA issuing the leg-C/leg-B SVIDs (no in-process
# `mtls_identity_override` test seam), driven through the BUILT `overdrive`
# binary (`overdrive serve` + two `overdrive deploy`). That full-system harness
# is #227 (disposable full-system Lima VM on the immutable OS) on #75 (the OS
# image). NEITHER has landed, so this runner self-reports `pending` rather than
# narrate a capture it cannot execute. (Same posture as the sibling E04.)
#
# Black-box only: surfaces are the `od` CLI + what the kernel exposes
# (`ss`, `getent`, `tcpdump`). This runner does NOT link any `overdrive-*`
# crate — when the precondition lands it drives the `od serve` + `od deploy ×2`
# shape sketched below (the client program is the checked-in `ping_pong.py` the
# specs already point at — no staging step) and observes the wire/CLI/counters,
# nothing more.
#
# The in-process Tier-3 witnesses are
# `crates/overdrive-control-plane/tests/integration/dns_responder_walking_skeleton.rs`
# (single-direction dial-by-name loop, GREEN) and
# `crates/overdrive-control-plane/tests/integration/dns_responder_ping_pong.rs`
# (the bidirectional proof — a REAL GREEN #[tokio::test], review-03-02.md
# resolution (a)); E05 is the black-box operator-observable `why` those tiers
# under-serve.

source "$REPO_ROOT/verification/harness/lima-helpers.sh"

SPEC_A="examples/dial-by-name-responder/a.toml"
SPEC_B="examples/dial-by-name-responder/b.toml"
PING_PONG_SCRIPT="examples/dial-by-name-responder/ping_pong.py"  # checked-in client program both specs run

# --- Precondition gate: the full-system #227 harness + #75 OS image -----------
# Until #227 (full-system Lima VM on the immutable OS) and #75 (Image Factory
# MVP) land, the production CA → SVID → leg-C/leg-B mTLS path cannot be driven
# black-box with two converged workloads dialing each other. Leave every
# sub-claim `pending`.
echo "  [pending] E05 needs the full-system EDD harness (#227) on the OS image (#75):"
echo "            a converged two-workload (a + b) deploy with the PRODUCTION workload-identity"
echo "            CA issuing leg-C/leg-B SVIDs (no in-process test seam), driven through the"
echo "            built 'overdrive serve' + 'overdrive deploy a.toml' + 'overdrive deploy b.toml'."
echo "            Neither landed yet, so this runner reports pending and captures no fabricated"
echo "            evidence. The in-process 'what, forever' witnesses are"
echo "            crates/overdrive-control-plane/tests/integration/dns_responder_walking_skeleton.rs"
echo "            and .../dns_responder_ping_pong.rs."
exit 0

# === SHAPE WHEN #227 + #75 LAND (not executed today) =========================
# In a #227 full-system Lima VM, as root, with the built binary:
#
#   # 0. no staging step — the client program is the checked-in
#   #    "$PING_PONG_SCRIPT" both specs already run via /usr/bin/python3.
#   #
#   # 1. boot the node (separate Lima-routed terminal). Launch `overdrive
#   #    serve` from the REPO ROOT ($REPO_ROOT) — the a/b specs' `command`
#   #    runs the script by the repo-root-relative path
#   #    "examples/dial-by-name-responder/ping_pong.py", and `ExecDriver` sets
#   #    no `current_dir` (it enters only CLONE_NEWNET, no mount ns), so the
#   #    workload inherits serve's cwd. Pin it so the capture can't drift on the
#   #    relative path:
#   #      cd "$REPO_ROOT" && overdrive serve --bind 127.0.0.1:7443 --data-dir /tmp/od-e05
#   #
#   # 2. deploy BOTH halves; both must Accept.
#   capture deploy_a od deploy "$SPEC_A"
#   capture deploy_b od deploy "$SPEC_B"
#   #    assert both .meta exit 0 and .out contains "Accepted."
#   #
#   # 3. from A's netns getent b.svc.overdrive.local → F ∈ 10.98.0.0/16 (NOT dig,
#   #    NOT a 10.99.0.0/16 backend); symmetrically from B's netns getent a.svc.
#   capture resolve_b_from_a in_lima ip netns exec <a-ns> getent ahostsv4 b.svc.overdrive.local
#   capture resolve_a_from_b in_lima ip netns exec <b-ns> getent ahostsv4 a.svc.overdrive.local
#   #
#   # 4. observe BOTH counters advance over a 60s window on a ~10s cadence
#   #    (scrape each workload's stdout / a CLI surface twice, ~60s apart, and
#   #    assert both inbound counts strictly increased).
#   capture counters_t0 od alloc status <a-alloc> <b-alloc>
#   sleep 60
#   capture counters_t1 od alloc status <a-alloc> <b-alloc>
#   #
#   # 5. confidentiality: capture EACH hop's inter-agent leg-B ↔ leg-C wire and
#   #    assert TLS-1.3 application_data (0x17) records in both directions; the
#   #    PLAINTEXT request/response markers never appear on the encrypted wire.
#   capture wire_a_to_b in_lima tcpdump -ni lo -c 80 'tcp port 18972'
#   capture wire_b_to_a in_lima tcpdump -ni lo -c 80 'tcp port 18971'
#
# Then set Status: satisfied ONLY after a different-fox adversarial audit of the
# captured evidence/ (per .claude/rules/verification.md). Do NOT self-stamp.
