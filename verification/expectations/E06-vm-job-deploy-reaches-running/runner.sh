# shellcheck shell=bash
# E06 — a `[job]` + `[vm]` spec deploys and its VM allocation reaches Running
# through the PRODUCTION `VmDriver` path (S-VM-39, black-box).
#
# EXECUTION SUBSTRATE IS THE BARE-METAL KVM BOX, **NOT** LIMA.
# S-VM-39 boots a real Cloud Hypervisor guest, which needs x86_64 + nested KVM.
# Lima on Apple Silicon cannot provide that, so every other runner's
# `cargo xtask lima run --` transport is unusable here. Per
# `.claude/rules/testing.md` § "Running tests — bare-metal KVM box (kvm-tests)"
# the canonical transport is `cargo xtask metal run --` against the host named
# by `OVERDRIVE_METAL_TARGET` (process env, else workspace-root `.env`).
# CONSEQUENCE, stated rather than hidden: the harness writes
# `executed_in_lima: <bool>` from whether THIS script ran, so a successful run
# stamps `executed_in_lima: true` while nothing ran in Lima. That field is
# literally inaccurate for E06; `evidence/execution_substrate.txt` records what
# actually executed where, and the README repeats it. Do not read the yaml
# field as a Lima claim for this expectation.
#
# BLACK-BOX ONLY. The surface is the BUILT `overdrive` binary's CLI
# (`serve` / `deploy` / `workload describe` / `job stop`) plus what the kernel
# exposes (`/proc`, cgroupfs, `/run`, `ip`, `bpftool`). No `overdrive-*` crate
# is imported or linked — `verification/` forfeits its independence the moment
# it does (`.claude/rules/verification.md` § Enforcement).
#
# PRODUCTION-DEFAULT BUILD, DELIBERATELY. The binary is built with DEFAULT
# features — no `integration-tests`, no `kvm-tests`. That is the whole point:
# K4 ("the production composition path can reach the VM driver via `overdrive
# serve` + `overdrive deploy`, with NO test-only wiring") is only measurable
# against the composition an operator actually gets. Adding the feature flag
# would measure the test harness instead.
#
# HYGIENE ON A SHARED BOX. The metal box is shared with other work and with
# the serialized `kvm-tests` lane. Teardown is DELTA-BASED, never a blanket
# sweep: the runner snapshots the hypervisor processes, allocation cgroup
# scopes, VM run directories and XDP attachments that exist BEFORE it starts,
# and on every exit path removes only what appeared DURING the run. A blanket
# `pkill cloud-hypervisor` / `for i in $(ip link)` sweep would kill a
# concurrent run's guest, so it is not used. The private rootfs clone, its loop
# device and its mount are unwound by the same trap.
source "$REPO_ROOT/verification/harness/lima-helpers.sh"

set -uo pipefail

EXPECTATION_SLUG="$(basename "$EXPECTATION_DIR")"
REMOTE_EVIDENCE="verification/expectations/$EXPECTATION_SLUG/evidence"

# The metal host is a real address in a gitignored `.env`; evidence is
# committed. Strip the host from anything captured so the catalogue never
# carries it. The box is identified in evidence by arch/kernel/CPU instead.
redact() { sed -E 's/@[A-Za-z0-9._-]+/@<metal-host-redacted>/g'; }

# `OVERDRIVE_METAL_TARGET` from the process env, else the workspace-root
# `.env` — the same two-source resolution `cargo xtask metal` itself uses
# (`xtask/src/main.rs`, `metal_target`).
resolve_metal_target() {
  if [[ -n "${OVERDRIVE_METAL_TARGET:-}" ]]; then
    printf '%s' "$OVERDRIVE_METAL_TARGET"
    return 0
  fi
  if [[ -f "$REPO_ROOT/.env" ]]; then
    sed -n 's/^OVERDRIVE_METAL_TARGET=//p' "$REPO_ROOT/.env" | tail -1
  fi
}

TARGET="$(resolve_metal_target)"

{
  echo "# E06 execution substrate"
  echo "#"
  echo "# transport:        cargo xtask metal run --   (NOT cargo xtask lima run --)"
  echo "# why:              S-VM-39 boots a real Cloud Hypervisor guest; that needs"
  echo "#                   x86_64 + nested KVM, which Lima on Apple Silicon cannot"
  echo "#                   provide (.claude/rules/testing.md § bare-metal KVM box)."
  echo "# target:           \$OVERDRIVE_METAL_TARGET (host redacted in evidence)"
  echo "# executed_in_lima: FALSE in fact. verification.yaml's field records only"
  echo "#                   that this runner executed, so it reads 'true' — that is"
  echo "#                   a harness limitation, not a Lima claim. This file and"
  echo "#                   the README are the accurate record."
  echo "# build:            cargo build -p overdrive-cli --bin overdrive"
  echo "#                   (DEFAULT features — no integration-tests, no kvm-tests)"
} > "$EVIDENCE_DIR/execution_substrate.txt"

if [[ -z "$TARGET" ]]; then
  {
    echo "BLOCKED: OVERDRIVE_METAL_TARGET is unset in the process environment and"
    echo "absent from $REPO_ROOT/.env (see .env.example)."
    echo "No fabricated evidence is written and NO Lima fallback is attempted:"
    echo "Lima cannot boot a KVM guest, so a Lima capture would be a different"
    echo "claim wearing this expectation's name."
  } > "$EVIDENCE_DIR/metal_preflight.out"
  echo "  [blocked] metal target unresolved — status stays 'pending'"
  exit 1
fi

# --- ssh preflight: is the box reachable, and is it the right box? -----------
SSH_OPTS=(-o StrictHostKeyChecking=accept-new -o ServerAliveInterval=30
          -o ConnectTimeout=15 -o BatchMode=yes)
# Un-redacted captures land OUTSIDE the evidence dir. `cargo xtask metal run`
# rsyncs this tree UP before executing, so a raw file sitting in evidence/ at
# that moment is pushed to the box and pulled back by the evidence rsync —
# re-introducing the very bytes redaction just removed.
RAW_TMP="$(mktemp -d "${TMPDIR:-/tmp}/e06-raw.XXXXXX")"
# shellcheck disable=SC2064  # expand RAW_TMP now, not at trap time
trap "rm -rf '$RAW_TMP'" EXIT

ssh "${SSH_OPTS[@]}" "$TARGET" 'echo REACHABLE; uname -srm; id -un' \
  > "$RAW_TMP/preflight" 2>&1
preflight_rc=$?
redact < "$RAW_TMP/preflight" > "$EVIDENCE_DIR/metal_preflight.out"
echo "# exit: $preflight_rc" >> "$EVIDENCE_DIR/metal_preflight.out"

if [[ "$preflight_rc" -ne 0 ]]; then
  echo "  [blocked] metal box unreachable (ssh exit $preflight_rc) — status stays 'pending'"
  echo "            evidence/metal_preflight.out carries the executed failure."
  exit 1
fi
echo "  [ok] metal box reachable"

# ---------------------------------------------------------------------------
# The whole bring-up runs as ONE root-context script on the metal box so the
# teardown trap spans staging + serve + deploy + stop + sweep. Host-side values
# are interpolated here; guest-side variables are `\$`-escaped. `cargo xtask
# metal run --` rsyncs the tree up (so $REMOTE_EVIDENCE exists remotely), then
# ssh-executes under `bash -lc` wrapped in `sudo … env HOME/PATH`, landing in
# `~/overdrive` as root — the permission surface KVM + cgroups need.
#
# Unlike Lima (virtiofs at the SAME absolute path), the metal tree lives at a
# DIFFERENT path in the guest, so the inner script writes evidence to the
# repo-RELATIVE remote path and the host rsyncs it back afterwards.
# ---------------------------------------------------------------------------
INNER=$(cat <<INNER_EOF
set -uo pipefail
EVID="$REMOTE_EVIDENCE"
mkdir -p "\$EVID"

STAGING="/srv/vm/overdrive-testing"
KERNEL="\$STAGING/kernel"
ROOTFS_MASTER="\$STAGING/rootfs.ext4"
WORKLOAD_ID="e06-vm-job"
BIND="127.0.0.1:7643"
VM_RUN_ROOT="/run/overdrive/vm"
WORKLOADS_SLICE="/sys/fs/cgroup/overdrive.slice/workloads.slice"

CFG_DIR=""
DATA_DIR=""
CREDS_DIR=""
WORK=""
LOOP=""
MNT=""
SERVE_PID=""
SERVE_STATUS="not-started"
DEPLOY_RC="n/a"
FINAL_STATE="unobserved"

# ---- black-box probes (only /proc, cgroupfs, /run, ip, bpftool) -------------
probe_capability() {
  echo "arch:            \$(uname -m)"
  echo "kernel:          \$(uname -sr)"
  echo "cgroup fstype:   \$(stat -fc %T /sys/fs/cgroup 2>&1)"
  if [ -c /dev/kvm ]; then echo "kvm:             /dev/kvm present"; else echo "kvm:             ABSENT"; fi
  if command -v cloud-hypervisor >/dev/null 2>&1; then
    echo "cloud-hypervisor: \$(command -v cloud-hypervisor) -- \$(cloud-hypervisor --version 2>&1 | head -1)"
  else
    echo "cloud-hypervisor: ABSENT"
  fi
  if [ -f "\$KERNEL" ]; then echo "staged kernel:   \$KERNEL (\$(stat -c %s "\$KERNEL") bytes)"; else echo "staged kernel:   ABSENT"; fi
  if [ -f "\$ROOTFS_MASTER" ]; then echo "staged rootfs:   \$ROOTFS_MASTER (\$(stat -c %s "\$ROOTFS_MASTER") bytes)"; else echo "staged rootfs:   ABSENT"; fi
}
# PIDs of live cloud-hypervisor processes, one per line, sorted.
probe_ch_pids() {
  for p in /proc/[0-9]*; do
    [ -r "\$p/cmdline" ] || continue
    argv0="\$(tr '\\0' '\\n' < "\$p/cmdline" 2>/dev/null | head -1)"
    case "\$(basename "\$argv0" 2>/dev/null)" in
      cloud-hypervisor) basename "\$p" ;;
    esac
  done | sort -n
}
probe_ch_detail() {
  for pid in \$(probe_ch_pids); do
    echo "pid \$pid: \$(tr '\\0' ' ' < "/proc/\$pid/cmdline" 2>/dev/null)"
  done
  [ -z "\$(probe_ch_pids)" ] && echo "(no cloud-hypervisor process)"
  return 0
}
probe_scopes() { ls "\$WORKLOADS_SLICE" 2>/dev/null | grep '^alloc-' | sort || true; }
probe_run_dirs() { ls "\$VM_RUN_ROOT" 2>/dev/null | sort || true; }
probe_xdp_ifaces() {
  for i in \$(ip -br link show 2>/dev/null | awk '{print \$1}'); do
    if ip link show "\$i" 2>/dev/null | grep -qE 'xdp(generic|drv)?'; then echo "\$i"; fi
  done | sort
}
probe_links() { ip -br link show 2>/dev/null | awk '{print \$1}' | sort; }

# The State cell of the FIRST attempt row in a \`workload describe\` render, or
# "none" when the workload has no allocation rows at all.
#
# Parsed structurally rather than grepped, because a substring match on the
# state name is a FALSE-POSITIVE TRAP this runner walked into twice, one layer
# apart:
#   1. \`grep -w Running\` matched the EMPTY-STATE diagnostic ("no allocation has
#      converged to a Running instance yet") on a render showing ZERO rows;
#   2. "a leading integer then a word" then matched that same diagnostic's own
#      opening, "0 allocations for workload …".
# So the parse is anchored on the TABLE HEADER: \`workload_describe\` emits
# \`Attempt  State  Exit  Started  Duration\` and then one row per attempt,
# \`{:<8} {:<12} …\` — the attempt INDEX followed by the state label
# (render.rs, format_job_alloc_status_attempts_table). Nothing before that
# header is a row, by construction.
first_attempt_state() {
  awk 'index(\$0, "Attempt") == 1 && \$2 == "State" { in_table = 1; next }
       in_table && \$1 ~ /^[0-9]+\$/ { print \$2; found = 1; exit }
       END { if (!found) print "none" }' "\$1"
}

# ---- BEFORE snapshot: everything teardown must NOT touch --------------------
probe_capability                 > "\$EVID/probe_before_capability.txt" 2>&1
probe_ch_detail                  > "\$EVID/probe_before_hypervisors.txt" 2>&1
{ echo "# alloc cgroup scopes BEFORE"; probe_scopes;   } > "\$EVID/probe_before_scopes.txt" 2>&1
{ echo "# VM run dirs BEFORE";         probe_run_dirs; } > "\$EVID/probe_before_run_dirs.txt" 2>&1
{ echo "# ifaces carrying XDP BEFORE"; probe_xdp_ifaces; } > "\$EVID/probe_before_xdp.txt" 2>&1
BEFORE_CH="\$(probe_ch_pids)"
BEFORE_SCOPES="\$(probe_scopes)"
BEFORE_RUN_DIRS="\$(probe_run_dirs)"
BEFORE_XDP="\$(probe_xdp_ifaces)"
BEFORE_LINKS="\$(probe_links)"

# Lines in \$2 that are absent from \$1 — the delta teardown is allowed to reap.
new_only() { comm -13 <(printf '%s\\n' "\$1" | sed '/^\$/d') <(printf '%s\\n' "\$2" | sed '/^\$/d'); }

# ---- teardown: delta-scoped, fires on EVERY exit path ----------------------
sweep() {
  [ -n "\$SERVE_PID" ] && kill "\$SERVE_PID" 2>/dev/null
  # Reap only hypervisors this run started. A blanket kill would take out a
  # concurrent kvm-tests guest on this shared box.
  for pid in \$(new_only "\$BEFORE_CH" "\$(probe_ch_pids)"); do
    echo "sweep: killing hypervisor pid \$pid started by this run"
    kill -KILL "\$pid" 2>/dev/null
  done
  # Same rule for allocation cgroup scopes: cgroup.kill + rmdir the NEW ones.
  for d in \$(new_only "\$BEFORE_SCOPES" "\$(probe_scopes)"); do
    echo "sweep: reaping cgroup scope \$d created by this run"
    echo 1 > "\$WORKLOADS_SLICE/\$d/cgroup.kill" 2>/dev/null
    rmdir "\$WORKLOADS_SLICE/\$d" 2>/dev/null
  done
  # And for VM run directories.
  for d in \$(new_only "\$BEFORE_RUN_DIRS" "\$(probe_run_dirs)"); do
    echo "sweep: removing VM run dir \$VM_RUN_ROOT/\$d created by this run"
    rm -rf "\$VM_RUN_ROOT/\$d" 2>/dev/null
  done
  # XDP: detach only from ifaces that GAINED an attachment during this run
  # (production serve attaches to its own ovd-veth pair, never eth0).
  for i in \$(new_only "\$BEFORE_XDP" "\$(probe_xdp_ifaces)"); do
    echo "sweep: detaching XDP from \$i attached by this run"
    ip link set dev "\$i" xdpgeneric off 2>/dev/null
    ip link set dev "\$i" xdpdrv off 2>/dev/null
    ip link set dev "\$i" xdp off 2>/dev/null
  done
  # Links this run created (serve's ovd-veth pair) — never a pre-existing one.
  for i in \$(new_only "\$BEFORE_LINKS" "\$(probe_links)"); do
    case "\$i" in
      ovd-*) echo "sweep: deleting link \$i created by this run"; ip link del "\$i" 2>/dev/null ;;
      *)     echo "sweep: LEAVING unexpected new link \$i (not ours to delete)" ;;
    esac
  done
  # Private rootfs clone: unmount, detach the loop device, drop the work dir.
  [ -n "\$MNT" ] && mountpoint -q "\$MNT" && umount "\$MNT" 2>/dev/null
  [ -n "\$LOOP" ] && losetup -d "\$LOOP" 2>/dev/null
  [ -n "\$WORK" ] && rm -rf "\$WORK" 2>/dev/null
  [ -n "\$CFG_DIR" ] && rm -rf "\$CFG_DIR" 2>/dev/null
  [ -n "\$DATA_DIR" ] && rm -rf "\$DATA_DIR" 2>/dev/null
  # The KEK credential, and the copy the provider parked in the kernel session
  # keyring on its miss path. A surviving keyring entry would let a LATER run
  # boot without any credential at all and look like a pass — the exact
  # "kernel keyring leak masks cold-boot bugs" hazard.
  [ -n "\$CREDS_DIR" ] && rm -rf "\$CREDS_DIR" 2>/dev/null
  if command -v keyctl >/dev/null 2>&1; then
    echo "sweep: purging the session-keyring KEK this run parked"
    keyctl purge user "overdrive:ca:kek:overdrive-ca-root" 2>/dev/null
  else
    echo "sweep: keyctl absent; the session keyring dies with this ssh session"
  fi
  return 0
}
on_exit() {
  sweep > "\$EVID/teardown_sweep.txt" 2>&1
  probe_ch_detail                  > "\$EVID/probe_post_teardown_hypervisors.txt" 2>&1
  { echo "# alloc cgroup scopes POST-TEARDOWN"; probe_scopes;   } > "\$EVID/probe_post_teardown_scopes.txt" 2>&1
  { echo "# VM run dirs POST-TEARDOWN";         probe_run_dirs; } > "\$EVID/probe_post_teardown_run_dirs.txt" 2>&1
  { echo "# ifaces carrying XDP POST-TEARDOWN"; probe_xdp_ifaces; } > "\$EVID/probe_post_teardown_xdp.txt" 2>&1
  # Machine-readable leak verdict: post-teardown state must equal BEFORE state.
  {
    echo "# Every line here must read 0. A non-zero count is residue this run left"
    echo "# on a SHARED box, and the host-side gate fails the run on it."
    echo "leaked_hypervisors=\$(new_only "\$BEFORE_CH"       "\$(probe_ch_pids)"   | grep -c . )"
    echo "leaked_scopes=\$(     new_only "\$BEFORE_SCOPES"   "\$(probe_scopes)"    | grep -c . )"
    echo "leaked_run_dirs=\$(   new_only "\$BEFORE_RUN_DIRS" "\$(probe_run_dirs)"  | grep -c . )"
    echo "leaked_xdp=\$(        new_only "\$BEFORE_XDP"      "\$(probe_xdp_ifaces)"| grep -c . )"
  } > "\$EVID/leak_verdict.txt" 2>&1
  return 0
}
trap on_exit EXIT

# ---- host capability gate: a miss here is 'cannot run', not 'claim refuted' --
if [ ! -c /dev/kvm ] || ! command -v cloud-hypervisor >/dev/null 2>&1 \\
   || [ ! -f "\$KERNEL" ] || [ ! -f "\$ROOTFS_MASTER" ]; then
  echo "CAPABILITY_MISSING — see probe_before_capability.txt"
  echo "INNER_DONE serve_status=capability-missing deploy_rc=n/a final_state=unobserved"
  exit 0
fi

# ---- private rootfs clone carrying a long-lived guest command ---------------
# The shared fixture rootfs carries ONLY overdrive-init, two device nodes and
# empty mountpoints — no /sbin/true, no shell. A guest command that cannot exec
# makes Running a window the poller can miss, so this run stages its OWN copy
# with the host's STATICALLY-LINKED /usr/bin/busybox at /sbin/busybox and asks
# for \`busybox sleep 3600\`. The shared master is opened read-only and never
# mutated (other Tier-3 scenarios reuse it concurrently).
WORK="\$(mktemp -d "\$STAGING/e06-XXXXXX")"
MNT="\$WORK/mnt"
mkdir -p "\$MNT"
if ! cp --reflink=auto "\$ROOTFS_MASTER" "\$WORK/rootfs.ext4" 2>> "\$EVID/rootfs_staging.log"; then
  echo "ROOTFS_CLONE_FAILED"; echo "INNER_DONE serve_status=rootfs-clone-failed deploy_rc=n/a final_state=unobserved"; exit 0
fi
LOOP="\$(losetup --find --show "\$WORK/rootfs.ext4" 2>> "\$EVID/rootfs_staging.log")"
if [ -z "\$LOOP" ]; then
  echo "LOSETUP_FAILED"; echo "INNER_DONE serve_status=losetup-failed deploy_rc=n/a final_state=unobserved"; exit 0
fi
if ! mount "\$LOOP" "\$MNT" 2>> "\$EVID/rootfs_staging.log"; then
  echo "MOUNT_FAILED"; echo "INNER_DONE serve_status=mount-failed deploy_rc=n/a final_state=unobserved"; exit 0
fi
cp /usr/bin/busybox "\$MNT/sbin/busybox" 2>> "\$EVID/rootfs_staging.log"
chmod 0755 "\$MNT/sbin/busybox" 2>> "\$EVID/rootfs_staging.log"
{
  echo "# staged guest command into the PRIVATE rootfs clone (master untouched)"
  echo "master: \$ROOTFS_MASTER"
  echo "clone:  \$WORK/rootfs.ext4"
  ls -l "\$MNT/sbin/" 2>&1
  file "\$MNT/sbin/busybox" 2>&1 || true
} >> "\$EVID/rootfs_staging.log" 2>&1
umount "\$MNT" 2>> "\$EVID/rootfs_staging.log"
losetup -d "\$LOOP" 2>> "\$EVID/rootfs_staging.log"
LOOP=""
MNT=""

# ---- the [job] + [vm] spec, the S-VM-39 shape ------------------------------
SPEC="\$WORK/render.toml"
cat > "\$SPEC" <<SPEC_EOF
[job]
id = "\$WORKLOAD_ID"

[vm]
command = "/sbin/busybox"
args = ["sleep", "3600"]
kernel = "\$KERNEL"
rootfs = "\$WORK/rootfs.ext4"

[resources]
cpu_milli = 500
memory_bytes = 134217728
SPEC_EOF
cp "\$SPEC" "\$EVID/spec_render.toml"

# ---- build the PRODUCTION binary (default features) ------------------------
echo "# building: cargo build -p overdrive-cli --bin overdrive (DEFAULT features)"
if ! cargo build -p overdrive-cli --bin overdrive > "\$EVID/build.log" 2>&1; then
  echo "BUILD_FAILED"; tail -40 "\$EVID/build.log"
  echo "INNER_DONE serve_status=build-failed deploy_rc=n/a final_state=unobserved"
  exit 0
fi
BIN="\${CARGO_TARGET_DIR:-\$PWD/target}/debug/overdrive"
if [ ! -x "\$BIN" ]; then
  echo "BIN_MISSING: \$BIN"; echo "INNER_DONE serve_status=bin-missing deploy_rc=n/a final_state=unobserved"; exit 0
fi
{
  echo "# the binary under test"
  echo "path:  \$BIN"
  echo "build: cargo build -p overdrive-cli --bin overdrive   (no --features)"
  ls -l "\$BIN"
} > "\$EVID/binary_under_test.txt" 2>&1

# ---- phase A: serve with NO KEK delivered ----------------------------------
# Captured deliberately, and it is NOT one of E06's sub-claims. Production
# \`serve\` composes SystemdCredsKeyring (crates/overdrive-cli/src/commands/
# serve.rs:118-121) and refuses to start when nothing delivers the
# workload-identity KEK — fail-closed per ADR-0063's Earned-Trust posture,
# never a throwaway key. That refusal is why phase B has to deliver one, so
# the evidence records it rather than leaving the reader to wonder why a
# credential appears out of nowhere.
CFG_DIR="\$(mktemp -d /tmp/od-e06-cfg.XXXXXX)"
DATA_DIR="\$(mktemp -d /tmp/od-e06-data.XXXXXX)"
echo "# phase A: serve with no KEK delivered (expect a fail-closed refusal)"
OVERDRIVE_CONFIG_DIR="\$CFG_DIR" "\$BIN" serve --bind "\$BIND" --data-dir "\$DATA_DIR" \\
  > "\$EVID/serve_no_kek.log" 2>&1 &
NO_KEK_PID=\$!
for _ in \$(seq 1 20); do
  kill -0 "\$NO_KEK_PID" 2>/dev/null || break
  [ -f "\$CFG_DIR/.overdrive/config" ] && break
  sleep 0.5
done
if kill -0 "\$NO_KEK_PID" 2>/dev/null; then
  echo "# phase A: serve is STILL UP without a delivered KEK — recorded, unexpected"
  kill "\$NO_KEK_PID" 2>/dev/null
  wait "\$NO_KEK_PID" 2>/dev/null
else
  echo "# phase A: serve refused to start, as designed (see serve_no_kek.log)"
fi
rm -rf "\$CFG_DIR" "\$DATA_DIR"

# ---- phase B: deliver the KEK the way systemd does, then serve for real -----
# This is the PRODUCTION delivery contract, not a test seam. The shipped
# provider is SystemdCredsKeyring::new() (no \`with_credentials_dir\` pin — that
# override is test-only and is NOT used here); on a keyring miss it reads
# \$CREDENTIALS_DIRECTORY/<kek_id>, which is precisely the file systemd
# materialises for a unit carrying
# \`LoadCredentialEncrypted=overdrive-ca-root:<path>\`
# (crates/overdrive-host/src/ca/keyring.rs:22-30, :260-285). Setting that env
# var is what an operator's unit file does; the gated dev fallback
# (OVERDRIVE_CA_KEK + OVERDRIVE_CA_KEK_DEV_OPT_IN) is deliberately NOT used,
# since it would put the capture in an explicitly non-production posture.
# 32 raw bytes are consumed verbatim as the 256-bit KEK.
CFG_DIR="\$(mktemp -d /tmp/od-e06-cfg.XXXXXX)"
DATA_DIR="\$(mktemp -d /tmp/od-e06-data.XXXXXX)"
CREDS_DIR="\$(mktemp -d /tmp/od-e06-creds.XXXXXX)"
chmod 0700 "\$CREDS_DIR"
head -c 32 /dev/urandom > "\$CREDS_DIR/overdrive-ca-root"
chmod 0400 "\$CREDS_DIR/overdrive-ca-root"
{
  echo "# workload-identity KEK delivery for this run"
  echo "provider:  SystemdCredsKeyring::new()  (production; no with_credentials_dir pin)"
  echo "contract:  \\\$CREDENTIALS_DIRECTORY/<kek_id>, the file systemd materialises for"
  echo "           LoadCredentialEncrypted=overdrive-ca-root:<path>"
  echo "kek_id:    overdrive-ca-root"
  echo "material:  32 raw bytes from /dev/urandom, fresh per run, 0400, removed at teardown"
  echo "NOT used:  the gated dev fallback OVERDRIVE_CA_KEK / OVERDRIVE_CA_KEK_DEV_OPT_IN"
  echo "           (an explicitly non-production posture, logged as ca.kek.dev_fallback_used)"
  echo "delivered: mode \$(stat -c %a "\$CREDS_DIR/overdrive-ca-root"), \$(stat -c %s "\$CREDS_DIR/overdrive-ca-root") bytes"
} > "\$EVID/kek_provisioning.txt" 2>&1

echo "# phase B: starting ephemeral production serve: bind=\$BIND"
OVERDRIVE_CONFIG_DIR="\$CFG_DIR" CREDENTIALS_DIRECTORY="\$CREDS_DIR" \\
  "\$BIN" serve --bind "\$BIND" --data-dir "\$DATA_DIR" \\
  > "\$EVID/serve.log" 2>&1 &
SERVE_PID=\$!

# serve writes the operator trust triple AFTER it binds the TLS listener.
CFG_FILE="\$CFG_DIR/.overdrive/config"
ready=0
for _ in \$(seq 1 60); do
  if [ -f "\$CFG_FILE" ]; then ready=1; break; fi
  if ! kill -0 "\$SERVE_PID" 2>/dev/null; then break; fi
  sleep 0.5
done
if [ "\$ready" -ne 1 ]; then
  SERVE_STATUS="not-ready"
  echo "SERVE_NOT_READY: trust triple never appeared at \$CFG_FILE"
  echo "--- serve.log tail ---"; tail -30 "\$EVID/serve.log"
  echo "INNER_DONE serve_status=not-ready deploy_rc=n/a final_state=unobserved"
  exit 0
fi
SERVE_STATUS="ready"
echo "# serve ready (pid \$SERVE_PID)"

# ---- sub-claim 1: deploy the [job] + [vm] spec -----------------------------
{
  echo "# command: overdrive deploy \$SPEC"
  echo "# seed:    $SEED"
  echo "# started: \$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} > "\$EVID/deploy_vm_job.meta"
DEPLOY_RC=0
OVERDRIVE_CONFIG_DIR="\$CFG_DIR" "\$BIN" deploy "\$SPEC" \\
  > "\$EVID/deploy_vm_job.out" 2>&1 || DEPLOY_RC=\$?
echo "# exit:    \$DEPLOY_RC" >> "\$EVID/deploy_vm_job.meta"
echo "# deploy exit: \$DEPLOY_RC"

# ---- sub-claim 2: poll workload describe for Running -----------------------
# 90s ceiling, comfortably above the 30s guest-boot deadline, so a timeout
# means the allocation did not reach Running -- never that the poll was
# impatient. Every observation is kept, so the states the allocation passed
# THROUGH are evidence too, not just where it landed.
: > "\$EVID/describe_poll_trail.out"
: > "\$EVID/observed_states.txt"
observed_running=0
for _ in \$(seq 1 45); do
  {
    echo "--- \$(date -u +%Y-%m-%dT%H:%M:%SZ) ---"
    OVERDRIVE_CONFIG_DIR="\$CFG_DIR" "\$BIN" workload describe "\$WORKLOAD_ID" 2>&1
  } > "\$EVID/describe_latest.out"
  cat "\$EVID/describe_latest.out" >> "\$EVID/describe_poll_trail.out"
  FINAL_STATE="\$(first_attempt_state "\$EVID/describe_latest.out")"
  echo "\$(date -u +%Y-%m-%dT%H:%M:%SZ) \$FINAL_STATE" >> "\$EVID/observed_states.txt"
  if [ "\$FINAL_STATE" = "Running" ]; then
    observed_running=1
    # sub-claim 3 must be probed WHILE Running -- the resources are the
    # allocation's own and vanish with it.
    probe_ch_detail                          > "\$EVID/probe_running_hypervisors.txt" 2>&1
    { echo "# alloc scopes WHILE Running";  probe_scopes;   } > "\$EVID/probe_running_scopes.txt" 2>&1
    { echo "# VM run dirs WHILE Running";   probe_run_dirs; } > "\$EVID/probe_running_run_dirs.txt" 2>&1
    {
      echo "# Allocation-scoped VM resources that appeared during this run."
      echo "# Non-zero on all three is the black-box form of the in-tree"
      echo "# assertion that the production VmDriver path RAN (rather than the"
      echo "# parser merely accepting the spec)."
      echo "new_hypervisors=\$(new_only "\$BEFORE_CH"       "\$(probe_ch_pids)"  | grep -c . )"
      echo "new_run_dirs=\$(   new_only "\$BEFORE_RUN_DIRS" "\$(probe_run_dirs)" | grep -c . )"
      echo "new_scopes=\$(     new_only "\$BEFORE_SCOPES"   "\$(probe_scopes)"   | grep -c . )"
    } > "\$EVID/resource_delta.txt" 2>&1
    break
  fi
  sleep 2
done
cp "\$EVID/describe_latest.out" "\$EVID/describe_final.out"

if [ "\$observed_running" -ne 1 ]; then
  FINAL_STATE="never-Running(last=\$FINAL_STATE)"
  # Capture the same resource delta anyway: it is the evidence that NOTHING
  # allocation-scoped was created, which is what distinguishes 'the driver ran
  # and failed' from 'no driver ran at all'.
  probe_ch_detail                            > "\$EVID/probe_running_hypervisors.txt" 2>&1
  { echo "# alloc scopes AFTER the poll ceiling"; probe_scopes;   } > "\$EVID/probe_running_scopes.txt" 2>&1
  { echo "# VM run dirs AFTER the poll ceiling";  probe_run_dirs; } > "\$EVID/probe_running_run_dirs.txt" 2>&1
  {
    echo "# Allocation-scoped VM resources that appeared during this run."
    echo "# Captured after the 90s ceiling elapsed without Running."
    echo "new_hypervisors=\$(new_only "\$BEFORE_CH"       "\$(probe_ch_pids)"  | grep -c . )"
    echo "new_run_dirs=\$(   new_only "\$BEFORE_RUN_DIRS" "\$(probe_run_dirs)" | grep -c . )"
    echo "new_scopes=\$(     new_only "\$BEFORE_SCOPES"   "\$(probe_scopes)"   | grep -c . )"
  } > "\$EVID/resource_delta.txt" 2>&1
fi

# ---- hygiene: stop through the production verb before shutting serve down ---
# A guest asked to sleep 3600 never exits on its own, and production spawns the
# VMM with kill_on_drop(false) -- an orphan would contaminate the next
# serialized kvm-tests /proc scan. Same lesson S-VM-05 learned on this box.
OVERDRIVE_CONFIG_DIR="\$CFG_DIR" "\$BIN" job stop "\$WORKLOAD_ID" \\
  > "\$EVID/stop_vm_job.out" 2>&1
echo "# stop exit: \$?" >> "\$EVID/stop_vm_job.out"
for _ in \$(seq 1 30); do
  OVERDRIVE_CONFIG_DIR="\$CFG_DIR" "\$BIN" workload describe "\$WORKLOAD_ID" \\
    > "\$EVID/describe_after_stop.out" 2>&1
  # Same structural predicate as the Running poll — never a substring grep.
  [ "\$(first_attempt_state "\$EVID/describe_after_stop.out")" = "Terminated" ] && break
  sleep 2
done

kill "\$SERVE_PID" 2>/dev/null
wait "\$SERVE_PID" 2>/dev/null

echo "INNER_DONE serve_status=\$SERVE_STATUS deploy_rc=\$DEPLOY_RC final_state=\$FINAL_STATE"
exit 0
INNER_EOF
)

# --- execute on the metal box ------------------------------------------------
echo "  --- cargo xtask metal run -- bash -c <inner> ---"
( cd "$REPO_ROOT" && cargo xtask metal run -- bash -c "$INNER" ) \
  > "$RAW_TMP/metal_run" 2>&1
metal_rc=$?
redact < "$RAW_TMP/metal_run" > "$EVIDENCE_DIR/metal_run.out"
echo "  metal run exit: $metal_rc"
tail -25 "$EVIDENCE_DIR/metal_run.out" || true

# --- pull the remote-written evidence back -----------------------------------
# The metal tree lives at ~/overdrive on the box, NOT at $REPO_ROOT (Lima's
# same-path virtiofs mount has no metal equivalent), so the inner script wrote
# into the remote copy of this evidence dir and it has to come home.
#
# HOST-AUTHORED FILES ARE EXCLUDED. `metal run` rsyncs this tree UP before it
# executes, so the previous run's host-authored evidence is sitting on the box
# while this run executes; an unfiltered pull would copy that STALE copy back
# over the fresh one. That is not hypothetical — it happened, and it made the
# host-side gate read a previous run's `serve_status`.
rsync -az -e "ssh ${SSH_OPTS[*]}" \
  --exclude='metal_run.out' --exclude='metal_preflight.out' \
  --exclude='execution_substrate.txt' --exclude='evidence_pullback.out' \
  --exclude='run.log' --exclude='verification.yaml' \
  --exclude='dirty-status.txt' --exclude='dirty-diff.patch' \
  "$TARGET:overdrive/$REMOTE_EVIDENCE/" "$EVIDENCE_DIR/" \
  > "$RAW_TMP/pullback" 2>&1
pull_rc=$?
redact < "$RAW_TMP/pullback" > "$EVIDENCE_DIR/evidence_pullback.out"
echo "# exit: $pull_rc" >> "$EVIDENCE_DIR/evidence_pullback.out"
if [[ "$pull_rc" -ne 0 ]]; then
  echo "  [blocked] could not retrieve remote evidence (rsync exit $pull_rc)"
  exit 1
fi
echo "  [ok] remote evidence retrieved"

# =============================================================================
# Sub-claim evaluation — every verdict reads a file the metal box wrote.
# =============================================================================
rc=0

# --- no-leak gate (HARD). A shared box; residue breaks other work. -----------
leak_rc=0
if [[ -f "$EVIDENCE_DIR/leak_verdict.txt" ]]; then
  while IFS='=' read -r key val; do
    case "$key" in
      leaked_*)
        if [[ "$val" == "0" ]]; then
          echo "  [PASS] no-leak: $key=0"
        else
          echo "  [FAIL] no-leak: $key=$val — this run left residue on the shared box"
          leak_rc=1
        fi
        ;;
      *) ;;
    esac
  done < "$EVIDENCE_DIR/leak_verdict.txt"
else
  echo "  [FAIL] no-leak: leak_verdict.txt absent — teardown proof missing"
  leak_rc=1
fi

# --- how far did the bring-up get? -------------------------------------------
# Anchored on the INNER_DONE line, never a loose `.*serve_status=` match: the
# inner script's own SOURCE is echoed into this log by the xtask runner, and it
# contains those key names verbatim.
serve_status="$(sed -n 's/^INNER_DONE .*serve_status=\([^ ]*\).*/\1/p' "$EVIDENCE_DIR/metal_run.out" | tail -1)"
final_state="$( sed -n 's/^INNER_DONE .*final_state=\([^ ]*\).*/\1/p'  "$EVIDENCE_DIR/metal_run.out" | tail -1)"

# Sub-claim 0 — the host CAN boot a guest, so a later failure is the platform's,
# never the box's.
if grep -q '/dev/kvm present' "$EVIDENCE_DIR/probe_before_capability.txt" 2>/dev/null \
   && grep -q 'cloud-hypervisor: /' "$EVIDENCE_DIR/probe_before_capability.txt" 2>/dev/null \
   && ! grep -q 'staged kernel:   ABSENT' "$EVIDENCE_DIR/probe_before_capability.txt" 2>/dev/null \
   && ! grep -q 'staged rootfs:   ABSENT' "$EVIDENCE_DIR/probe_before_capability.txt" 2>/dev/null; then
  echo "  [PASS] sub-claim 0: the box is KVM-capable with cloud-hypervisor and staged artifacts"
else
  echo "  [FAIL] sub-claim 0: host capability missing — see probe_before_capability.txt"
  rc=1
fi

if [[ "$serve_status" != "ready" ]]; then
  echo "  [pending] production serve did not bind (serve_status='${serve_status:-unknown}')."
  echo "            Sub-claims 1-3 are UNOBSERVED, not refuted. Inspect"
  echo "            evidence/serve.log for the executed reason."
  exit 1
fi

# Sub-claim 1 — accepted, not rejected.
deploy_rc="$(sed -n 's/^# exit:[[:space:]]*//p' "$EVIDENCE_DIR/deploy_vm_job.meta" 2>/dev/null)"
if [[ "$deploy_rc" == "0" ]]; then
  echo "  [PASS] sub-claim 1a: deploy exited 0"
else
  echo "  [FAIL] sub-claim 1a: deploy exited '${deploy_rc:-<none>}' (expected 0)"
  rc=1
fi
if [[ -f "$EVIDENCE_DIR/deploy_vm_job.out" ]]; then
  evidence_contains deploy_vm_job "Accepted." || rc=1
else
  echo "  [FAIL] sub-claim 1b: no deploy output captured"
  rc=1
fi

# Sub-claim 2 — the allocation reaches Running.
if [[ "$final_state" == "Running" ]]; then
  echo "  [PASS] sub-claim 2: the VM allocation reached Running within the 90s ceiling"
else
  echo "  [FAIL] sub-claim 2: the VM allocation never reached Running (final_state='${final_state:-unknown}')"
  echo "         see describe_final.out / describe_poll_trail.out for the states observed"
  rc=1
fi

# Sub-claim 3 — the production VmDriver path RAN. The black-box inverse of the
# in-tree residue assertion: a hypervisor process, a run directory and a cgroup
# scope that did not exist before this run.
if [[ -f "$EVIDENCE_DIR/resource_delta.txt" ]]; then
  delta_ok=1
  for key in new_hypervisors new_run_dirs new_scopes; do
    val="$(sed -n "s/^${key}=//p" "$EVIDENCE_DIR/resource_delta.txt")"
    if [[ "${val:-0}" -gt 0 ]]; then
      echo "  [PASS] sub-claim 3: $key=$val"
    else
      echo "  [FAIL] sub-claim 3: $key=${val:-<none>} — no allocation-scoped VM resource appeared"
      delta_ok=0
    fi
  done
  [[ "$delta_ok" -eq 1 ]] || rc=1
  # Cross-check, and it earns its keep: an earlier revision of this runner
  # matched `Running` as a substring and hit the empty-state diagnostic ("no
  # allocation has converged to a Running instance yet"), reporting Running
  # against a render showing ZERO allocations. Sub-claim 3 reading all-zero is
  # what exposed it. A state that claims Running while NOTHING
  # allocation-scoped exists is vacuous evidence, so it fails the run outright
  # rather than being quietly reported as 2-of-3.
  if [[ "$final_state" == "Running" && "$delta_ok" -eq 0 ]]; then
    echo "  [FAIL] CONTRADICTION: state reads Running but no allocation-scoped VM"
    echo "         resource exists. One of the two probes is lying; the evidence"
    echo "         cannot support the claim either way."
    rc=1
  fi
else
  echo "  [FAIL] sub-claim 3: resource_delta.txt absent"
  rc=1
fi

[[ "$leak_rc" -eq 0 ]] || rc=1
echo "E06 sub-claim aggregate exit: $rc"
exit "$rc"
