# Calibration plan: probe of the kache/Garage path, then a cold plain build.
plan_main() {
  rm -f "$STATE/disks/cache.img" "$STATE/disks/dev-a.img"
  boot_vm cache 10.77.0.2 2 4 100 cache
  wait_file "$BUS/cache/ready" 300
  boot_vm dev-a 10.77.0.11 8 24 150 dev SPIKE_STEPS=probe,calib
  wait_vms dev-a
}
