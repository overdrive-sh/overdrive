# S6 concurrency + S11 toolchain facts. Plan args: (none)
#   wave 1: 3 VMs (5 vCPU) build the full tree at once from a COLD remote
#   wave 2: 3 fresh VMs build it at once from the now WARM remote
#   wave 3: 8 VMs (2 vCPU) build overdrive-core at once from a cold remote
wave() { # NAME PFX SPEC VCPUS MEM_GB IPBASE COUNT BUDGET_GB
  local name="$1" pfx="$2" spec="$3" vcpus="$4" mem="$5" base="$6" n="$7" budget="$8" i peers="" names=()
  for i in $(seq "$n"); do names+=("$name$i"); peers+="${peers:+,}$name$i"; done
  ensure_room_n "$n" "$budget"
  for v in "${names[@]}"; do rm -f "$STATE/disks/$v.img"; done
  for i in $(seq "$n"); do
    boot_vm "$name$i" "10.77.0.$((base + i))" "$vcpus" "$mem" 80 dev SPIKE_STEPS=conc WAVE="$name" PFX="$pfx" SPEC="$spec" PEERS="$peers"
  done
  wait_vms "${names[@]}" || true
  for v in "${names[@]}"; do
    # shellcheck disable=SC2086
    kill ${VM_FS[$v]} 2>/dev/null || true
    rm -f "$STATE/disks/$v.img"
  done
}
plan_main() {
  rm -f "$STATE/disks/cache.img"
  boot_vm cache 10.77.0.2 4 8 200 cache
  wait_file "$BUS/cache/ready" 300
  wave w1 s6-full full 5 12 30 3 22
  wave w2 s6-full full 5 12 40 3 22
  wave w3 s6-core core 2 4 50 8 6
}
