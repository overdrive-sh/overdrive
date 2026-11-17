# S7/S9 probes (see scripts/step-wronghit.sh and step-place.sh). Plan args: (none)
plan_main() {
  rm -f "$STATE/disks/cache.img"
  boot_vm cache 10.77.0.2 4 8 100 cache
  wait_file "$BUS/cache/ready" 300
  local c="SPIKE_STEPS=wronghit PFX=wh-1"
  run_vm wh-a 10.77.0.11 8 24 100 dev $c MODE=publish
  run_vm wh-b 10.77.0.12 8 24 100 dev $c MODE=probe
  run_vm wh-c 10.77.0.13 8 24 100 dev $c MODE=fresh
  run_vm pl-p 10.77.0.14 8 24 100 dev SPIKE_STEPS=place ARM=plain PFX=pl-1
  run_vm pl-k 10.77.0.15 8 24 100 dev SPIKE_STEPS=place ARM=kache PFX=pl-1
}
