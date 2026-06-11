# ADR-0068 — Pinned appliance kernel: pin the latest LTS that meets the feature floor (currently 6.18 LTS); the multi-kernel support matrix collapses to "pinned kernel + bpf-next soft-fail"

## Status

Accepted. 2026-06-11. Decision-makers: Morgan (solution-architecture,
proposing); ratified by the user 2026-06-11. Tags: phase-2, dataplane,
kernel, appliance-os, image-factory, mtls, ktls, sockops, test-matrix.

**Tracks**: roadmap 2.4 / [#26](https://github.com/overdrive-sh/overdrive/issues/26)
(transparent kernel mTLS — sockops + kTLS). This ADR settles the kernel
floor that #26's host-socket kTLS path and #222's guest-stack tap-proxy
path both build on.

**Supersedes**: the "5.10 LTS floor" / "support 5.10 → current LTS"
statements in **ADR-0053** (§ Consequences "Kernel floor unchanged",
~L572-575; § Fallback clause, ~L964) and **ADR-0059** (Kernel
compatibility row, ~L104; cleanup-mechanism note, ~L170-174; references,
~L284). Those back-references are updated to point here; the 4.17 /
5.7 / 5.14 *primitive*-availability facts they cite are unaffected — only
the **floor** they measure margin against moves from 5.10 to 6.18.

## Context

Overdrive ships its own immutable, minimal appliance OS (Image Factory,
§23) and **controls the kernel image it boots**. It is not a CNI plugin,
a sidecar, or a userspace agent that must run on an operator's arbitrary
kernel. Every production Overdrive node runs a kernel Overdrive built and
pinned.

The prior test matrix — `5.10, 5.15, 6.1, 6.6, current LTS` (whitepaper
§22; `.claude/rules/testing.md` § "Kernel matrix") — encoded the
*opposite* assumption: that Overdrive must run across a diverse range of
operator-supplied kernels, with 5.10 as the floor "first LTS with BPF
LSM, kTLS, and sockops jointly stable." That premise is false for an
appliance that owns its kernel, and it is not free. The 5.10 / 5.15
entries force a **degraded kTLS tier**: in-kernel TLS 1.3 **receive**-side
decryption did not land until **6.0** (`tls: rx: 1.3 support` series).
On 5.10 / 5.15 the only correct shapes are TX-only kTLS with userspace RX,
or full-userspace TLS — a fallback tier the transparent-mTLS architecture
(#26, #222) would otherwise have to carry and test for kernels Overdrive
never actually ships.

The completed research is consistent on the kernel facts and on the
appliance framing:

- `docs/research/dataplane/sockops-mtls-ktls-installation-comprehensive-research.md`
  — kTLS install mechanism; the RX-vs-TX kernel-version split.
- `docs/research/dataplane/sockops-ktls-plaintext-race-window-research.md`
  — the controlled-kernel implication: because Overdrive pins the kernel,
  an out-of-tree write-block patch is a legitimate option no
  upstream-bound mesh has.
- `docs/research/dataplane/transparent-mtls-recommended-architecture-research.md`
  — the decision-grade synthesis; pins the appliance kernel at an
  Overdrive-controlled LTS ("Overdrive controls it") as the premise of
  the recommended architecture.

At a pinned latest-LTS floor (currently 6.18) the degraded tier has no reason
to exist: in-kernel TLS 1.3 **TX + RX** is guaranteed, and the platform tests
exactly the one kernel it ships.

## Decision

### 1. Pin the latest LTS that meets the feature floor; refresh it deliberately

Overdrive pins the kernel its appliance OS boots. The pin is the **latest
LTS that meets the feature floor** (in-kernel TLS 1.3 TX+RX ≥6.0,
`CONFIG_NET_HANDSHAKE` ≥6.5), **refreshed deliberately at image rebuilds**
— **currently 6.18 LTS** (released 2025-11-30, EOL Dec 2028). Production
runs the pinned kernel; no Overdrive node runs a kernel Overdrive did not
build.

The decision is the **principle** — "track the latest qualifying LTS" — not
a frozen version number. A bare number goes stale: the prior pin (6.6 LTS,
EOL Dec 2027) was already two LTS generations behind by this revision, which
is exactly the trap this framing exists to avoid. 6.18 is today's concrete
instance of the principle, not the principle itself.

The pin is **advanced deliberately at image rebuilds**, because Overdrive
owns the image. Advancing the pin is a tested image change — re-validate the
dataplane's verifier-acceptance and complexity budgets on the new kernel
(every kernel release re-rolls the verifier), then ship — not an
operator-environment variable Overdrive must defensively support a range of.

### 2. The test matrix collapses to "pinned kernel + bpf-next soft-fail"

The Tier-3 kernel matrix is exactly two entries:

| Kernel | Role |
|---|---|
| **6.18 LTS** (the pin) | The one kernel Overdrive ships (latest qualifying LTS; EOL Dec 2028). Every Tier-3 / Tier-4 gate runs here; this is the merge-blocking signal. |
| **`bpf-next`** | Early-warning only. Soft-fail nightly. Catches upstream verifier / BPF-subsystem changes before a future pin bump adopts them. Never merge-blocking. |

The `5.10`, `5.15`, `6.1`, and `current LTS` entries are **removed**. They
tested kernels Overdrive does not ship; the diversity they bought was a
cost, not a guarantee.

### 3. This ADR is the required record for dropping kernels

`.claude/rules/testing.md` § "Kernel matrix" states: *"Dropping a kernel
requires an ADR."* This ADR is that record for dropping 5.10, 5.15, 6.1,
and current-LTS-as-a-distinct-entry. The propagation edits to
whitepaper §22, `testing.md`, and `c4-diagrams.md` land with this ADR so
the matrix SSOT stays in lockstep with the decision.

## Consequences

### Positive

- **Full in-kernel TLS 1.3 TX + RX is guaranteed.** At 6.18 the kTLS RX
  path (≥6.0) is always present. The #26 host-socket kTLS path and the
  #222 tap-proxy host-egress kTLS path both get kernel record-layer
  encrypt **and** decrypt with NIC offload — no userspace-RX fallback
  tier to design, ship, or test.
- **`CONFIG_NET_HANDSHAKE` is guaranteed.** The in-kernel TLS handshake
  upcall infrastructure (`net/handshake`, ≥6.5) is present at 6.18, so a
  `tlshd`-style handshake path is available to the design if the DESIGN
  wave wants it (it is an option, not a commitment here).
- **The out-of-tree write-block option is legitimised.** Because the
  kernel is pinned and owned, the custom in-kernel write-block patch the
  race-window research identifies (closes the sockops→kTLS plaintext
  race by *blocking* `write()` instead of `SK_DROP`-ing it) is a
  supportable appliance-OS patch carried across pin bumps — not the
  non-starter it is for any upstream-bound mesh.
- **The test matrix shrinks 6→2.** One blocking kernel + one soft-fail
  early-warning. Faster Tier-3 CI; the signal is exactly the kernel that
  ships, not an average over kernels that don't.
- **Pin is advanceable.** Overdrive moves the floor forward on its own
  schedule by shipping a new image — gaining future BPF features and kernel
  mechanisms when it chooses, not when an operator's distro does.
- **Maximum support runway before a forced pin-advance.** 6.18 is EOL
  Dec 2028 (vs the prior 6.6 pin's Dec 2027). Pinning the *newest*
  qualifying LTS — rather than an older one still in support — buys the
  longest window of upstream security backports before the standing
  responsibility to advance the pin (below) forces a rebuild.

### Negative

- **A single pinned kernel removes cross-kernel regression breadth.** The
  old matrix would catch a verifier behaviour change on 5.15 that 6.18
  masks. The `bpf-next` soft-fail lane is the replacement early-warning;
  it is nightly and non-blocking, so a regression it surfaces is caught
  before the *next* pin bump, not at the current one. For an appliance
  that ships one kernel this is the correct trade — but it is a real
  reduction in breadth, recorded here deliberately.
- **The floor is a moving commitment.** "Latest qualifying LTS" today is
  6.18; keeping the appliance current is Overdrive's standing
  responsibility. The refresh discipline is explicit: at each image
  rebuild, re-evaluate whether a newer LTS now qualifies, and advance the
  pin to it (re-validating the dataplane's verifier budgets on the new
  kernel before shipping). A pin that ages indefinitely accrues missing
  features and unpatched CVEs — the prior 6.6 pin drifting two LTS
  generations behind is the concrete failure mode this discipline exists
  to prevent. This is owned work, not a one-time decision.
- **In-place SVID rekey is unavailable to the recommended stack — the
  SOLE blocker is the userspace rustls→kTLS bridge; the kernel is ready.**
  TLS 1.3 `KeyUpdate`-driven in-place rekey of an established kTLS session
  needs support at **two** layers: (1) the **kernel** kTLS path (surface an
  inbound `KeyUpdate` to userspace and accept mid-stream re-keying via
  `setsockopt(TLS_TX/TLS_RX)`), and (2) the **userspace** rustls→kTLS bridge
  (compute the post-`KeyUpdate` traffic secrets via rustls's `kernel` /
  `KernelConnection` API, rustls ≥0.23.27, and drive the re-keying). Layer (1)
  is **confirmed present at the pinned v6.18 kernel** — verified against
  `Documentation/networking/tls.rst` at the **`v6.18` git tag**, which carries
  the "TLS 1.3 Key Updates" section: inbound `KeyUpdate` pauses decryption and
  reads return `EKEYEXPIRED` until userspace re-provides keys via
  `TLS_RX`/`TLS_TX`, with `TlsTxRekeyOk` / `TlsRxRekeyOk` /
  `TlsRxRekeyReceived` exposed via the MIB (also documented at
  [docs.kernel.org/networking/tls.html](https://docs.kernel.org/networking/tls.html);
  originated in Sabrina Dubroca's "tls: implement key updates for TLS1.3"
  series). Gap 4 in
  `sockops-mtls-ktls-installation-comprehensive-research.md` is therefore
  **RESOLVED for the kernel** — the prior "likely / unconfirmed" hedge no
  longer applies. (Hardware-offloaded `KeyUpdate` on ConnectX-6 Dx / mlx5
  — [LWN 1055522](https://lwn.net/Articles/1055522/) — is a separate, newer,
  NIC-specific series with a software fallback, irrelevant to Overdrive's
  software-kTLS / virtio path.) With layer (1) confirmed, **the userspace
  bridge is now the SOLE remaining blocker**: Overdrive's recommended bridge,
  the `ktls` crate (per
  `sockops-mtls-ktls-installation-comprehensive-research.md` Finding 5 /
  recommendations), **does not support `KeyUpdate` today** —
  [rustls/ktls#59](https://github.com/rustls/ktls/issues/59) ("Support new
  rustls `KernelConnection` API, and thence TLS1.3 KeyUpdates") is **open**,
  and the rewrite that would add it,
  [rustls/ktls#62](https://github.com/rustls/ktls/pull/62), is **open and
  unmerged** (under review; maintainers want it split, testing incomplete).
  Consequently **advancing the kernel pin does not unblock in-place rekey** —
  the kernel already supports it; the userspace half is the only thing
  missing. **v1 SVID rotation on long-lived connections is therefore
  teardown + reconnect** (drop the connection, re-handshake with the new
  SVID), which is acceptable for Overdrive's request-first east-west traffic
  (the application retries). The lever that would enable in-place rekey is the
  **userspace bridge landing `KeyUpdate` upstream** (adopt rustls's `kernel`
  API and land/vendor the ktls rewrite — rustls/ktls#59 / #62, tracked in
  [#229](https://github.com/overdrive-sh/overdrive/issues/229)), explicitly
  **not** a kernel-pin advance.

### Quality-attribute impact

- **Security — confidentiality/integrity**: positive. Guaranteed
  in-kernel TLS 1.3 TX+RX at the floor; no userspace-RX fallback surface
  to get wrong.
- **Maintainability — testability**: positive (large). The test surface
  is the one shipped kernel, not a 5-kernel average; the degraded-tier
  code path is deleted rather than tested.
- **Reliability — maturity**: mixed. Single-kernel focus deepens the
  signal on the kernel that ships (positive); cross-kernel regression
  breadth narrows to a nightly soft-fail lane (recorded negative).
- **Portability**: neutral-by-design. Overdrive is an appliance; it does
  not target portability across operator kernels, and this ADR makes that
  explicit rather than implied.

## Alternatives Considered

### A — Keep the 5.10 → current-LTS matrix unchanged

**Rejected.** It encodes a premise — "Overdrive must run on operator-supplied
kernels across a wide range" — that is false for an appliance OS that
builds and pins its own kernel (§23). The breadth is not free: it forces
a TX-only-kTLS / userspace-RX degraded tier for 5.10 / 5.15 (TLS 1.3 RX
needs ≥6.0) that the transparent-mTLS architecture would have to design,
ship, and test for kernels Overdrive never deploys. Cost with no matching
production benefit.

### B — Pin, but at an older floor (e.g. 6.1 LTS) for broader hardware reach

**Rejected.** 6.1 has TLS 1.3 RX (≥6.0) but **not** `CONFIG_NET_HANDSHAKE`
(≥6.5), foreclosing the `tlshd`-style handshake-upcall option the DESIGN
wave may want. Pinning means Overdrive picks the kernel; picking the
*latest* LTS maximises available kernel mechanisms (handshake upcall,
sockmap+kTLS maturity) at no operator-compatibility cost, since there is
no operator kernel to be compatible with. "Pin older to reach more
hardware" is the appliance-as-portable-software framing this ADR rejects.

### C — No pin; track "whatever LTS the base image ships," widen `bpf-next` to blocking

**Rejected.** Floating the floor reintroduces the diverse-kernel premise
through the back door (the floor drifts with the base image) and makes
`bpf-next` — an intentionally unstable early-warning target — a
merge-blocking gate, which would make CI hostage to upstream churn. The
pin is the point: Overdrive decides its kernel, deliberately and tested,
and `bpf-next` stays a soft-fail canary.

### D — Pin the latest *mainline* (e.g. 7.0 / 7.1), not an LTS

**Rejected.** This is the direct answer to "we control the kernel, so why
not pin the *newest* kernel rather than the newest *LTS*?" 7.0 (released
2026-04-12) and 7.1 are **mainline/stable, not longterm**: a non-LTS
release is maintained only until the next release ships (~2–3 months) and
then EOLs with **no security backports**. For an appliance OS Overdrive
must ship CVE fixes for, that turns the pin into a continuous forced-rebase
treadmill — every ~2–3 months the floor's upstream support evaporates and
Overdrive must jump to the next mainline or fall off the security cliff.
The cost compounds because **every kernel release re-rolls the eBPF
verifier**: each bump forces a full re-validation of the dataplane's
verifier-acceptance and complexity budgets (Tier-4 `verifier-regress`)
across every BPF program. An LTS gives a **stable verifier target** plus
multi-year backports (6.18 → Dec 2028), so the re-validation and rebase
work happens on Overdrive's deliberate schedule, not on mainline's ~2–3
month cadence. "Newest kernel" and "newest *supportable* kernel" are
different questions; the appliance wants the latter.

**Precedent.** The closest precedent that *also* controls and builds its
own kernel reaches the identical conclusion. Talos Linux is a minimal
immutable appliance OS that builds its kernel from kernel.org source (via
`siderolabs/pkgs`, Clang/ThinLTO) — the same self-build sourcing model as
Overdrive's Yocto kernel, not a distro base — and ships **LTS only**.
Maintainer `smira`, verbatim: *"Talos always ships LTS only versions, as
we need them to [be] updated for the lifecycle of Talos 1.9 ... which is
around 4-5 months"* — exactly this ADR's backports argument, since a
non-LTS branch EOLs well inside a multi-month OS support window while a
self-built kernel rides kernel.org's longterm maintainers. Talos ships
**6.18 LTS today** (v1.12 → 6.18.1, `pkgs` main → 6.18.34 — the same
kernel this ADR pins) and tracks the *latest* LTS advancing deliberately
(6.6 → 6.12 → 6.18), matching this ADR's "latest qualifying LTS, advanced
at rebuilds" principle. Flatcar (6.12/6.6), Bottlerocket (6.12/6.18), and
Yocto `linux-yocto` (kernel.org LTS only) corroborate: every own-kernel
appliance OS is on an LTS line. Controlling the kernel is an argument for
LTS, not against it.

## Propagation (lands with this ADR)

The kernel-matrix SSOT is updated in lockstep so no document contradicts
this decision:

- **`docs/whitepaper.md` §22** — the Tier-3 "Kernel matrix" table and the
  CI-topology `matrix:` line rewritten to "6.18 (pin) + bpf-next
  soft-fail."
- **`.claude/rules/testing.md` § "Kernel matrix"** — the 5-kernel list
  rewritten to the pinned 6.18 + bpf-next model; the "Dropping a kernel
  requires an ADR" sentence now points at this ADR.
- **`docs/product/architecture/c4-diagrams.md`** — the Linux-kernel
  external-system note "kernels 5.10+ supported" → "pinned 6.18 LTS
  appliance kernel."
- **ADR-0053** / **ADR-0059** — the "5.10 floor" back-references updated
  to cite ADR-0068 (the cited primitive-availability kernel versions are
  unchanged; only the floor moves).

## References

- `docs/research/dataplane/sockops-mtls-ktls-installation-comprehensive-research.md`
  — kTLS install mechanism; RX-vs-TX kernel-version split.
- `docs/research/dataplane/sockops-ktls-plaintext-race-window-research.md`
  — controlled-kernel implication (the out-of-tree write-block patch).
- `docs/research/dataplane/transparent-mtls-recommended-architecture-research.md`
  — decision-grade synthesis; pins the appliance kernel at an
  Overdrive-controlled LTS.
- `docs/research/platform/talos-kernel-versioning-strategy-research.md`
  — Talos Linux precedent (own-kernel appliance OS, LTS-only policy, ships
  6.18 LTS) and ecosystem corroboration (Flatcar, Bottlerocket, Yocto);
  validates Alternative D's rejection of pinning mainline.
- whitepaper §22 (Real-Kernel Integration Testing — kernel matrix), §23
  (Immutable, Minimal, Secure OS — Image Factory / kernel pin).
- `.claude/rules/testing.md` § "Tier 3 — Real-Kernel Integration" → "Kernel
  matrix" ("Dropping a kernel requires an ADR" — satisfied here).
- ADR-0053 (same-host backend delivery via `cgroup_sock_addr`) — floor
  back-reference updated.
- ADR-0059 (exec-probe cgroup placement) — floor back-reference updated.
- [#26](https://github.com/overdrive-sh/overdrive/issues/26) — transparent
  kernel mTLS (host-socket sockops + kTLS path).
- [#222](https://github.com/overdrive-sh/overdrive/issues/222) — guest-stack
  host L4 tap-proxy mTLS subsystem (MicroVM, unikernel).
