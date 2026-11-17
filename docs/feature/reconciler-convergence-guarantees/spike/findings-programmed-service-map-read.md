# V-6 findings: reading the programmed service map back from the kernel

**Verdict: WORKS**, with weight 1 on every backend, which is what production writes today (`crates/overdrive-reconcilers/src/service_lifecycle.rs:1122`). Two conditions shape the D-20 contract; they are listed under "Design implications".

- Kernel: `uname -r` = `7.0.0-38-generic`, on the test machine, run as root.
- Probe: `spike-scratch/programmed-service-map-read/`. It drives the real `EbpfDataplane::new_with_pin_dir` and `update_service` against a private pin directory and veth pair, then reads the maps back with raw `bpf()` calls.
- Run: `cargo xtask metal run -- bash spike-scratch/programmed-service-map-read/run.sh <mode> [prod]`. Raw logs are in `spike-scratch/programmed-service-map-read/out/`.
- Predictions: `predictions-programmed-service-map-read.md`, written before the first run.

## How the read works

1. Look up the frontend's `ServiceKey` in the outer `SERVICE_MAP` (`HASH_OF_MAPS`). From userspace the lookup returns the inner map's **id**, not an fd. `map_lookup_elem` dispatches a fd-hash map to `bpf_fd_htab_map_lookup_elem` (`kernel/bpf/syscall.c:329-330 @ v7.0`). That reaches `bpf_map_fd_sys_lookup_elem`, which returns `((struct bpf_map *)ptr)->id` (`kernel/bpf/map_in_map.c:131-134 @ v7.0`).
2. Open that id with `BPF_MAP_GET_FD_BY_ID` (`bpf_map_get_fd_by_id`, `kernel/bpf/syscall.c:4859 @ v7.0`). The inner map is an `ARRAY` of 16,381 u32 slots, the Maglev table.
3. Read the inner array with one `BPF_MAP_LOOKUP_BATCH` (`generic_map_lookup_batch`, `kernel/bpf/syscall.c:2049 @ v7.0`), collect the distinct `BackendId`s, and look each one up in `BACKEND_MAP` to get `(addr, weight, healthy)`.

## Predicted vs actual

| Hypothesis | Predicted | Actual |
|---|---|---|
| H1 exactness | exact set for 1, 16 and 256 backends, including weight 0 and 65535 | **Exact with weight 1** for 1, 16 and 256 backends, every read strategy. **Not exact with skewed weights**: at n=16 with weights drawn from {0, 1, 65535, 0..999}, only 6 of 16 backends were found. A backend with a tiny weight next to a 65535-weight backend gets no Maglev slot, so it is in `BACKEND_MAP` but nothing links it to the frontend. The prediction was wrong. |
| H2 consistency | unguarded reads tear; reads under a lock spanning `update_service` do not | Confirmed. Unguarded: 2,734 reads against 514 writes; 1,735 torn (neither set A nor B) and 994 with a `BACKEND_MAP` ENOENT. Guarded: 0 torn, 0 errors. |
| H3 restart | a fresh `EbpfDataplane` over the same pin dir reads empty; reboot the same | Confirmed. The fresh instance creates new `SERVICE_MAP` and `BACKEND_MAP` ids and reads 0 backends. Re-programming reads exact again. A brand-new bpffs directory (the reboot analogue) reads empty. |
| H4 cost | one batch read 3–10 ms, flat in backend and frontend count | Confirmed: batch read p50 4.9–5.5 ms; single lookups 13–14 ms. The inner-array scan is about 4.9 ms; `BACKEND_MAP` lookups add 13 µs (1 backend) to 396 µs (256). Flat in frontend count: a sweep of 1,000 frontends × 16 backends takes 5.1 s batched and 13.4 s with single lookups. |

Excerpts (from the `prod` runs on 2026-11-17):

```
n=256 Batch(16381): read 256 backends (256 distinct ids), set (addr,weight,healthy) equal=true
PASS old inner fd still readable after swap (kernel refcount)
PASS withdraw: empty, slot gone
PASS tcp/53 and udp/53 on one VIP are independent keys
OBSERVATION shared-endpoint read(:80) reflects the LATER write (weight 99, unhealthy): true
ALL CORRECTNESS CHECKS PASSED
instance 2 over the SAME pin dir: ... read -> 0 backends, outer slot present=false
PASS fresh dataplane over same pin dir reads empty (serve restart)
guard=NONE ... TORN(neither A nor B):1735 ... backend-missing(ENOENT on BACKEND_MAP):994
guard=spike-level RwLock around the whole update_service ... TORN(neither A nor B):0
frontends=1000 backends= 16 batch(16381) per-frontend ... p50=5090us ... | full sweep of all 1000 = 5119.3 ms
```

Without `prod` (skewed weights):

```
n=16 Single: read 6 backends (6 distinct ids), set (addr,weight,healthy) equal=false
```

## Edge cases

- **Never-programmed frontend:** no outer slot, so the read is empty.
- **Withdrawn frontend** (`update_service(f, [])`): the slot is gone, so the read is empty.
- **Inner map after a swap:** a held inner fd stays readable after the swap (kernel refcount). The old id is freed once the last fd drops, and `GET_FD_BY_ID` then returns ENOENT. A reader that resolves the id before a swap and opens it after can see ENOENT.
- **Protocols:** tcp/53 and udp/53 on one VIP are independent keys.
- **Shared endpoint:** `BACKEND_MAP` is keyed by endpoint. The probe programmed one endpoint under two frontends by hand and the read showed the later write. Production cannot produce this: each allocation has its own workload address and belongs to one Service, so no endpoint is shared between services.
- **IPv6:** a backend is refused by `update_service`, and the read is unchanged.

## Design implications for D-20

1. **Exactness depends on every backend owning a Maglev slot.** The inner table encodes slot ownership, not membership. A backend whose weight earns zero slots is invisible to the read.
   - This holds today, because production writes weight 1 for every backend (`service_lifecycle.rs:1122`) and a frontend has at most 16,381 backends.
   - The `ProgrammedServiceView` rustdoc must state this precondition rather than promise the exact projection of `B` unconditionally.
   - If weights ever become non-uniform, the read needs a membership source, for example a per-frontend member map written beside the inner table.
2. **The read must hold the dataplane's apply-commit guard for read**, as D-20 already pins. Without it, 63% of reads under constant writes were torn.
3. **Cost is about 5 ms per frontend**, dominated by the fixed 16,381-slot scan and independent of backend count. At 1,000 services, one hydrator pass is about 5.1 s of read time per 30 s period, about 17% of one core. This adds to the V-3 resync cost. A cheaper read would need membership stored separately (see 1).

## Gate recommendation

PROMOTE the full-set read for D-20, with the precondition written into the contract, and with the per-pass cost added to the validation budget.

## Comparison of read mechanisms (2026-11-17)

The full-table read costs about 5 ms per frontend, so we measured two cheaper mechanisms on the same host and under the same load. The probe mode is `compare`. It builds its own maps with the dataplane's shapes: a `HASH_OF_MAPS` outer map with key 8 and value 4, and inner `ARRAY` maps of u32 → u32. Each run has 1,000 frontends with 16 backends. "Apply" means creating the inner map, filling it and swapping it into the outer map, which is the write side of `update_service`.

| Mechanism | Read p50 | Read p99 | Sweep of 1,000 | Apply p50 | Correct |
|---|---|---|---|---|---|
| Baseline: batch scan of the 16,381-slot table | 5.41 ms | 8.21 ms | 5.84 s | 17.2 ms | yes |
| Fingerprint: 64-bit applied-set fingerprint in two extra inner slots (16381, 16382), written before the outer swap; read = outer lookup + `GET_FD_BY_ID` + two element lookups | **4.7 µs** | 6.3 µs | **6.2 ms** | 13.5 ms | yes |
| Mmap: inner maps `BPF_F_MMAPABLE`; read = outer lookup + `GET_FD_BY_ID` + `mmap` + slot scan | 107 µs | 137 µs | 112 ms | 15.2 ms | yes |

```
Baseline: frontends=1000 backends=16 read p50=5.408385ms p99=8.209874ms sweep(all 1000)=5.840446716s correct=true | apply(create+fill+swap) p50=17.167684ms p99=32.79474ms
Fingerprint: frontends=1000 backends=16 read p50=4.677µs p99=6.3µs sweep(all 1000)=6.17547ms correct=true | apply(create+fill+swap) p50=13.46591ms p99=18.425548ms
Mmap: frontends=1000 backends=16 read p50=106.761µs p99=137.237µs sweep(all 1000)=111.583756ms correct=true | apply(create+fill+swap) p50=15.245159ms p99=18.477407ms
```

Why the baseline is slow: `generic_map_lookup_batch` walks the array one key at a time, calling `map_get_next_key`, `bpf_map_copy_value` and `copy_to_user` per slot (`kernel/bpf/syscall.c:2093-2108 @ v7.0`). That is 16,381 round trips to copy 64 KB.

### CPU and kernel memory (second run, same host and load)

CPU is the thread's user plus system time from `getrusage(RUSAGE_THREAD)`. Memory is the kernel's `memlock` accounting from `/proc/self/fdinfo/<fd>`.

| Mechanism | CPU per read | CPU per apply | Kernel memory per inner map | 1,000 live inner maps |
|---|---|---|---|---|
| Baseline | 5.36 ms | 13.8 ms | 131,360 B | 128,281 KiB |
| Fingerprint | **4.8 µs** | 13.9 ms | 131,376 B (**+16 B**) | 128,296 KiB (+15 KiB, +0.012%) |
| Mmap | 129 µs | 14.5 ms | 135,168 B (+3,808 B, page-aligned) | 132,000 KiB (+3.6 MiB, +2.9%) |

```
Baseline:   CPU (thread user+sys): read sweep=5.359998s (5.359998ms/read) | apply all=13.798616s (13.798616ms/apply)
Baseline:   kernel memlock: inner map=131360 B each, x1000 live = 128281 KiB; outer map=328704 B
Fingerprint:   CPU (thread user+sys): read sweep=4.838ms (4.838µs/read) | apply all=13.891054s (13.891054ms/apply)
Fingerprint:   kernel memlock: inner map=131376 B each, x1000 live = 128296 KiB; outer map=328704 B
Mmap:   CPU (thread user+sys): read sweep=128.966ms (128.966µs/read) | apply all=14.470871s (14.470871ms/apply)
Mmap:   kernel memlock: inner map=135168 B each, x1000 live = 132000 KiB; outer map=328704 B
```

- **Read CPU matches read latency.** All three reads are CPU-bound syscall work; nothing waits.
- **The fingerprint's memory cost is the two slots.** A non-mmapable array element is the value size rounded up to 8 bytes (`kernel/bpf/arraymap.c:93 @ v7.0`), so two slots add 16 bytes.
- **Mmap costs more memory.** It rounds the array up to whole pages.
- **Apply cost is the same for all three** (about 14 ms of CPU). The probe fills each table with one update per slot, so this number shows the three mechanisms do not differ; it is not a measurement of production's fill path.
- **During `update_service`, the old inner map lives until its last reference drops.** Each live service briefly holds two tables. That is true for all three mechanisms.

### Fingerprint

- **It reads the resource, not a receipt.** The word lives in the inner map, is written in the same atomic outer swap as the table, and disappears with the map on restart or reboot. This is not the View-marker anti-pattern in `reconcilers.md`, which is a hash the reconciler keeps of what it emitted.
- **It compares the whole applied set**, weights and health included. That removes design implication 1, because a backend with no Maglev slot no longer matters. It needs no `BACKEND_MAP` lookups.
- **What it does not detect:** a `BACKEND_MAP` entry lost while the inner map survives. Only an Overdrive bug could cause that, because both maps are created and destroyed together.
- **XDP is untouched, but this is unverified.** XDP indexes the table modulo 16,381, so the extra slots are never selected. This probe ran no XDP program. The existing Tier-3 dataplane tests must confirm it once the inner capacity becomes 16,383.

### Mmap

- **Kernel support:** mmap is allowed for `ARRAY` inner maps (`kernel/bpf/arraymap.c:66-68, 578-591 @ v7.0`). The prototype and every inner map must carry the flag, because `bpf_map_meta_equal` compares `map_flags` (`kernel/bpf/map_in_map.c:90 @ v7.0`).
- **The reader must open the map read-write**, even though it only maps it `PROT_READ`. On a read-only file, `MAP_SHARED` clears `VM_SHARED` (`mm/mmap.c:465 @ v7.0`), and `bpf_map_mmap` then returns `EINVAL` (`kernel/bpf/syscall.c:1095-1096 @ v7.0`). The probe hit this first.
- **The 107 µs excludes the `BACKEND_MAP` lookups**, about 80 µs at 16 backends (measured above). It keeps the full membership read, so implication 1 still applies.

### Recommendation

Use the fingerprint. Its read takes about 1,000× less CPU than the baseline and about 25× less than mmap. It costs 16 bytes of kernel memory per service, and its apply is no slower. It also removes the slot-starvation precondition. A 1,000-service pass spends about 5 ms of CPU reading instead of 5.4 s.
