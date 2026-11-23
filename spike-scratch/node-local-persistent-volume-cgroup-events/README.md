Throwaway probe (see docs/feature/node-local-persistent-volume/spike/findings-cgroup-events.md).
Run through the ordinary Metal grammar; every test is placed in a `cgroup-default` / `volume-pool` guest:

    cargo xtask metal run -- cargo nextest run --manifest-path spike-scratch/node-local-persistent-volume-cgroup-events/Cargo.toml -j 1 --success-output immediate --failure-output immediate

`evidence/run8-filtered.txt` is the SPIKE-line extract of the recorded run; `out/` is not committed.
