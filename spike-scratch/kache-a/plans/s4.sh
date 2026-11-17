# S4 paired edit loop, plain vs kache, in one VM. Plan args: (none)
plan_main() {
  rm -f "$STATE/disks/cache.img"
  boot_vm cache 10.77.0.2 4 8 100 cache
  wait_file "$BUS/cache/ready" 300
  run_vm s4-a 10.77.0.11 8 24 150 dev SPIKE_STEPS=s4 N=20
}
