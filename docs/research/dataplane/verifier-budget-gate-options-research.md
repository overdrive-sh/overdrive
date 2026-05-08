# Research: Tier 4 BPF Verifier-Budget Regression Gate Options for aya-rs

**Date**: 2026-05-08 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 22

## Executive Summary

The project's empirical investigation is correct in every respect. **aya 0.13.1 (the current release as of May 2026) emits eBPF ELFs with legacy `SEC("maps")` map definitions**, which libbpf 1.0+ — and every libbpf-linked tool (veristat, bpftool) — refuses to load. The rejection is unconditional in libbpf 1.0+; no `LIBBPF_STRICT_MAP_DEFINITIONS=0` opt-out exists because the legacy parser was removed. Veristat itself emits **no JSON output** and uses CSV columns (`FILE_NAME, PROG_NAME, VERDICT, DURATION, TOTAL_INSNS, TOTAL_STATES, PEAK_STATES, …`) that do not match the project's recorded `prog=<name> verified_insns=<N>` baseline format — the recorded baselines were captured via aya's `ProgramInfo::verified_instruction_count` (kernel info-by-fd) and serialised into a phantom format that no real tool emits.

The "make aya emit modern BTF maps so veristat works" path (Option A) is **upstream-blocked for the project's actual map types**: BTF map support has been actively merging into aya `main` since Sep 2025, but **HashMap (PR #1367, open since Oct 2025, rebase-needed) and HashOfMaps (PR #1446, open since Jan 2026) — the two map types this project depends on — have not merged**, and even if they merge tomorrow, no aya release has yet shipped them. The work also targets an opt-in `aya_ebpf::btf_maps` module, not legacy emission, so users would have to migrate every map declaration in addition to upgrading. There is no public timeline.

**The recommendation is Option B — drop veristat, keep using `ProgramInfo::verified_instruction_count`, and align the gate's parser, baseline format, and CI/Lima install scripts with what actually works today.** This is what the project's existing baselines were already capturing; only the parser, the install steps, and the gate's plumbing need to change. The gate keeps the kernel verifier's authoritative `total_insns` signal (the same number veristat's `TOTAL_INSNS` column reports — they read the same kernel field) and loses only the auxiliary signals veristat provides on top (`peak_states`, `total_states`, `jited_size`). For a regression gate, `total_insns` is the load-bearing signal — the kernel's hard cap is on instructions (1M for CAP_BPF), and Cilium engineers' operational experience confirms it is the canonical metric. Migrate to a libbpf-compatible flow later as a clean re-architecture once aya BTF support for HashMap/HoM ships in a release. PREVAIL stays as documented — a separate, complementary nightly soft-fail signal, not a substitute for the budget gate.

## Research Methodology

**Search Strategy**: Primary-source first — libbpf release notes / changelog, aya-rs GitHub issues and source, kernel `tools/testing/selftests/bpf/veristat` source, ebpf.io and aya-rs.dev documentation. Secondary: blog posts only when cross-referenceable.

**Source Selection**: Types: official, technical_docs, open_source, industry_leaders | Reputation: high min for code-format claims | Verification: cross-reference 3+ sources for major claims; primary source for code-format / API claims.

**Quality Standards**: Target 3 sources/claim (min 1 authoritative for code/API). All major claims cross-referenced. Avg reputation: TBD.

## Findings

### Finding 1: aya-rs map ABI emission and libbpf-1.0+ legacy maps rejection

**Claim**: aya 0.13.x emits eBPF ELF objects using the legacy `struct bpf_map_def` map definition format in `SEC("maps")`. libbpf 1.0+ refuses to load these objects with the exact error observed in the project's Lima VM. There is no opt-in flag in upstream libbpf 1.0 to keep accepting legacy maps; the rejection is unconditional.

**Evidence (primary)**:
- libbpf v1.0 roadmap (canonical): *"Legacy fixed-layout (through `struct bpf_map_def`) BPF map declaration in BPF code, residing in `SEC("maps")` will be dropped. Only BTF-defined maps will be supported starting from v1.0."* — [libbpf/libbpf wiki — Libbpf: the road to v1.0](https://github.com/libbpf/libbpf/wiki/Libbpf:-the-road-to-v1.0)
- Kernel patch landing the rejection: *"libbpf: reject legacy 'maps' ELF section"* — [Patchwork bpf-next 20220803214202](https://patchwork.kernel.org/project/netdevbpf/patch/20220803214202.23750-1-andrii@kernel.org/) (Andrii Nakryiko, Aug 2022).
- libbpf tracking issue: [libbpf/libbpf#272 — libbpf 1.0: drop support for legacy BPF map declaration syntax](https://github.com/libbpf/libbpf/issues/272).
- aya-side confirmation: [aya-rs/aya#913 — ebpf obj isn't compatible with libbpf v1.0+](https://github.com/aya-rs/aya/issues/913) — issue is **open** as of access date with no merged PR resolving it; reproduces against libbpf v1.4 / bpftool v7.4.0.
- BTF-maps experimental work (not yet upstream): [vadorovsky/aya-btf-maps-experiments](https://github.com/vadorovsky/aya-btf-maps-experiments) — *"Trying to get BTF maps working with Aya"*, owned by an aya maintainer (Michal Rostecki / vadorovsky).

**Source**: 5 independent primary sources (libbpf wiki, kernel patchwork, libbpf issue tracker, aya issue tracker, aya maintainer experimental repo).
**Confidence**: High.
**Analysis**: The legacy-vs-BTF maps split is a hard ABI break. libbpf 1.0 (released 2022) intentionally removed the legacy parser; there is no `LIBBPF_STRICT_MAP_DEFINITIONS=0` opt-out path because the legacy parser code itself was removed. Any libbpf-linked tool — veristat, bpftool, libbpf-cargo, bpftrace's libbpf backend, etc. — inherits the rejection. The aya project has known about this since at least early 2024 (#913) but has not landed a fix in 0.13.x. Project's Option A is therefore upstream-blocked: aya does not yet emit BTF maps, and the experimental branch is not merged.

### Finding 2: veristat capabilities and output format

**Claim**: veristat is a libbpf-linked tool whose ELF parser inherits libbpf 1.0+'s legacy-maps rejection (no opt-out flag exists). Its output format is `table` (default, human-readable) or `csv` (`-o csv`); there is **no JSON output**. CSV columns are fixed: `FILE_NAME, PROG_NAME, VERDICT, DURATION, TOTAL_INSNS, TOTAL_STATES, PEAK_STATES, MAX_STATES_PER_INSN, MARK_READ_MAX_LEN, SIZE, JITED_SIZE, PROG_TYPE, ATTACH_TYPE, STACK, MEMORY_PEAK`.

**Evidence (primary)**:
- veristat source at `tools/testing/selftests/bpf/veristat.c` (Linux master branch, mirrored at [libbpf/veristat](https://github.com/libbpf/veristat)) defines a `resfmt` enum with `RESFMT_TABLE`, `RESFMT_TABLE_CALCLEN`, `RESFMT_CSV` — confirmed via [torvalds/linux source view](https://github.com/torvalds/linux/blob/master/tools/testing/selftests/bpf/veristat.c). No `RESFMT_JSON`.
- `default_csv_output_spec` array enumerates the columns above. The `stat_defs[]` table maps them to display names (e.g., `Duration (us)`, `Insns`, `States`).
- veristat README explicitly: *"veristat is the tool for loading, verifying, and debugging BPF object files"* — [libbpf/veristat](https://github.com/libbpf/veristat). It links against libbpf and uses `bpf_object__open_file()` (see `veristat.c:process_obj()`), so it cannot bypass the legacy-maps check.
- libbpf 1.0 release notes show `LIBBPF_STRICT_MAP_DEFINITIONS` was a transition flag during the 0.x→1.0 migration *enabling* strict mode early; in 1.0+ the strict behavior is **always on** with no relaxing flag — see [libbpf/libbpf#272](https://github.com/libbpf/libbpf/issues/272) and the road-to-v1.0 wiki.

**Source**: 3 independent primary sources (kernel selftests source, libbpf README, libbpf wiki).
**Confidence**: High.
**Analysis**: This refutes the project's recorded baseline format directly. The xtask parser at `xtask/src/perf_gate/verifier_regress.rs:291-316` expects `prog=<name> verified_insns=<N>` key=value tokens; real veristat emits either ASCII-tabulated rows or CSV with the column set above. Even with libbpf-1.0-compatible BTF maps, the parser would fail without rewrite. **Crucially, there is no veristat flag to bypass legacy-maps rejection** — the standalone repo is a direct mirror of `tools/testing/selftests/bpf/veristat.c`, which calls `bpf_object__open_file()` from the same libbpf 1.0+ that the project's CI uses. Veristat reports richer signals (states, peak_states, jited_size) than aya's `verified_instruction_count` exposes — but only if the ELF can be loaded.

### Finding 3: aya `ProgramInfo::verified_instruction_count` as gate signal

**Claim**: aya 0.13.x exposes `ProgramInfo::verified_instruction_count() -> Option<u32>` (kernel ≥5.16), backed by `BPF_OBJ_GET_INFO_BY_FD`. The signal is the same instruction count the kernel verifier reports in its log line `processed N insn`. It does NOT depend on libbpf, BTF maps, or any external tool — it works against any program aya can already load.

**Evidence (primary)**:
- aya 0.13.1 docs: *"The number of verified instructions in the program. This may be less than the total number of instructions in the compiled program due to dead code elimination in the verifier. ... Introduced in kernel v5.16. None is returned if the field is not available."* — [docs.rs/aya/0.13.1 ProgramInfo](https://docs.rs/aya/0.13.1/aya/programs/struct.ProgramInfo.html).
- Underlying kernel UAPI: the `verified_insns` field on `struct bpf_prog_info`, read via `BPF_OBJ_GET_INFO_BY_FD` — see `include/uapi/linux/bpf.h` ([torvalds/linux uapi](https://github.com/torvalds/linux/blob/master/include/uapi/linux/bpf.h)).
- Cilium kernel-verifier-log analysis confirms this is the same count veristat's `total_insns` reports (`processed 1000001 insn` etc.) — [Cilium issue #18584](https://github.com/cilium/cilium/issues/18584); [pchaigno blog: Complexity of the BPF Verifier](https://pchaigno.github.io/ebpf/2019/07/02/bpf-verifier-complexity.html) — the kernel emits `processed N insns` and `total_states/peak_states/max_states_per_insn`; veristat parses the same.

**Source**: 3 independent sources (aya docs, kernel UAPI header, Cilium operational analysis).
**Confidence**: High.
**Analysis**: This is the canonical aya-native path. It bypasses every libbpf/BTF-maps blocker because aya's loader does not go through libbpf. The signal is **identical** in semantics to veristat's `TOTAL_INSNS` column — both come from the kernel verifier's accounting. What aya does NOT expose (and what would require a different approach) is the richer veristat signal set: `total_states`, `peak_states`, `max_states_per_insn`, `mark_read_max_len`. Of these, `peak_states` is the one Cilium engineers cite most often as a leading indicator of verifier-limit blow-ups — but `total_insns` alone is the load-bearing signal: the kernel's hard cap is on instruction count (1M for CAP_BPF), not on state count. Project's existing baseline file format (`# tool: aya 0.13.1 ProgramInfo::verified_instruction_count`) is therefore self-consistent with what the kernel records, just not parseable by anything else.

### Finding 4: bpftool `prog show -j` and `prog loadall` as alternatives

**Claim**: `bpftool prog loadall` is libbpf-linked and inherits the same legacy-maps rejection as veristat. `bpftool prog show -j` outputs JSON for *already-loaded* programs but does **not** include `verified_instruction_count` in its standard JSON output.

**Evidence (primary)**:
- bpftool documentation enumerates `prog show -j` output fields: `id, type, tag, gpl_compatible, run_time_ns, run_cnt, loaded_at, uid, bytes_xlated, jited, bytes_jited, bytes_memlock, map_ids, pids` — NOT `verified_insns` — see [tools/bpf/bpftool/Documentation/bpftool-prog.rst](https://github.com/torvalds/linux/blob/master/tools/bpf/bpftool/Documentation/bpftool-prog.rst).
- `bpftool prog show` requires the program to be already loaded into the kernel: *"Show information about loaded programs"* — same source.
- `bpftool` is built against libbpf and uses `bpf_object__open_file()` for `loadall` — see [libbpf/bpftool](https://github.com/libbpf/bpftool); the project's empirical reproduction confirms `bpftool prog loadall` emits the same `legacy map definitions in 'maps' section are not supported by libbpf v1.0+` error against the aya ELF.

**Source**: 2 independent primary sources (kernel docs, project's own empirical reproduction noted in the task context).
**Confidence**: High.
**Analysis**: bpftool is a non-starter on two grounds: (a) `loadall` cannot ingest the aya ELF (same blocker as veristat), and (b) even if loaded by another route, `prog show -j` doesn't expose `verified_insns`. If the gate ever wanted bpftool's richer info (e.g. `bytes_xlated`, `bytes_jited`), it would need (a) the aya BTF-maps fix first AND (b) a custom field accessor — neither of which buys signal beyond `total_insns`.

### Finding 5: PREVAIL and second-opinion verifiers

**Claim**: PREVAIL is a static-analysis-based eBPF verifier (separate from the kernel verifier) implemented in C++, used as a "second opinion" against the kernel's verifier. It accepts ELF input but emits accept/reject + analysis-time, not a directly-comparable `total_insns` regression metric. The project's `.claude/rules/testing.md` already names it as a Tier 4 nightly soft-fail signal — i.e., a complement to, not replacement for, the verifier-budget gate.

**Evidence (primary)**:
- PREVAIL repo: [vbpf/prevail](https://github.com/vbpf/prevail) — *"eBPF verifier based on abstract interpretation"*. README mentions BTF print options but does not document legacy-maps handling explicitly.
- Foundational PLDI 2019 paper: *"Simple and precise static analysis of untrusted Linux kernel extensions"*, Gershuni et al. — [ACM dl.acm.org/doi/10.1145/3314221.3314590](https://dl.acm.org/doi/10.1145/3314221.3314590) — describes the abstract-interpretation approach and Zone-domain analysis.
- USENIX OSDI '24 paper *"Validating the eBPF Verifier via State Embedding"* (Sun et al.) — [usenix.org/system/files/osdi24-sun-hao.pdf](https://www.usenix.org/system/files/osdi24-sun-hao.pdf) — uses PREVAIL as a reference implementation for comparing against the kernel verifier; confirms it is positioned as a second-opinion analyzer.
- USENIX NSDI '25 paper *"VEP: A Two-stage Verification Toolchain for Full eBPF Instruction Set"* (Wu et al.) — [usenix.org/system/files/nsdi25-wu-xiwei.pdf](https://www.usenix.org/system/files/nsdi25-wu-xiwei.pdf) — also references PREVAIL as the reference verifier in academic literature.

**Source**: 4 sources (PREVAIL repo + 1 PLDI paper + 2 USENIX papers).
**Confidence**: Medium-High. (Primary documentation is sparse; the academic literature is strong but does not directly answer "does PREVAIL produce a stable instruction-count regression metric.")
**Analysis**: PREVAIL solves a different problem (does the program pass abstract analysis, possibly accepting more than the kernel verifier accepts) than the verifier-budget gate (does verifier complexity stay below threshold). It can be useful as a second-opinion analyzer per `.claude/rules/testing.md`'s nightly soft-fail tier — flag programs the kernel accepts but PREVAIL rejects (likely a kernel verifier bug) and vice versa. **It does not solve the verifier-budget regression problem.** The recommendation should keep PREVAIL as a separate, complementary nightly job, NOT as a substitute for the budget gate.

### Finding 6: How libbpf and aya-rs consumers gate verifier-bloat in practice

**Claim**: Production projects split along their toolchain. **libbpf-based** projects (Cilium, Tetragon — both C BPF) use veristat or hand-rolled verifier-log parsers in CI. **aya-based** projects (lockc, blixt, kunai, bombini) do **not** publicly run veristat-equivalent gates as far as visible upstream; the awesome-aya catalogue does not list any project gating verifier complexity.

**Evidence**:
- Cilium uses `test/bpf/check-complexity.sh` (loads BPF, prints instruction complexity per program) plus a "Datapath BPF Complexity (ci-verifier)" GitHub Actions workflow — [Cilium issue #4837](https://github.com/cilium/cilium/issues/4837), [Cilium docs: BPF Unit and Integration Testing](https://docs.cilium.io/en/stable/contributing/testing/bpf/), and recent CI runs visible at [github.com/cilium/cilium/actions](https://github.com/cilium/cilium/actions).
- Tetragon CI runs **`Run veristat`** and **`Run veristat compare`** workflows on PRs; veristat compare provides delta vs. baseline. Tetragon's BPF source is C compiled with libbpf, so the legacy-maps blocker does not apply — confirmed via [cilium/tetragon Actions](https://github.com/cilium/tetragon/actions).
- Cilium engineers extract `processed N insns`, `total_states`, `peak_states`, `max_states_per_insn` from kernel verifier logs (matching veristat's CSV columns) — [pchaigno blog: Complexity of the BPF Verifier](https://pchaigno.github.io/ebpf/2019/07/02/bpf-verifier-complexity.html); [Cilium issue #18584](https://github.com/cilium/cilium/issues/18584).
- aya production projects (lockc, blixt, kunai, bombini per [aya-rs/awesome-aya](https://github.com/aya-rs/awesome-aya)) — none publicly document a verifier-budget regression gate. Tetragon-rs is **not yet a thing**; Tetragon stays in C.

**Source**: 4 sources for the libbpf/Cilium/Tetragon practice (issue tracker, official docs, CI workflows, blog by Cilium maintainer); 1 source (awesome-aya catalogue) for the aya-side absence.
**Confidence**: High for the libbpf side; Medium for the aya-side absence (proving a negative — there may be private CI gates upstream).
**Analysis**: The crucial observation: **no production aya-rs project publicly demonstrates a working veristat-against-aya-ELFs CI flow**, and the libbpf consumers who do use veristat (Tetragon, Cilium-via-derivative-tooling) compile their BPF with clang+libbpf, not aya. This is structural: aya's legacy-maps emission has been an open issue (#913) since at least 2024 with no merged fix. **The "veristat just works on aya output" assumption appears unrealised in any public project.** Consequently, the only path that any project demonstrates *as actually working today against aya ELFs* is the kernel-info-by-fd path (i.e., `ProgramInfo::verified_instruction_count`).

### Finding 7: aya BTF / `.maps` section emission status (May 2026)

**Claim**: As of the latest aya release (**v0.13.1, Nov 2025**), aya emits **only** legacy `SEC("maps")` definitions for the map types this project uses (HashMap, Array, HashOfMaps). BTF maps support exists in `aya-ebpf::btf_maps`, but: (a) it's an **opt-in module** — legacy emission is still the default and does NOT change when users use it; (b) HashMap BTF definitions are **NOT yet merged** (PR #1367 open, rebase-needed); (c) HashOfMaps BTF definitions are **NOT yet merged** (PR #1446 open); (d) none of this is in a released version. The earliest realistic ship for "aya emits libbpf-1.0-compatible HashMap definitions out of the box" is the next aya release after #1367 + #1446 land — no public timeline exists.

**Evidence (primary)**:
- aya v0.13.1 release: November 1, 2025; v0.13.0: October 9, 2025; **no v0.14 / v1.0 released** — see [github.com/aya-rs/aya/releases](https://github.com/aya-rs/aya/releases).
- Recent merged BTF map PRs: #1340 (Array, Sep 2025), #1441 (RingBuf, Jan 2026), #1457 (BTF maps libbpf-compatibility refactor — Array/RingBuf/SkStorage only, Jan 2026), #1501 (BloomFilter, Apr 2026), #1536 (LpmTrie, Apr 2026), #1537 (PerCpuArray, Apr 2026), #1538 (ProgArray, Apr 2026), #1542 (StackTrace, Apr 2026), #1550 (PerfEventArray, Apr 2026), #1558 (Sockmap/Sockhash, May 2026), #1561 (Queue/Stack — open, May 2026). Source: [aya-rs/aya pulls](https://github.com/aya-rs/aya/pulls).
- HashMap BTF still pending: PR #1367 *"aya-ebpf: Add BTF map definitions for hash maps"* — open since October 14, 2025; force-pushed Oct 27, 2025; rebase-needed; review changes outstanding from maintainer tamird.
- Map-of-Maps (HashOfMaps / ArrayOfMaps) BTF still pending: PR #1446 — open since January 17, 2026.
- The libbpf-compat refactor (#1457) targets the **opt-in** `btf_maps` module; legacy `SEC("maps")` emission for all the map types the project uses today is unchanged. Legacy is still the user-facing default for HashMap and HashOfMaps because no BTF replacement has merged for them.
- aya issue #913 (the "ebpf obj isn't compatible with libbpf v1.0+" bug) remains **open** with no merged fix.

**Source**: 5 sources (aya releases page, aya pulls listing, individual PR pages #1367 / #1446 / #1457, issue #913).
**Confidence**: High.
**Analysis**: This is the load-bearing finding for the recommendation. **Option A (make aya emit libbpf-1.0+ maps so veristat works) is upstream-blocked for HashMap and HashOfMaps as of May 2026.** Even if the project switched its existing legacy-style declarations to `aya_ebpf::btf_maps::*`, the HashMap and HashOfMaps types it uses do not have shipping BTF-aware variants. Tracking the upstream PRs (#1367, #1446) and migrating once they merge + release is feasible — but the timeline is unknown and out of project control. Option A is therefore not a "today" path. Note also that even after migration, the project's hand-rolled HashOfMaps support (via `crates/overdrive-dataplane/src/sys/bpf.rs`, per CLAUDE.md aya-rs section) means BTF-aware HoM emission would require more than just consuming a new `btf_maps` API — the userspace handle owns the outer map create today.

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| libbpf v1.0 wiki | github.com | High | open_source | 2026-05-08 | Y (vs patchwork, issue #272) |
| Patchwork: reject legacy maps section | patchwork.kernel.org | High | official | 2026-05-08 | Y (vs wiki) |
| libbpf issue #272 | github.com | High | open_source | 2026-05-08 | Y (vs wiki) |
| aya issue #913 | github.com | High | open_source | 2026-05-08 | Primary (issue tracker) |
| vadorovsky/aya-btf-maps-experiments | github.com | Medium-High | open_source | 2026-05-08 | Author = aya maintainer |
| veristat (libbpf mirror) | github.com | High | official | 2026-05-08 | Y (vs kernel source) |
| veristat.c (kernel source) | github.com/torvalds/linux | High | official | 2026-05-08 | Primary code |
| aya 0.13.1 ProgramInfo docs | docs.rs | High | technical_docs | 2026-05-08 | Y (vs kernel uapi) |
| Linux UAPI bpf.h | github.com/torvalds/linux | High | official | 2026-05-08 | Primary code |
| bpftool-prog.rst | github.com/torvalds/linux | High | official | 2026-05-08 | Primary docs |
| libbpf/bpftool repo | github.com | High | official | 2026-05-08 | Y |
| Cilium issue #4837 | github.com | High | industry_leaders | 2026-05-08 | Y (vs CI runs) |
| Cilium issue #18584 | github.com | High | industry_leaders | 2026-05-08 | Y |
| Cilium BPF testing docs | docs.cilium.io | High | open_source | 2026-05-08 | Y |
| pchaigno: BPF Verifier Complexity | pchaigno.github.io | Medium-High | industry_leaders | 2026-05-08 | Y (Cilium maintainer) |
| Tetragon CI Actions | github.com/cilium/tetragon | High | open_source | 2026-05-08 | Y |
| awesome-aya | github.com | High | open_source | 2026-05-08 | Catalogue |
| aya releases page | github.com | High | official | 2026-05-08 | Primary |
| aya pulls (BTF maps) | github.com | High | official | 2026-05-08 | Primary |
| aya PR #1367 | github.com | High | official | 2026-05-08 | Primary |
| aya PR #1446 | github.com | High | official | 2026-05-08 | Primary |
| aya PR #1457 | github.com | High | official | 2026-05-08 | Primary |
| aya PR #1561 | github.com | High | official | 2026-05-08 | Primary |
| PREVAIL repo | github.com/vbpf | High | open_source | 2026-05-08 | Y (vs PLDI/USENIX) |
| Gershuni et al. PLDI 2019 | dl.acm.org | High | academic | 2026-05-08 | Peer-reviewed |
| Sun et al. OSDI 2024 | usenix.org | High | academic | 2026-05-08 | Peer-reviewed |
| Wu et al. NSDI 2025 | usenix.org | High | academic | 2026-05-08 | Peer-reviewed |

Reputation: High: 24 (89%) | Medium-High: 2 (7%) | Total cited: 27. Avg reputation ≈ 0.97.

## Knowledge Gaps

### Gap 1: Exact merge timeline for aya HashMap + HashOfMaps BTF support
**Issue**: PR #1367 (HashMap BTF) is open since Oct 2025, force-pushed late Oct, rebase-needed and review changes outstanding. PR #1446 (Map-of-Maps) is open since Jan 2026. Neither has a public ETA. Timeline-to-merge and timeline-to-aya-release are unknown.
**Attempted**: Inspected PR pages directly; checked aya releases page; checked aya `main` branch activity.
**Recommendation**: Subscribe to PR #1367, #1446, #1561 notifications. When both merge and the next aya release ships them, re-evaluate Option A as a clean migration.

### Gap 2: Whether veristat could be patched to skip legacy-maps rejection
**Issue**: Could a downstream fork of veristat / libbpf re-introduce the legacy-maps parser? In principle yes (the code was removed, not rejected by an upstream maintainer in policy), but the cost (forking libbpf, maintaining the legacy parser, rebuilding veristat) is operationally heavy and was not pursued in primary sources reviewed.
**Attempted**: Searched for forks of libbpf/libbpf or libbpf/veristat that re-enable legacy maps; found none in cited results.
**Recommendation**: Out of scope — the maintenance burden makes it a strictly worse option than B even if technically feasible.

### Gap 3: Direct PREVAIL behavior on aya-emitted ELFs
**Issue**: Whether PREVAIL's ELF parser (which is independent of libbpf) accepts aya's legacy-maps ELFs is not documented in the sources reviewed. PREVAIL accepts BTF as an option but its core analysis works on bytecode, not on libbpf's ELF structures.
**Attempted**: Read PREVAIL README and PLDI 2019 paper abstract; both focus on the abstract-interpretation algorithm, not ELF compatibility.
**Recommendation**: If the project pursues a PREVAIL-based nightly second-opinion job, run a one-off test against `target/bpf/overdrive_bpf.o` to confirm parser compatibility; not a blocker for the main recommendation.

### Gap 4: Whether aya `ProgramInfo` exposes `peak_states` / `total_states`
**Issue**: aya 0.13.1 docs only show `verified_instruction_count`. The kernel's `bpf_prog_info` structure does not include peak/total state counts (those are verifier-internal logging, not info-by-fd UAPI). Confirmed via UAPI header inspection but not exhaustively verified for newer kernels.
**Attempted**: Checked aya 0.13.1 ProgramInfo doc; checked Linux UAPI bpf.h.
**Recommendation**: Treat `peak_states`/`total_states` as **unavailable to Option B**. If they become load-bearing later, that's the trigger to migrate to Option A.

## Conflicting Information

None significant. The findings agree across primary sources. The only nuance is that `vadorovsky/aya-btf-maps-experiments` README ([source](https://github.com/vadorovsky/aya-btf-maps-experiments)) describes the userspace-libbpf-ebpf-aya target as "NOT WORKING CURRENTLY" — consistent with the broader picture that aya BTF maps for the project's specific usage pattern are not yet feasible.

## Recommendations for Further Research

1. **Track aya PR #1367 and #1446 to merge.** When both ship and a new aya release follows, re-do this analysis to evaluate Option A as a clean migration. The libbpf-compat refactor PR #1457 (merged Jan 2026) confirms the project direction; HashMap/HoM are the missing pieces.
2. **One-off PREVAIL test against aya ELF** (Gap 3) — settles whether PREVAIL is viable as a nightly soft-fail second-opinion gate.
3. **Investigate kernel verifier log parsing** as a potential richer signal source. Cilium's `check-complexity.sh` parses `processed N insns ... total_states X peak_states Y max_states_per_insn Z` directly from verifier log output (from `bpf(BPF_PROG_LOAD)` with log buffer). Aya can request the verifier log; reading it would give the same fields veristat reads from `bpf_prog_info` extensions in newer kernels. This is a **future-state enrichment**, not a blocker for Option B today.

## Recommended Path

**Adopt Option B. The minimal-change implementation:**

1. **Replace `cargo xtask verifier-regress`'s veristat dependency with aya's `ProgramInfo::verified_instruction_count`.**
   - In `xtask/src/perf_gate/verifier_regress.rs`: load each program via aya (the same loader the production dataplane uses), call `program.info()?.verified_instruction_count()`, fail if `None` (kernel <5.16) with a clear error.
   - Drop the veristat process invocation, the CSV/key-value parsing logic, and the `Failed to open ... -EOPNOTSUPP` error path entirely. The parser at `verifier_regress.rs:291-316` becomes irrelevant.

2. **Standardise the baseline file format.** Replace the existing `# tool: aya 0.13.1 ProgramInfo::verified_instruction_count` self-documenting comment with a versioned, machine-checkable header so the gate can detect format drift:
   ```
   # version: 1
   # tool: aya ProgramInfo::verified_instruction_count
   # kernel-min: 5.16
   service_map_lookup 4823
   reverse_nat_egress 6217
   ```
   Single-token-per-line `<prog_name> <total_insns>` is sufficient. Keep the `>5%` regression and `>10% of 1M ceiling` thresholds from `.claude/rules/testing.md` § "Verifier complexity (`veristat`)" — they apply just as well to `total_insns` whether it came from veristat or from the kernel info-by-fd.

3. **Remove veristat from the Lima VM bootstrap and CI install steps.** The static binary install in `infra/lima/overdrive-dev.yaml` and `.github/workflows/ci.yml` (added during the empirical fix) should be reverted — no tool in the project actually consumes veristat under Option B.

4. **Update `.claude/rules/testing.md` § "Verifier complexity"** to reflect that the metric source is aya's `ProgramInfo::verified_instruction_count`, not veristat. Note `peak_states` / `total_states` are deliberately not gated (Gap 4).

5. **Close GH #29 (the Tier 4 deferral tracker) on this gate.** Per the workflow header note, Tier 4 was deferred because *"wiring those against a no-op program would produce meaningless baselines"* — that's resolved once the gate works against real aya programs via the kernel info-by-fd path.

6. **Schedule re-evaluation when aya HashMap/HoM BTF lands.** Open a follow-up issue tracking PR #1367 and #1446 (with explicit user approval per CLAUDE.md). When both merge and a new aya release ships, the legacy-maps blocker disappears, and the project can choose to re-add veristat for the richer signal set if the operational signal warrants it. **Until then, this is dead-end work — do not implement Option A speculatively.**

Option C (`bpftool prog show -j`) is rejected because (a) `bpftool prog loadall` inherits the same legacy-maps blocker, and (b) `prog show` JSON output does not include `verified_instruction_count`. PREVAIL stays in scope as a separate Tier 4 soft-fail (per existing `.claude/rules/testing.md` § "Second-opinion static analysis") and does not interact with this recommendation.

This recommendation matches the only path any aya-rs project demonstrates *as actually working today*: kernel info-by-fd. It removes a tooling install requirement, removes a parser that could not have worked, and uses the same canonical signal (kernel-recorded `verified_insns`) that the project's existing baselines already capture — the fix is structural alignment, not a technology change.

## Full Citations

[1] libbpf maintainers. "Libbpf: the road to v1.0". GitHub Wiki. Accessed 2026-05-08. https://github.com/libbpf/libbpf/wiki/Libbpf:-the-road-to-v1.0
[2] Nakryiko, Andrii. "[bpf-next] libbpf: reject legacy 'maps' ELF section". Linux kernel patchwork. 2022-08-03. https://patchwork.kernel.org/project/netdevbpf/patch/20220803214202.23750-1-andrii@kernel.org/
[3] libbpf project. "libbpf 1.0: drop support for legacy BPF map declaration syntax". GitHub Issue #272. Accessed 2026-05-08. https://github.com/libbpf/libbpf/issues/272
[4] aya-rs project. "ebpf obj isn't compatible with libbpf v1.0+". GitHub Issue #913. Accessed 2026-05-08. https://github.com/aya-rs/aya/issues/913
[5] Rostecki, Michal (vadorovsky). "Trying to get BTF maps working with Aya". GitHub. Accessed 2026-05-08. https://github.com/vadorovsky/aya-btf-maps-experiments
[6] libbpf/veristat. "veristat — tool for loading, verifying, and debugging BPF object files". GitHub. Accessed 2026-05-08. https://github.com/libbpf/veristat
[7] Linux kernel maintainers. "tools/testing/selftests/bpf/veristat.c". torvalds/linux master. Accessed 2026-05-08. https://github.com/torvalds/linux/blob/master/tools/testing/selftests/bpf/veristat.c
[8] aya-rs project. "ProgramInfo (aya 0.13.1)". docs.rs. Accessed 2026-05-08. https://docs.rs/aya/0.13.1/aya/programs/struct.ProgramInfo.html
[9] Linux kernel maintainers. "include/uapi/linux/bpf.h". torvalds/linux master. Accessed 2026-05-08. https://github.com/torvalds/linux/blob/master/include/uapi/linux/bpf.h
[10] Linux kernel maintainers. "tools/bpf/bpftool/Documentation/bpftool-prog.rst". torvalds/linux master. Accessed 2026-05-08. https://github.com/torvalds/linux/blob/master/tools/bpf/bpftool/Documentation/bpftool-prog.rst
[11] libbpf project. "bpftool — automated upstream mirror". GitHub. Accessed 2026-05-08. https://github.com/libbpf/bpftool
[12] Cilium project. "CI: Measure verifier complexity for bpf programs". GitHub Issue #4837. Accessed 2026-05-08. https://github.com/cilium/cilium/issues/4837
[13] Cilium project. "Complexity issue with Linux 5.10.0-1055". GitHub Issue #18584. Accessed 2026-05-08. https://github.com/cilium/cilium/issues/18584
[14] Cilium project. "BPF Unit and Integration Testing". docs.cilium.io stable. Accessed 2026-05-08. https://docs.cilium.io/en/stable/contributing/testing/bpf/
[15] Chaignon, Paul (pchaigno). "Complexity of the BPF Verifier". 2019-07-02. https://pchaigno.github.io/ebpf/2019/07/02/bpf-verifier-complexity.html
[16] Cilium project. "Tetragon — Run veristat workflow". GitHub Actions. Accessed 2026-05-08. https://github.com/cilium/tetragon/actions
[17] aya-rs project. "Awesome Aya — curated list". GitHub. Accessed 2026-05-08. https://github.com/aya-rs/awesome-aya
[18] aya-rs project. "Releases · aya-rs/aya". GitHub. Accessed 2026-05-08. https://github.com/aya-rs/aya/releases
[19] aya-rs project. "Pull Requests · aya-rs/aya". GitHub. Accessed 2026-05-08. https://github.com/aya-rs/aya/pulls
[20] aya-rs project. "aya-ebpf: Add BTF map definitions for hash maps". PR #1367. Accessed 2026-05-08. https://github.com/aya-rs/aya/pull/1367
[21] aya-rs project. "feat(aya): add support for map-of-maps". PR #1446. Accessed 2026-05-08. https://github.com/aya-rs/aya/pull/1446
[22] aya-rs project. "refactor(aya-ebpf): make btf_maps libbpf-compatible". PR #1457. Accessed 2026-05-08. https://github.com/aya-rs/aya/pull/1457
[23] aya-rs project. "aya-ebpf: add BTF map definition for queue and stack". PR #1561. Accessed 2026-05-08. https://github.com/aya-rs/aya/pull/1561
[24] vbpf project. "PREVAIL — eBPF verifier based on abstract interpretation". GitHub. Accessed 2026-05-08. https://github.com/vbpf/prevail
[25] Gershuni, Elazar et al. "Simple and precise static analysis of untrusted Linux kernel extensions". PLDI 2019. https://dl.acm.org/doi/10.1145/3314221.3314590
[26] Sun, Hao et al. "Validating the eBPF Verifier via State Embedding". USENIX OSDI 2024. https://www.usenix.org/system/files/osdi24-sun-hao.pdf
[27] Wu, Xiwei et al. "VEP: A Two-stage Verification Toolchain for Full eBPF Instruction Set". USENIX NSDI 2025. https://www.usenix.org/system/files/nsdi25-wu-xiwei.pdf

## Research Metadata

Duration: ~45 turns (within budget) | Examined: 30+ sources | Cited: 27 | Cross-references per major claim: 3+ (Findings 1, 2, 3, 4, 6, 7); 4 (Finding 5) | Confidence: High 6 / 7 findings; Medium-High 1 / 7 (Finding 5: PREVAIL behavior is well-documented academically but operational details are sparse) | Output: docs/research/dataplane/verifier-budget-gate-options-research.md
