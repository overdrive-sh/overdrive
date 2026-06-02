# shellcheck shell=bash
# E02 — a deployed UDP service's reply is sourced from the VIP, not the backend IP.
#
# Tier-3 surface: the REVERSE_NAT_MAP dump and the wire capture both require the
# real overdrive-testing ThreeIfaceTopology netns/veth setup + a running
# dataplane, run as root inside Lima. This runner is honest about topology: it
# checks what it can reach, captures whatever real evidence is available
# (including an honest *absent* map dump as negative evidence), and leaves the
# topology-bound sub-claims `pending` rather than narrate them.
#
# Black-box only: surfaces are the `od` CLI and what the kernel exposes
# (`bpftool map dump`, `tcpdump`). This runner does NOT run
# reverse_nat_udp_e2e.rs — that test links overdrive-* crates and would forfeit
# the black-box independence; it is the Tier-3 `what, forever` witness, not
# evidence this runner produces.
source "$REPO_ROOT/verification/harness/lima-helpers.sh"

VIP="10.96.0.10"
VIP_PORT="15353"
SPEC="examples/dns-resolver.toml"

if [[ ! -f "$REPO_ROOT/$SPEC" ]]; then
  echo "  [pending] fixture missing: $SPEC"
  exit 0
fi

rc=0

# --- Best-effort REVERSE_NAT_MAP dump (CP-independent, honest negative) -------
# Sub-claim 2 is the D (dataplane/kernel) surface. Capture `bpftool map dump
# name REVERSE_NAT_MAP` UNCONDITIONALLY up front: it needs a running dataplane
# that pinned the map, which on this VM never exists (production serve's
# EbpfDataplane XDP attach to `lo` fails at boot — see O03/evidence/serve.log —
# so no map is ever programmed). The dump therefore fails with "Error: can't
# find map" — that absent dump IS honest negative evidence for sub-claim 2, and
# we keep the sub-claim `pending` (never synthesize a pass). We do NOT stand up
# the ThreeIfaceTopology veth/dataplane to make the map appear: that re-builds a
# test tier (forbidden by verification.md) and risks leaked XDP/cgroups on the
# shared VM. The Tier-3 `what, forever` witness for the (backend_ip,5353,udp)->VIP
# key is reverse_nat_udp_e2e.rs.
capture reverse_nat_map_dump_preflight in_lima bpftool map dump name REVERSE_NAT_MAP || true
revnat_pre_rc="$(sed -n 's/^# exit:[[:space:]]*//p' "$EVIDENCE_DIR/reverse_nat_map_dump_preflight.meta")"
if [[ "$revnat_pre_rc" == "0" ]] && grep -qiF "udp" "$EVIDENCE_DIR/reverse_nat_map_dump_preflight.out" \
   && grep -qF "$VIP_PORT" "$EVIDENCE_DIR/reverse_nat_map_dump_preflight.out"; then
  echo "  [candidate-PASS] sub-claim 2 (preflight): REVERSE_NAT_MAP present with udp/$VIP_PORT entry"
  echo "                   (adversarial review must confirm the (backend_ip,$VIP_PORT,udp)->VIP key)"
else
  echo "  [pending] sub-claim 2 (preflight): REVERSE_NAT_MAP not dumpable (rc=$revnat_pre_rc) —"
  echo "            no running dataplane pinned the map (production serve's XDP attach"
  echo "            to lo fails at boot on this VM; see O03 evidence/serve.log). Captured"
  echo "            the absent/failed dump as honest negative evidence. The Tier-3 witness"
  echo "            for the (backend_ip,$VIP_PORT,udp)->VIP key is reverse_nat_udp_e2e.rs."
fi

# --- Sub-claim 1 (precondition, inherits O03): deploy accepted. -------------
if ! capture preflight_cluster od cluster status; then
  cat <<MSG
  [pending] control plane not reachable — E02 needs a running deployment AND
  the ThreeIfaceTopology veth setup. In a separate Lima-routed terminal:

      cargo overdrive serve --bind 127.0.0.1:7443 --data-dir /tmp/od-e02

  then re-run E02. This run captured the preflight failure (and the absent
  REVERSE_NAT_MAP dump above) as evidence and is leaving every sub-claim
  'pending'.
MSG
  exit 0
fi

capture deploy_dns_resolver od deploy "$SPEC" || true
deploy_rc="$(sed -n 's/^# exit:[[:space:]]*//p' "$EVIDENCE_DIR/deploy_dns_resolver.meta")"
if [[ "$deploy_rc" == "0" ]] && grep -qF -- "Accepted." "$EVIDENCE_DIR/deploy_dns_resolver.out"; then
  echo "  [PASS] sub-claim 1: deploy exited 0 and printed Accepted."
else
  echo "  [FAIL] sub-claim 1: deploy did not cleanly accept (rc=$deploy_rc)"
  rc=1
fi

# --- Sub-claim 2 (D — dataplane/kernel): REVERSE_NAT_MAP carries the key. ----
# Honest negative evidence: if the dataplane/topology is not up, the map is
# absent and the dump fails — we capture that verbatim and leave the sub-claim
# `pending`. We do NOT synthesize a pass.
capture reverse_nat_map_dump in_lima bpftool map dump name REVERSE_NAT_MAP || true
revnat_rc="$(sed -n 's/^# exit:[[:space:]]*//p' "$EVIDENCE_DIR/reverse_nat_map_dump.meta")"
if [[ "$revnat_rc" == "0" ]] && grep -qiF "udp" "$EVIDENCE_DIR/reverse_nat_map_dump.out" \
   && grep -qF "$VIP_PORT" "$EVIDENCE_DIR/reverse_nat_map_dump.out"; then
  echo "  [candidate-PASS] sub-claim 2: REVERSE_NAT_MAP dump present with udp/$VIP_PORT entry"
  echo "                   (adversarial review must confirm the (backend_ip,$VIP_PORT,udp)->VIP key)"
else
  echo "  [pending] sub-claim 2: REVERSE_NAT_MAP not dumpable (rc=$revnat_rc) —"
  echo "            no running dataplane/topology. Captured the absent/failed dump"
  echo "            as honest negative evidence. The Tier-3 witness for the"
  echo "            (backend_ip,$VIP_PORT,udp)->VIP key is reverse_nat_udp_e2e.rs."
fi

# --- Sub-claim 3 (E — wire): reply sourced from the VIP, not the backend IP. -
# This requires the ThreeIfaceTopology veth + a live UDP round-trip + tcpdump on
# the client veth. Standing that up safely in-runner (netns, veth, route,
# sysctl, then a background tcpdump with a cleanup trap) is the exact Tier-3
# setup overdrive-testing owns and reverse_nat_udp_e2e.rs drives. We do not
# reproduce it black-box here (the topology helpers are crate code, and a
# half-built veth + leaked XDP across runs is a documented hazard); we leave the
# sub-claim `pending` and point at the regression alarm.
echo "  [pending] sub-claim 3: VIP-sourced reply ($VIP:$VIP_PORT) wire capture"
echo "            requires the ThreeIfaceTopology veth setup + live UDP round-trip."
echo "            The Tier-3 'what, forever' witness (wire source == VIP, #163 guard)"
echo "            is crates/overdrive-dataplane/tests/integration/reverse_nat_udp_e2e.rs."

echo "E02 sub-claim aggregate exit: $rc  (sub-claims 2 & 3 topology-bound -> pending unless dataplane up)"
exit "$rc"
