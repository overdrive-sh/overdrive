# sourced on metal. Paths for the probe session.
export PATH=/home/ubuntu/.cargo/bin:$PATH
V=/home/ubuntu/overdrive/spike-scratch/pig-vgw
ELF=$V/target/bpf-target/bpfel-unknown-none/release/vgw-bpf
VGW=$V/target/host/release/vgw
DIAL=$V/target/host/release/vgwdial
PIN=/sys/fs/bpf/pigvgw
SERVE_CG=/sys/fs/cgroup/overdrive.slice/control-plane.slice
NEG_CG=/sys/fs/cgroup/pigvgw-neg
OUT=$V/out
stamp() { echo "=== $* [$(date -u +%T.%3N)] uname=$(uname -r)"; }
dumpmap() { bpftool map dump pinned "$PIN/$1" 2>&1; }
