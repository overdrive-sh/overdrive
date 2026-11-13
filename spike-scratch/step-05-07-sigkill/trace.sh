#!/usr/bin/env bash
# Historical bounded native diagnostic for step 05-07; see README.md.
# This revised overlap variant did not complete an executed capture.
# Run only through canonical `cargo xtask metal run --exclusive -- bash ...`.
#
# Hypothesis: a second run_server boot over a distinct local store sees the
# first process's VM as an unowned host artifact and writes its cgroup.kill.
# B is the existing two-serve kernel-assigned-port test: its NodeAdapters
# retain RealVmHostState while simulating the dataplane, so it reaches boot
# reclamation without refusing at A's host XDP slot. This reproduces a real
# cross-fixture owner path, not the default binary's host-dataplane boot.
# Prediction: after A owns a live alloc-loopback-binds scope, B's traced
# run_server writes 1 to that exact cgroup.kill; A observes SIGKILL and its
# captured allocation has no deadline Fail, while standalone C passes.
# Falsification: the write is absent from the complete A/B owner trees, or a
# distinct same-tree kill/terminate path precedes the victim SIGKILL.
set -uo pipefail
umask 077

if ! command -v strace >/dev/null 2>&1; then
    # The exclusive Metal maintenance run is the host-maintenance boundary. Bound the
    # one diagnostic-package installation and refuse if it cannot complete.
    timeout 120s apt-get update -qq &&
        timeout 120s env DEBIAN_FRONTEND=noninteractive apt-get install -y -qq strace
    if ! command -v strace >/dev/null 2>&1; then
        printf 'TRACE_BLOCKER strace_missing_after_install\n'
        exit 69
    fi
fi

out=".context/step-05-07-native/post-rebase/sigkill-trace-${OVERDRIVE_METAL_RUN_ID:-unknown}"
mkdir -p "$out"
printf 'utc=%s\ncommit=%s\nrun_id=%s\n' \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    "$(git rev-parse HEAD)" \
    "${OVERDRIVE_METAL_RUN_ID:-unknown}" > "$out/meta.txt"
sha256sum \
    crates/overdrive-control-plane/tests/integration/guest_listener_real_boot.rs \
    crates/overdrive-control-plane/tests/integration/serve_kernel_assigned_vsock_ports.rs \
    crates/overdrive-control-plane/tests/common/node_adapters.rs \
    crates/overdrive-control-plane/src/vm_reclamation_boot.rs \
    crates/overdrive-host/src/vm_host_state.rs \
    infra/metal/test-placement.json >> "$out/meta.txt"

test_filter='test(integration::guest_listener_real_boot::a_loopback_only_bind_never_satisfies_a_declared_listener)'
nextest=(cargo nextest run -p overdrive-control-plane --features 'integration-tests,kvm-tests' -E "$test_filter")
secondary=(cargo nextest run -p overdrive-control-plane --features 'integration-tests,kvm-tests' -E 'test(serve_kernel_assigned_vsock_ports)')
trace=(strace -ff -ttt -yy -s 256 -e 'trace=%process,openat,openat2,write,writev,kill,tgkill,tkill' -e 'raw=write,writev')

printf 'TRACE_PHASE controlled_overlap_a_start\n'
"${trace[@]}" -o "$out/a.syscalls" "${nextest[@]}" > "$out/a.nextest.log" 2>&1 &
a_pid=$!

# Start B only after A has an actual live Cloud Hypervisor in a cgroup.
# An exclusive maintenance run prevents unrelated checkouts from adding
# a fresh VM during this probe. Never signal or clean a foreign process.
scope_root=/sys/fs/cgroup/overdrive.slice/workloads.slice
victim_scope=''
victim_pid=''
for _ in $(seq 1 180); do
    while IFS= read -r candidate; do
        # cgroupfs virtual files report st_size=0 even when readable content
        # exists, so `test -s` would discard a live scope.
        candidate_pid=$(head -n 1 "$candidate/cgroup.procs" 2>/dev/null)
        [ -n "$candidate_pid" ] || continue
        case "$(ps -p "$candidate_pid" -o args= 2>/dev/null)" in
            *cloud-hypervisor*) victim_scope=$candidate; victim_pid=$candidate_pid; break ;;
        esac
    done < <(find "$scope_root" -maxdepth 1 -type d -name 'alloc-loopback-binds-*.scope' -print 2>/dev/null)
    [ -z "$victim_scope" ] || break
    kill -0 "$a_pid" 2>/dev/null || break
    sleep 0.25
done
printf 'victim_scope=%s\nvictim_pid=%s\n' "$victim_scope" "$victim_pid" >> "$out/meta.txt"

b_status=not_started
if [ -n "$victim_scope" ]; then
    printf 'TRACE_PHASE controlled_overlap_b_start victim_pid=%s scope=%s\n' "$victim_pid" "$victim_scope"
    "${trace[@]}" -o "$out/b.syscalls" "${secondary[@]}" > "$out/b.nextest.log" 2>&1
    b_status=$?
fi
wait "$a_pid"
a_status=$?
printf 'a_exit=%s\nb_exit=%s\n' "$a_status" "$b_status" >> "$out/meta.txt"

# One same-source, same-host control with no second server. This matters if
# tracing alters scheduling or the test fails independently of overlap.
printf 'TRACE_PHASE standalone_c_start\n'
"${trace[@]}" -o "$out/c.syscalls" "${nextest[@]}" > "$out/c.nextest.log" 2>&1
c_status=$?
printf 'c_exit=%s\n' "$c_status" >> "$out/meta.txt"

if [ "$c_status" -ne 0 ]; then
    printf 'TRACE_PHASE standalone_untraced_d_start\n'
    "${nextest[@]}" > "$out/d.nextest.log" 2>&1
    d_status=$?
    printf 'd_exit=%s\n' "$d_status" >> "$out/meta.txt"
fi

grep -HEn 'cgroup.kill|SIGKILL|kill\(|wait4\(|waitid\(' "$out"/*.syscalls.* > "$out/kill-path-index.txt" 2>/dev/null || true
printf 'TRACE_COMPLETE out=%s a=%s b=%s c=%s victim_pid=%s scope=%s\n' \
    "$out" "$a_status" "$b_status" "$c_status" "$victim_pid" "$victim_scope"
