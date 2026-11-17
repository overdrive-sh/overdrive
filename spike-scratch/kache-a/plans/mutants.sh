# S8 cargo-mutants. Plan args: (none). plain, then kache cold remote, then kache warm remote.
plan_main() {
  rm -f "$STATE/disks/cache.img"
  boot_vm cache 10.77.0.2 4 8 100 cache
  wait_file "$BUS/cache/ready" 300
  local c="SPIKE_STEPS=mutants"
  run_vm mt-p 10.77.0.11 8 24 100 dev $c ARM=plain PFX=m-plain
  run_vm mt-k 10.77.0.12 8 24 100 dev $c ARM=kache PFX=m-kache
  run_vm mt-w 10.77.0.13 8 24 100 dev $c ARM=kache PFX=m-kache
}
