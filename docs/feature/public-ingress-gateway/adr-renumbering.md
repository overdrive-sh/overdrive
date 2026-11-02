# Gateway ADR numbering — final map (2026-11-02)
Final surviving gateway ADRs: 0220–0243 (24 files in docs/product/architecture/). Main's own ADRs are ≤0219 and keep their meaning
(note main ADR-0114..0143 are UNRELATED #295 etc. decisions).

| Branch (B:, 0114–0143) | Intermediate (pass 1–5, 0220–0250) | FINAL |
|---|---|---|
| 0114 embed in serve | 0220 | 0220 |
| 0115 singleton Route Set | 0221 | 0221 |
| 0116 route+application owner | 0222 | 0222 |
| 0117 producer-neutral custody | 0223 | 0223 |
| 0118 cgroup BPF selection | 0224 | 0224 |
| 0119 gateway SVID lifecycle | 0225 | 0225 |
| 0120 TLS1.3 only | 0226 | 0226 |
| 0121 redacted status | 0227 | 0227 |
| 0122 custody domain boundary | 0228 | merged → 0223 |
| 0123 identity slot | 0229 | merged → 0225 |
| 0124 receipt ownership | 0230 | 0228 |
| 0125 demand lifecycle | RETIRED-0125 | retired (D2) → replaced by 0242 |
| 0126 state-based domain ownership | 0231 | retired → content in 0221 |
| 0127 acquisition vs consumption | 0232 | merged → 0223 |
| 0128 existing cgroup BPF dataplane / packet entry | 0233 | merged → 0224 (TEACH half retired, D1) |
| 0129 independent generations | 0234 | 0229 |
| 0130 public TLS vs gateway mTLS | 0235 | merged → 0234 |
| 0131 listener gating | 0236 | 0230 |
| 0132 preserve last usable key | 0237 | 0231 |
| 0133 demand-gated TEACH | RETIRED-0133 | retired (D1) |
| 0134 selected-backend receipt | 0238 | 0232 |
| 0135 commit-gated identity publication | 0239 | 0233 |
| 0136 exact-peer mTLS | 0240 | 0234 |
| 0137 HTTP/1.1 only | 0241 | 0235 |
| 0138 canonical route match key | 0242 | 0236 |
| 0139 proxy header policy | 0243 | 0237 |
| 0140 fixed finite limits | 0244 | 0238 |
| 0141 streamed bodies | 0245 | 0239 |
| 0142 single upstream attempt | 0246 | 0240 |
| 0143 applied backend identity | 0247 | 0241 |
| (new) D2 typed unavailability | 0248 | 0242 |
| (new) D5-d socket preparation | 0249 | 0243 |
| (new) shutdown order | 0250 | deleted (content in feature-delta D5 (c)) |
| (new, G1 2026-11-02) gateway legs use kernel TLS after a userspace rustls handshake | — | 0244 |
| (new, G1 2026-11-02) gateway parses HTTP/1.1 heads and owns message framing | — | 0245 |
| (new, G1 2026-11-02) splice out of kTLS only into an empty private pipe >= 64 KiB | — | 0246 |

Main's own ADRs end at 0219 (`origin/main`, 2026-11-02); 0244-0246 are the next free numbers. ADR-0235, 0237 and 0239 keep their numbers (decision text current; 0239's filename keeps its original slug); 0226, 0234 and 0238 keep their decisions.
