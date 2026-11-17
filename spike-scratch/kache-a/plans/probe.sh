# Plumbing probe: Garage VM + one dev VM.
plan_main() {
  boot_vm cache 10.77.0.2 2 4 60 cache
  wait_file "$BUS/cache/ready" 300
  boot_vm dev-a 10.77.0.11 8 16 120 dev SPIKE_STEPS=probe
  wait_vms dev-a
}
