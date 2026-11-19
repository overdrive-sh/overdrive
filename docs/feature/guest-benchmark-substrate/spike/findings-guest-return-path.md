# V-B (O6) — Returning a block of samples from an in-guest payload to the host half, from inside the benchmark form

**Verdict: WORKS, with no production-code change.** A benchmark crate under `spike-scratch/` booted a guest-kit guest from inside the `cargo bench` process that `cargo xtask metal run --benchmark <family> --spike <name>` launches, ran a prepared in-guest binary, and received its u64 nanosecond samples back intact over both the guest console and vsock. Two spike-local adaptations were needed (cwd, and a concurrent reader for vsock); neither touches `crates/`.

Host: native Metal host, `uname -r` = `7.0.0-38-generic`, AMD EPYC 8024P. Commit `5eaa4f7e4ff93a08fb14480bbc1ef979be5e8e1b` (working tree dirty with docs only). Run 2026-11-19.

Probe: `spike-scratch/guest-bench-return/` (`src/lib.rs` host half, `src/guest_sampler.rs` in-guest payload, `benches/benchmarks.rs`). Records: `spike-scratch/guest-bench-return/results/guest-bench-return/` (three runs, kept as evidence; the last, `1728b190aa4eaf615bf534f1`, is the passing one).

## Hypothesis, prediction, falsification

- **Hypothesis:** the in-guest lane's mechanism (the guest console carries the payload's output; vsock reaches the host) works unchanged from a benchmark body inside the benchmark form, for a block of 1,000 u64 samples and well beyond.
- **Prediction:** 1,000 samples return with matching count and checksum over the console and over vsock; staging the payload needs only `GuestFile` in the image spec and the existing `guest-kit-prepare` artefacts.
- **Falsification:** the payload cannot be staged without editing `xtask/src/guest_kit_prepare.rs` or `crates/`; or the block arrives short, corrupted, or not at all at 1,000 samples.

## How the layout and staging work (the `--spike` form)

- The runner executes `cargo bench --manifest-path spike-scratch/<name>/Cargo.toml --bench benchmarks -- run --family <family> --results-root <checkout>/spike-scratch/<name>/results` (`infra/metal/bootstrap.sh:830`). The spike therefore needs `benches/benchmarks.rs` (harness = false) calling `overdrive_bench::run_main(Evidence::Spike)` and a `use <crate> as _;` so the `#[overdrive_bench]` registrations link. Precedent: `spike-scratch/pig-connect-cost`.
- A family is exactly one benchmark; the spike declares two families (`guest-boot-cost`, `guest-bench-return`) and runs each with the same `--spike`.
- **The in-guest payload** is a second `[[bin]]` of the spike crate (`guest-sampler`, dynamically linked). `cargo bench` builds it beside the bench binary (`target/release/guest-sampler`); the host half stages it with `GuestImageSpec.extra = [GuestFile { host, guest: /opt/spike/guest-sampler, with_shared_libraries: true }]` into a `GuestInit::ScriptedPeer` image and runs it with `GuestPeer::exec`. No change to `IN_GUEST_LANES` in `xtask/src/guest_kit_prepare.rs` was needed: that list stages libtest binaries for nextest lanes; `GuestFile` stages any host file.
- Prerequisites used: `cargo xtask metal run -- cargo xtask guest-kit-prepare` (the forwarder module, the static scripted peer, the static `overdrive-init`) and `cargo xtask metal run -- env OVERDRIVE_BPF_NATIVE=1 cargo xtask bpf-build`. `kmod-load` was not needed.

## Spike-local adaptations (no `crates/` change)

1. **cwd.** First run: every replicate failed with `kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed … package ID specification 'overdrive-init' did not match any packages` (run `c0ffa9c50e30be5fb984e45c`, 90 of 90 failed). Cause: `vm_fixture::workspace_root()` runs `cargo locate-project --workspace` from the cwd, and `cargo bench` runs the bench with cwd = the package directory, which is the spike's own workspace. The spike calls `std::env::set_current_dir(<repo root>)` before `GuestKit::provision`. A maintained family inside the workspace (`crates/overdrive-bench-families`) does not hit this because its cwd locates the real workspace. A guest-substrate design that resolves the kit from a `--spike` crate, or that runs a family from outside the workspace, hits it.
2. **vsock needs a concurrent reader.** Second run (`fbd007f8944b07177409d285`): vsock 1k passed; vsock 100k failed 10 of 10 with `exec: no reply to EXEC … within 130s`. Cause (a harness bug, not a transport limit): `GuestPeer::exec` returns only when the guest process exits, the guest blocks in `write` once the vsock buffer fills (800 KB sent against a host that only reads after `exec` returns), so neither side proceeds. Third run: the host half reads the vsock block on a thread started before `exec`; all vsock sizes pass. The console path has no such coupling because the console log is a file the VMM writes regardless of the host's reading.

## Observed (third run, `1728b190aa4eaf615bf534f1`: 70 of 70 replicates ok, 0 failed, 10 per arm)

Framing: console = `OVDSAMPLES-BEGIN <n>`, lines of 16 space-separated decimals, `OVDSAMPLES-END <n> <fnv1a-64 hex>`; vsock = `n x 8` little-endian bytes then the 8-byte FNV-1a (guest connects to CID 2). The host recounts the samples and recomputes the checksum over the little-endian bytes; `integrity_ok` is 1 only if both match.

```
| samples_returned | console-1k | 1000 | ...       | integrity_ok | console-1k | 1 |
| samples_returned | vsock-1k | 1000 |            | integrity_ok | vsock-1k | 1 |
| samples_returned | console-100k | 100000 |       | integrity_ok | console-100k | 1 |
| samples_returned | vsock-100k | 100000 |         | integrity_ok | vsock-100k | 1 |
| samples_returned | console-1m | 1000000 |        | integrity_ok | console-1m | 1 |
| samples_returned | vsock-1m | 1000000 |          | integrity_ok | vsock-1m | 1 |

| bytes_on_wire | console-1k | 3146 |     | exec_to_block_ns | console-1k | 61037275 |
| bytes_on_wire | vsock-1k | 8008 |      | exec_to_block_ns | vsock-1k | 8167649.5 |
| bytes_on_wire | console-100k | 306750 | | exec_to_block_ns | console-100k | 5152294063 |
| bytes_on_wire | vsock-100k | 800008 |   | exec_to_block_ns | vsock-100k | 14916882.5 |
| bytes_on_wire | console-1m | 3066671 |  | exec_to_block_ns | console-1m | 52280674493.5 |
| bytes_on_wire | vsock-1m | 8000008 |    | exec_to_block_ns | vsock-1m | 88118125.5 |
```

(medians over 10 replicates per arm; `exec_to_block_ns` is the `exec` call plus extraction on the host.) The returned samples also round-trip through `Replicate::record_series`: the report shows `guest_sample_ns` with 10,000 values over 10 replicates for each 1k arm (p50 30 ns, p95 40 ns) and 1,000,000 values over 10 replicates for each 100k arm. The 1M arms skip `record_series` (count and checksum are the evidence there).

Commands:

```
cargo xtask metal run --benchmark guest-bench-return --spike guest-bench-return
cargo xtask metal pull spike-scratch/guest-bench-return/results/guest-bench-return/1728b190aa4eaf615bf534f1
```

## Size limits and cost observed

- **Console:** no limit hit up to 1,000,000 samples (3.07 MB, intact). Throughput is about 59 KB/s (3.07 MB in 52.3 s; 307 KB in 5.15 s; 3.1 KB in 61 ms). The cost is linear in bytes and is paid inside the guest while the payload runs, so a 1,000-sample block costs about 50 ms of replicate time and a 100,000-sample block 5 s. The payload's decimals here are short (about 3 bytes per sample); 5 to 7 digit latencies would cost roughly 2 to 3 times more bytes per sample. The console also carries the kernel's own output, so the host extracts by markers and tolerates it.
- **vsock:** 8 MB (1M samples) in 88 ms, 100x to 600x faster than the console at the larger sizes, binary and exact. Needs the in-guest payload to open an `AF_VSOCK` socket to CID 2 and a host `VsockListener` bound on a kernel-assigned port (passed in argv), read concurrently (adaptation 2). `GuestPeer::exec` carries only the exit status, not stdout.
- No size ceiling was reached on either path.

## Production-code changes required

None. The two adaptations are in the spike crate. Design findings that are not production changes but that a design must decide:

1. The kit resolves its artefacts from `vm_fixture::workspace_root()`, which depends on the cwd; a benchmark body that runs outside the workspace must set it.
2. `IN_GUEST_LANES` is nextest-oriented; a benchmark's in-guest payload is staged by `GuestFile` and built by `cargo bench` beside the bench binary, so a maintained family needs its payload as a bin target of `overdrive-bench-families` (or a sibling) rather than an entry in that list. Whether to add a prepare step for it is a design question this spike does not decide.
3. Benchmark families boot guests on `substrate = native`; the registry's `guest` substrate is closed (`registry.rs`), so a guest-substrate declaration would be new surface and is out of this spike.

## Gate recommendation

PROMOTE the return path: console for blocks up to roughly 1,000 to 10,000 samples (cost 50 to 500 ms), vsock for anything larger; both verified intact with a count and checksum.
