# S4 paired edit loop after a RESTORE: publisher VM, then one VM with a plain and a kache checkout.
plan_main() {
  rm -f "$STATE/disks/cache.img"
  boot_vm cache 10.77.0.2 4 8 100 cache
  wait_file "$BUS/cache/ready" 300
  run_vm s4-p 10.77.0.11 8 24 150 dev SPIKE_STEPS=s4 MODE=publish
  run_vm s4-b 10.77.0.12 8 24 150 dev SPIKE_STEPS=s4 MODE=paired N=12
}
