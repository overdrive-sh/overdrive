#!/usr/bin/env bash
# Run on metal: cargo xtask metal run -- bash spike-scratch/listen-hooks/run.sh [mode]
# mode: all | v32 | v33 | v35 | v37 | build
set -euo pipefail
export PATH=$HOME/.cargo/bin:$PATH
here="$(cd "$(dirname "$0")" && pwd)"
mode="${1:-all}"
T=/tmp/listen-hooks-target
out="$here/evidence"; mkdir -p "$out"
trap 'chown -R "${SUDO_UID:-0}:${SUDO_GID:-0}" "$here" 2>/dev/null || true' EXIT

build() {
  RUSTFLAGS="-C linker=bpf-linker -C target-cpu=v3" CARGO_TARGET_DIR=$T/bpf-listen \
    cargo +nightly-2026-08-05 build --release --target bpfel-unknown-none -Z build-std=core \
    --features listen --manifest-path "$here/bpf/Cargo.toml"
  RUSTFLAGS="-C linker=bpf-linker -C target-cpu=v3" CARGO_TARGET_DIR=$T/bpf-base \
    cargo +nightly-2026-08-05 build --release --target bpfel-unknown-none -Z build-std=core \
    --manifest-path "$here/bpf/Cargo.toml"
  CARGO_TARGET_DIR=$T/host cargo build --release --manifest-path "$here/probe/Cargo.toml"
}

v37() {
  echo "================ V37 kernel config ================"
  echo "uname -r: $(uname -r)   uname -m: $(uname -m)   systemd-detect-virt: $(systemd-detect-virt || true)"
  cfg=/boot/config-$(uname -r)
  echo "--- host kernel config ($cfg)"
  grep -E '^(# )?CONFIG_(BPF_SYSCALL|CGROUP_BPF|BPF|INET|IPV6|NET_HANDSHAKE|BPF_JIT|CGROUPS|DEBUG_INFO_BTF)[ =]' "$cfg" || true
  echo "--- bpftool feature probe kernel (filtered)"
  bpftool feature probe kernel 2>/dev/null | grep -E 'program_type (sock_ops|cgroup_sock|cgroup_sock_addr)|map_type (ringbuf|percpu_array)|CONFIG_(BPF_SYSCALL|CGROUP_BPF)|bpf_get_current_pid_tgid|bpf_getsockopt|bpf_get_socket_cookie' || true
  echo "--- attach-type availability is proven by the real loads/attaches in the V32/V33/V35 output"
  echo "--- fixture guest kernel (OVERDRIVE_METAL_KERNEL)"
  k="${OVERDRIVE_METAL_KERNEL:-/srv/vm/overdrive-testing/kernel}"
  ls -l "$k" || true
  real="$(readlink -f "$k")"
  echo "resolved: $real"
  file -L "$k" || true
  ls -l /srv/vm/overdrive-testing/ 2>/dev/null || true
  strings -a "$real" 2>/dev/null | grep -m1 'Linux version' || true
  if cmp -s "$real" "/boot/vmlinuz-$(uname -r)"; then
    echo "fixture kernel == /boot/vmlinuz-$(uname -r) (byte-identical)"
  else
    echo "fixture kernel is NOT byte-identical to /boot/vmlinuz-$(uname -r)"
  fi
  python3 - "$real" <<'PY'
import sys, zlib, lzma, bz2
d = open(sys.argv[1], 'rb').read()
KEYS = ("CONFIG_BPF_SYSCALL", "CONFIG_CGROUP_BPF", "CONFIG_INET=", "CONFIG_IPV6=", "CONFIG_BPF=")
def dump(raw, label):
    j = raw.find(b'IKCFG_ST')
    if j < 0:
        return False
    t = zlib.decompressobj(31).decompress(raw[j + 8:]).decode()
    print("embedded IKCFG config extracted from", label)
    for l in t.splitlines():
        if l.startswith(KEYS):
            print(l)
    return True
print("size", len(d))
if dump(d, "raw image"):
    sys.exit(0)
for name, magic, dec in (
    ("gzip", b'\x1f\x8b\x08', lambda b: zlib.decompressobj(31).decompress(b)),
    ("xz", b'\xfd7zXZ\x00', lambda b: lzma.LZMADecompressor().decompress(b)),
    ("bz2", b'BZh', lambda b: bz2.BZ2Decompressor().decompress(b)),
):
    p = 0
    while True:
        p = d.find(magic, p)
        if p < 0 or p > len(d) - 16:
            break
        try:
            if dump(dec(d[p:]), name + " payload at %d" % p):
                sys.exit(0)
        except Exception:
            pass
        p += 1
print("no IKCFG config found in", sys.argv[1])
PY
}

case "$mode" in
  build) build ;;
  v37) v37 ;;
  *)
    build
    L=$T/bpf-listen/bpfel-unknown-none/release/listen-hooks-bpf
    B=$T/bpf-base/bpfel-unknown-none/release/listen-hooks-bpf
    P=$T/host/release/probe
    W=$T/host/release/wl
    ls -l "$L" "$B" "$P" "$W"
    if [ "$mode" = all ]; then v37 | tee "$out/v37.txt"; fi
    "$P" "$mode" "$L" "$B" "$W" 2>&1 | tee "$out/$mode.txt"
    ;;
esac
