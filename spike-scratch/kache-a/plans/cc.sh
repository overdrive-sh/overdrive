# kache + PATH C/C++ shims (build-script objects through kache), cold publish then fresh restore, state X only.
plan_main() {
  rm -f "$STATE/disks/cache.img"
  boot_vm cache 10.77.0.2 4 8 100 cache
  wait_file "$BUS/cache/ready" 300
  local c="SPIKE_STEPS=ladder SMOKE=0 ARM=kache-cc PFX=cc-1 FRESH=X"
  run_vm cc-a 10.77.0.11 8 24 150 dev $c ROLE=A
  run_vm cc-b 10.77.0.12 8 24 150 dev $c ROLE=B
}
