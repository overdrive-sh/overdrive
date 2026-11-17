# Commit ladder (S0-S3, S4, S5, S7, S10). Plan args: <iteration> <arm,arm,...> [full]
# Arms (each in fresh VMs, one VM at a time):
#   plain, plain-noinc   one VM, the whole ladder
#   coldpi, coldpn       one fresh VM, state X only, plain / plain-noinc (S0 samples)
#   sccache              A publishes (noinc), B empty VM same path
#   kache                A publishes (adaptive default), B empty VM same path (ladder),
#                        D empty VM at a DIFFERENT checkout path, state X only (S5)
#   kachepub             A publishes with KACHE_ADAPTIVE_INCREMENTAL=0, B persistent-target
#                        rebase ladder (adaptive default), C a fresh VM building X+all diffs
#   kachepubc            as kachepub without B
# `full` adds the S4 edit loops and the S7 default-lane tests.
plan_main() {
  local iter="${PA[0]}" arms="${PA[1]}" full=0
  [ "${PA[2]:-}" = full ] && full=1
  local smoke=0; [ "${PA[3]:-}" = smoke ] && smoke=1
  rm -f "$STATE/disks/cache.img"
  boot_vm cache 10.77.0.2 4 8 150 cache
  wait_file "$BUS/cache/ready" 300
  local arm pfx
  local lp_plain=0 lp_noinc=0 lp_sc=0 lp_ka=0 t7=0
  if [ "$full" = 1 ]; then lp_plain=20 lp_noinc=8 lp_sc=8 lp_ka=20 t7=1; fi
  if [ "$smoke" = 1 ]; then lp_plain=2 lp_noinc=2 lp_sc=2 lp_ka=2; fi
  local common="SPIKE_STEPS=ladder SMOKE=$smoke"
  for arm in ${arms//,/ }; do
    pfx="l${iter}-${arm}"
    case "$arm" in
      plain)       run_vm pi-a 10.77.0.11 8 24 150 dev $common ARM=plain PFX="$pfx" ROLE=A LOOP=$lp_plain ;;
      plain-noinc) run_vm pn-a 10.77.0.11 8 24 150 dev $common ARM=plain-noinc PFX="$pfx" ROLE=A LOOP=$lp_noinc TESTS=$t7 ;;
      coldpi)      run_vm cp-a 10.77.0.11 8 24 150 dev $common ARM=plain PFX="$pfx" ROLE=A FRESH=X ;;
      coldpn)      run_vm cn-a 10.77.0.11 8 24 150 dev $common ARM=plain-noinc PFX="$pfx" ROLE=A FRESH=X ;;
      sccache)
        run_vm sc-a 10.77.0.11 8 24 150 dev $common ARM=sccache PFX="$pfx" ROLE=A LOOP=$lp_sc
        run_vm sc-b 10.77.0.12 8 24 150 dev $common ARM=sccache PFX="$pfx" ROLE=B ;;
      kache)
        run_vm ka-a 10.77.0.11 8 24 150 dev $common ARM=kache PFX="$pfx" ROLE=A LOOP=$lp_ka
        run_vm ka-b 10.77.0.12 8 24 150 dev $common ARM=kache PFX="$pfx" ROLE=B TESTS=$t7
        run_vm ka-d 10.77.0.13 8 24 150 dev $common ARM=kache PFX="$pfx" ROLE=D WS=/mnt/alt/checkout-d FRESH=X ;;
      kachepub|kachepubc)
        run_vm kp-a 10.77.0.11 8 24 150 dev $common ARM=kache-noadapt PFX="$pfx" ROLE=A
        [ "$arm" = kachepub ] && run_vm kp-b 10.77.0.12 8 24 150 dev $common ARM=kache PFX="$pfx" ROLE=B
        run_vm kp-c 10.77.0.13 8 24 150 dev $common ARM=kache PFX="$pfx" ROLE=C FRESH=Xm TESTS=$t7 ;;
    esac
  done
}
