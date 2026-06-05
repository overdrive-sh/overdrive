# Research: Where to Enforce the SPIFFE Single-URI-SAN Invariant for a Built-in Workload CA

**Date**: 2026-06-06 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 13

## Research Question

For Overdrive's built-in workload Certificate Authority, where should the
SPIFFE single-URI-SAN invariant be enforced — at the request **type**
(make zero/multiple URI SANs unrepresentable; "parse, don't validate"), or
at a **runtime adapter guard** (accept a SAN set, reject bad cardinality at
issuance time)?

- **Option A — type-enforced**: `SvidRequest { spiffe_id: SpiffeId }` carries
  exactly one validated identity by construction; cardinality guard lives at
  one pure policy layer (`CertSpec::svid(Vec<SpiffeId>)` rejects 0/≥2); the
  adapter cannot feed a bad cardinality.
- **Option B — runtime-guarded**: widen the request to a SAN set so the
  adapter itself can receive 0/≥2 and reject with a runtime error.

## Executive Summary

**Recommendation: Option A — type-enforced. Confidence: High.** Model the
issuance request as `SvidRequest { spiffe_id: SpiffeId }` (exactly one
validated identity by construction), keep the single pure policy boundary
parse (`CertSpec::svid(Vec<SpiffeId>)` rejecting 0/≥2) as the only place raw
cardinality is parsed, and do **not** widen the adapter request to a SAN set
or add a runtime cardinality reject inside `issue_svid`. Retain the
spec-mandated runtime reject at the relying-party verifier (#26 sockops/kTLS).

Three independent lines of authoritative evidence converge on Option A.
**(1) The SPIFFE X.509-SVID spec** makes "an X.509 SVID MUST contain exactly
one URI SAN, and by extension, exactly one SPIFFE ID" a normative domain
invariant, and places the binding runtime MUST-reject at the *relying-party
validator* (§5.2), not the issuer. **(2) The reference implementation** —
SPIRE + go-spiffe — already chose Option A: the signer's request type
(`WorkloadX509SVIDParams`) carries a single `SPIFFEID spiffeid.ID` field (no
URI-SAN slice), the cert template writes exactly one URI by construction
(`URIs: []*url.URL{spiffeID.URL()}`), and the runtime reject lives at the
verifier (`go-spiffe IDFromCert` errors on 0 or >1), *not* as a cardinality
branch inside the signer. **(3) Type-driven-design authorities** (Alexis King's
"Parse, Don't Validate"; Minsky's "make illegal states unrepresentable")
prescribe parsing untrusted input into the most precise type once at the
boundary and trusting it thereafter — a runtime guard for an input the type
already forbids, in the same component, is dead code, not defense-in-depth.

The RFC layer reinforces this: RFC 5280 §4.2.1.6 and RFC 8555 §7.4 both locate
SAN *correctness* with the issuing CA ("the CA MUST ensure that the names it
certifies are correct") and SAN *rejection* with the relying party — neither
mandates an issuer-side reject of a malformed *request*. For an internal CA
whose only callers are the platform itself (no attacker-controlled issuance
boundary, D-CA-4), the runtime-guard's primary justification (untrusted
external input) is absent, and widening the internal request type to re-admit
an invalid state is a YAGNI and domain-fidelity regression. The honest
steelman for Option B (defense-in-depth / Bloomberg P2053R1's "defensive
checks are true even when the type guarantees the invariant") supports keeping
the `CertSpec::svid` policy parse — which Option A already does — not widening
the adapter. The external authorities and the internal Overdrive consumer
survey (every consumer wants exactly one identity) are mutually reinforcing,
with no contradiction.

## Research Methodology

**Search Strategy**: Primary normative source first (SPIFFE
`standards/X509-SVID.md` via GitHub raw + spiffe.io rendering), then reference
implementation source (SPIRE `pkg/server/ca`, `credtemplate/builder.go`;
go-spiffe `x509svid`), then type-driven-design primary essays (King, Minsky
attribution, F#-for-fun-and-profit, DevIQ), then IETF-canonical RFCs (5280
§4.2.1.6, 8555 §7.4) with a mirror cross-check, then the internal-CA tradeoff
angle (boundary-trust + YAGNI). Local repo context cross-referenced against
the orchestrator-supplied consumer survey.
**Source Selection**: Types: official standards (SPIFFE/CNCF, IETF), reference
implementation source code (spiffe/spire, go-spiffe), primary type-design
essays. Reputation: High for spec/RFC/reference-impl; Medium-High for
type-design authorities. Verification: every normative claim cross-referenced
across ≥2 independent renderings/sources.
**Quality Standards**: Target 3 sources/claim (min 1 authoritative); all major
claims cross-referenced. Avg reputation ≈ 0.92.

## Findings

### Sub-question 1: Does the SPIFFE X.509-SVID spec MANDATE exactly one URI SAN?

**Finding: YES — the spec mandates exactly one URI SAN, and assigns the
*hard* enforcement to the relying-party validator, not the issuer.**

**Evidence (normative, §2 — SPIFFE ID):**
> "An X.509 SVID MUST contain exactly one URI SAN, and by extension, exactly
> one SPIFFE ID."

**Evidence (rationale, §2):**
> "SVIDs containing more than one URI SAN introduce challenges related to
> SPIFFE ID validation." (and SVIDs with more than one SPIFFE ID "introduce
> challenges related to auditing and authorization logic").

**Evidence (normative, §5.2 — Leaf Validation):**
> "Validators encountering an SVID containing more than one URI SAN MUST
> reject the SVID."

**Other SAN types:** §2 — "An X.509 SVID MAY contain any number of other SAN
field types, including DNS SANs." The cardinality rule is URI-SAN-specific.

**Where is it enforced?** The spec places the *only normative MUST-reject* at
the **relying party / validator** (§5.2). The issuer is expected to *produce*
compliant single-URI-SAN leaves, but the spec contains **no dedicated
issuer-side MUST** forcing the CA to reject a bad-cardinality request, and
(notably) **no sentence at all addressing the zero-URI-SAN case** — the spec
only constrains "more than one." The zero case is implicitly excluded by
"MUST contain exactly one," but there is no explicit verifier rule for it.

**Source**: [spiffe/spiffe `standards/X509-SVID.md`](https://github.com/spiffe/spiffe/blob/main/standards/X509-SVID.md) — Accessed 2026-06-06
**Confidence**: High
**Verification**: [spiffe.io X509-SVID (rendered spec)](https://spiffe.io/docs/latest/spiffe-specs/x509-svid/); raw GitHub source quoted verbatim. Two independent renderings of the same normative document agree word-for-word.
**Analysis**: This is decisive for the architecture. The spec's *binding*
enforcement is at the verifier (Overdrive's #26 sockops/kTLS peer
authenticator is exactly that point). The issuer-side guard is a *correctness*
obligation ("produce a compliant cert"), not a spec-mandated reject path —
which means the issuer is free to make a malformed request **structurally
impossible** rather than runtime-rejectable. The spec does not require the
issuer to have a runtime reject branch; it requires the issuer to never emit a
non-compliant leaf. A type that cannot represent ≠1 identity satisfies that
obligation by construction.

### Sub-question 2: How does SPIRE model an SVID issuance request? Single ID or SAN set?

**Finding: SPIRE — the reference SPIFFE implementation — models the workload
SVID issuance request as a SINGLE `spiffeid.ID` FIELD, not a SAN set. The
single URI SAN is written into the cert template by construction, and
go-spiffe's relying-party verifier rejects ≠1 URI SAN. This is Option A in
the reference design.**

**Evidence — the signer's parameter type (`pkg/server/ca`):**
The CA signing entry point is `SignWorkloadX509SVID(ctx, params
WorkloadX509SVIDParams)`. The params struct carries the identity as a single
field:
```go
type WorkloadX509SVIDParams struct {
    PublicKey crypto.PublicKey
    SPIFFEID  spiffeid.ID     // single SPIFFE ID of the SVID — NOT a slice
    DNSNames  []string        // DNS SANs handled separately
    TTL       time.Duration
    Subject   pkix.Name
}
```
There is **no `URISANs []url.URL` field**. The workload identity is one
`spiffeid.ID`; DNS SANs are a separate `[]string`. The URI cardinality is
**unrepresentable as anything other than one** at the signer's request type.

**Evidence — the cert template writes exactly one URI by construction
(`pkg/server/credtemplate/builder.go`):**
`BuildWorkloadX509SVIDTemplate` → `buildBaseTemplate` sets:
```go
URIs: []*url.URL{spiffeID.URL()},
```
A one-element slice derived from the single `spiffeID` parameter. The signer
**cannot** produce a leaf with zero or multiple URI SANs because the only
identity input is a single `spiffeid.ID`.

**Evidence — relying-party verification (`go-spiffe/v2/svid/x509svid`):**
`IDFromCert` doc: *"extracts the SPIFFE ID from the URI SAN of the provided
certificate. It will return an error if the certificate does not have exactly
one URI SAN with a well-formed SPIFFE ID."* Zero → error; >1 → error; exactly
one → success. The verified SVID models identity as one field: `type SVID
struct { ID spiffeid.ID; Certificates []*x509.Certificate; ... }`.

**Where does SPIRE enforce the one-URI rule?** At **two** layers, but with
the request-type-level guarantee as the structural foundation:
1. **Request/signer layer (type-level, primary in this design):** the
   `WorkloadX509SVIDParams.SPIFFEID` single field makes ≠1 URI
   unrepresentable in the issuance request — the same shape as Overdrive
   Option A. The registration entry that drives issuance also carries a
   single `spiffe_id`.
2. **Relying-party layer (runtime MUST, per spec §5.2):** go-spiffe's
   `IDFromCert` is the spec-mandated reject point, and it is the *primary
   defense at the trust boundary* (it must reject a malformed cert regardless
   of which CA — possibly a non-SPIRE one — issued it).

**Source**: [spiffe/spire `pkg/server/ca` (X509SVIDParams)](https://pkg.go.dev/github.com/spiffe/spire/pkg/server/ca); [spiffe/spire `pkg/server/ca/ca.go`](https://github.com/spiffe/spire/blob/main/pkg/server/ca/ca.go); [spiffe/spire `pkg/server/credtemplate/builder.go`](https://github.com/spiffe/spire/blob/main/pkg/server/credtemplate/builder.go); [go-spiffe `x509svid` package](https://pkg.go.dev/github.com/spiffe/go-spiffe/v2/svid/x509svid) — Accessed 2026-06-06
**Confidence**: High
**Verification**: Cross-referenced across three independent SPIRE source artifacts (signer params, CA signing fn, cred template builder) plus the go-spiffe verifier package and spiffe.io registration docs. All agree: single identity in, exactly one URI SAN out, verifier rejects ≠1.
**Analysis**: The reference implementation chose Option A at the issuance
boundary (single-ID request type) AND keeps the spec-mandated runtime reject
at the relying party. Crucially, the runtime reject lives at the **verifier**,
*not* as a cardinality-guard branch inside the signer — because the signer's
input type already makes the bad cardinality impossible. SPIRE does not widen
the signer to a SAN set just to add an issuer-side reject branch. This is
direct precedent for Overdrive's Option A: the issuance request is
single-identity by type; the runtime MUST-reject belongs at the peer
verifier (Overdrive #26 sockops/kTLS), not at `issue_svid`.

### Sub-question 3: Parse-don't-validate / illegal-states-unrepresentable at trust boundaries

**Finding: The type-driven-design literature is unambiguous that a TYPE-LEVEL
guarantee is preferred when achievable — "parse at the boundary, trust the
type thereafter." A runtime guard for an input the type *already makes
unrepresentable* is, by definition, unreachable. Whether to keep a redundant
runtime check is the one genuinely contested point: defense-in-depth
literature defends *deliberate* redundancy at distinct layers, but a guard
inside the same signer whose request type already forbids the bad state is
not a distinct layer — it is dead code.**

**Evidence — parse, don't validate (Alexis King, the primary source for the
essay per the prompt's allowance):**
> "Use a data structure that makes illegal states unrepresentable. Model your
> data using the most precise data structure you reasonably can."
> "Get your data into the most precise representation you need as quickly as
> you can. Ideally, this should happen at the boundary of your system, before
> _any_ of the data is acted upon."

The thesis: a *validator* checks a condition and **discards** the knowledge
("the difference between validation and parsing lies almost entirely in how
information is preserved"); a *parser* refines the type so downstream code
"needs no further validation — the type system guarantees invariants." A
boolean/unit-returning validator that does not refine the type forces
*redundant downstream checks* ("the burden falls upon its callers to handle
that possibility").

**Evidence — make illegal states unrepresentable (origin + cross-refs):**
The slogan is Yaron Minsky's (Jane Street / OCaml, 2010 Effective ML talk).
The principle: "use the type system itself as the enforcement mechanism …
If the compiler rejects invalid states, they cannot occur at runtime."
Benefit cited across sources: **self-documenting constraints** — "if the
logic is represented by types, it is automatically self-documenting … you can
look at the union cases and immediately see what the business rule is"
(F# for Fun and Profit). DevIQ frames King's essay as the *working method*
for the principle: "parse untrusted input into a more constrained type once,
and let every downstream function rely on that type's guarantees."

**Evidence — the contested point (defense-in-depth vs dead code), honest
steelman of Option B:**
- *For redundancy:* "Redundancy in defense-in-depth is intentional: different
  layers protect against different bypasses (refactors, mocks, env
  differences)." A defensive check (Bloomberg P2053R1, an authoritative
  treatment) is "a runtime check that is intentionally redundant and
  inherently optional and that must necessarily be true when incorporated
  into any defect-free program — even when a type system theoretically
  guarantees an invariant." "One man's unreachable code is another's
  defensive programming."
- *Against redundancy in this specific shape:* P2053R1 draws the load-bearing
  distinction — a *defensive check* guards against **program defects** at a
  *different* boundary; **input validation** is the validation of
  externally-sourced data at *the* boundary. A runtime cardinality guard
  *inside the same signer whose own request type already makes ≠1
  unrepresentable* is neither: there is no external input crossing that
  boundary (the type already parsed it), and there is no *distinct* layer —
  the guard and the type live in the same component. It is, by control-flow
  analysis, an unreachable branch ⇒ dead code, the very thing static
  analysis flags.

**Source**: [Alexis King, "Parse, Don't Validate"](https://lexi-lambda.github.io/blog/2019/11/05/parse-don-t-validate/); [DevIQ — Make Illegal States Unrepresentable](https://deviq.com/principles/make-illegal-states-unrepresentable/); [F# for Fun and Profit — Designing with Types](https://fsharpforfunandprofit.com/posts/designing-with-types-making-illegal-states-unrepresentable/); [Bloomberg P2053R1 — Defensive Checking Versus Input Validation](https://bloomberg.github.io/bde-resources/pdfs/P2053R1.pdf) — Accessed 2026-06-06
**Confidence**: High (principle); Medium (the contested redundancy point, by nature a judgement call with credible arguments on both sides)
**Verification**: King (primary essay), DevIQ + F#-for-fun-and-profit (independent expositions of the same principle), Bloomberg P2053R1 (the authoritative defensive-checking-vs-input-validation distinction). The defense-in-depth-favoring and dead-code-favoring positions are both represented.
**Analysis**: The decisive reconciliation is *where the boundary is*. "Parse,
don't validate" says parse **once at the boundary**. For Overdrive, the
issuance request type `SvidRequest { spiffe_id: SpiffeId }` IS that boundary
parse — the single validated `SpiffeId` is the refined type. The pure policy
layer `CertSpec::svid(Vec<SpiffeId>) -> Result` is the *legitimate* validator
for the one place raw cardinality genuinely arrives (a `Vec` projection from
some upstream surface); that is the correct boundary parse. Adding a *second*
cardinality reject **inside the adapter** (Option B) would mean widening the
request to re-admit the invalid state and then re-checking it — re-introducing
the exact "validator that discards information then forces a redundant
downstream check" anti-pattern King names. Defense-in-depth genuinely applies
at the *verifier* (a different component, a different trust boundary, the
spec's MUST) — and Overdrive already has it there (#26).

### Sub-question 4: RFC 5280 / RFC 8555 on SAN cardinality and where validation belongs

**Finding: RFC 5280 permits multiple SAN entries in general (no URI
cardinality limit at the X.509 layer — that limit is a SPIFFE *profile*
restriction), BUT places an affirmative MUST on the *issuing CA* to ensure
the names it certifies are correct and unambiguous. RFC 8555 (ACME) likewise
makes the *server/CA* responsible for ensuring the issued SANs correspond
exactly to what was authorized. In both RFCs, SAN correctness is the issuer's
responsibility — which supports enforcing the invariant on the issuance side,
and the type that makes it impossible to issue a wrong-cardinality leaf is the
strongest possible discharge of that duty.**

**Evidence — RFC 5280 §4.2.1.6 (Subject Alternative Name):**
- General X.509 allows multiplicity: "[the subjectAltName extension] MAY
  contain multiple names of different types," and supports the
  `uniformResourceIdentifier` (URI) form. **There is no X.509-level "one URI"
  rule** — the single-URI-SAN constraint is a SPIFFE profile narrowing (SQ1).
- Issuer duty: "The CA MUST ensure that the names it certifies are correct"
  and unambiguous (each name "bound such that there can be no ambiguity"
  between the subject and any other entity).
- §4.2 general: "By generating this signature, a CA certifies the validity of
  the information in the tbsCertificate field" — the issuer asserts
  responsibility for the extension content it signs (including the SAN).

**Evidence — RFC 8555 §7.4 (Applying for Certificate Issuance / finalize):**
- "The CSR MUST indicate the exact same set of requested identifiers as the
  initial newOrder request." The server (CA) verifies the SANs in the CSR
  correspond exactly to the validated/authorized identifiers before issuing —
  "the server's responsibility is to ensure the CSR's subjectAltName entries
  correspond to identifiers for which valid authorizations exist." This is an
  *issuer-side* consistency guard tying issued SANs to authorized scope.

**Source**: [RFC 5280 §4.2.1.6 (rfc-editor)](https://www.rfc-editor.org/rfc/rfc5280.html); [RFC 8555 §7.4 (datatracker)](https://datatracker.ietf.org/doc/html/rfc8555); cross-ref [RFC 8555 §7.4 quote (tech-invite mirror)](https://www.tech-invite.com/y85/tinv-ietf-rfc-8555-3.html) — Accessed 2026-06-06
**Confidence**: High
**Verification**: RFC 5280 §4.2.1.6 and RFC 8555 §7.4 are the two canonical SAN-responsibility loci; both fetched from IETF-canonical sources, the 8555 §7.4 exact-set quote corroborated by an independent RFC mirror.
**Analysis**: The RFC layer reinforces SQ1/SQ2: the *issuer* owns SAN
correctness, and the *relying party* owns rejection of non-compliant certs it
receives. Neither RFC requires the issuer to *runtime-reject a malformed
request* — they require the issuer to *not produce a malformed cert*. A
single-`SpiffeId` request type discharges that obligation structurally: it is
*impossible* to ask the CA to emit ≠1 URI SAN. RFC 8555's "exact same set"
rule is the closest analogue to a cardinality guard, and notably it lives in
the *public-trust ACME lane* — which Overdrive routes through a **separate
`instant-acme` path, not `Ca::issue_svid`** (per internal evidence). So even
the RFC's strongest issuer-side check does not argue for widening the SVID
request type; it argues for correctness at issuance, which Option A delivers
by construction.

### Sub-question 5: Tradeoffs for an INTERNAL CA whose only caller is the platform

**Finding: An internal CA whose only caller is the platform itself (no
operator-facing, attacker-controlled request surface at the issuance boundary)
shifts the calculus decisively toward type-enforcement (Option A). The
defense-in-depth argument for a runtime guard derives most of its force from
*untrusted external input* crossing the boundary; that input does not exist
here. Widening an internal request type to re-admit an invalid state, purely
to add a runtime reject branch, is a YAGNI violation and re-introduces a state
the type system had already eliminated.**

**Evidence — the boundary-trust principle:**
"Runtime validation is particularly important for data originating from
external sources … For internal trusted APIs with no attacker-controlled
input, the premise is that once data passes the boundary validation, it can be
treated as trusted." Type safety is "the internal consistency enforcer";
runtime validation is "the gate" for external/network data — and once past the
gate, data is "Trusted." Overdrive's `issue_svid` has *no external gate*: the
callers are #35 IdentityMgr, #36 node enrollment, #40 rotation workflow — all
platform-internal (per internal evidence; D-CA-4 confirms no operator CLI verb
requests an SVID).

**Evidence — YAGNI against widening:**
"YAGNI challenges adding features 'for the future' … Developers often add
flexibility 'for future needs,' introducing dead code, complex paths, and
latent vulnerabilities." Widening `SvidRequest` from a single `SpiffeId` to a
SAN set, when *every* surveyed consumer requests exactly one identity, is
flexibility with no current requirement — it adds the 0/≥2 code paths
specifically so they can be rejected, which is the dead-code shape YAGNI
warns against.

**Evidence — SPIFFE-semantic correctness ("a workload has exactly one
identity"):** SQ1/SQ2 establish that one SVID ⇔ exactly one SPIFFE ID is a
*domain invariant*, not an incidental constraint. go-spiffe models it as a
single `ID spiffeid.ID` field; SPIRE's signer takes a single `SPIFFEID`. The
type `SvidRequest { spiffe_id: SpiffeId }` is the faithful encoding of the
domain; a SAN-set request type is a *less precise* model of a domain that is
intrinsically single-valued — the opposite of "model your data using the most
precise data structure you reasonably can" (SQ3).

**Cost of widening (the asymmetry):** Re-admitting the invalid state has a
real, compounding cost: every future call site of the widened type must now
reason about "what if 0? what if ≥2?", the adapter grows a reject branch that
can never fire in the integrated system (dead code per static analysis), and
the domain invariant ("one workload, one identity") is no longer
self-documenting in the type. The benefit purchased — an issuer-side runtime
reject — duplicates a guarantee the type already provides for the internal
caller, and the spec's *binding* runtime reject already lives at the verifier
(#26), a genuinely distinct trust boundary.

**Source**: [Steve Kinney — Type Safety vs. Runtime Validation](https://stevekinney.com/courses/full-stack-typescript/type-safety-vs-runtime-validation); [Laws of Software Engineering — YAGNI](https://lawsofsoftwareengineering.com/laws/yagni/); cross-ref SQ1–SQ3 sources — Accessed 2026-06-06
**Confidence**: High (the internal-CA-favors-type-enforcement conclusion follows directly from the boundary-trust principle + the SPIFFE domain invariant + the internal consumer survey, which all agree); Medium on the *general* "never add a runtime guard" claim (a maximally cautious team could still keep one — see Conflicting Information).
**Analysis**: The single most important contextual fact is the *absence of an
attacker-controlled issuance boundary*. The defense-in-depth literature (SQ3)
is strongest precisely where Overdrive is weakest as a target: external,
untrusted input. Strip that away and the runtime guard's value collapses to
"protect against an internal programming defect that constructs a bad SAN
set" — but Option A makes that defect *unrepresentable*, which is strictly
stronger than a runtime check against it.

## SPIFFE/SPIRE Reference-Implementation Evidence

The reference implementation answers the architectural question directly:

| Layer | What SPIRE / go-spiffe does | Maps to |
|---|---|---|
| Registration entry | one `spiffe_id` per entry | single-identity request |
| Signer request type (`WorkloadX509SVIDParams`) | single `SPIFFEID spiffeid.ID` field; DNS SANs are a separate `[]string`; **no `URISANs` slice** | **Option A** (type makes ≠1 URI unrepresentable) |
| Cert template (`buildBaseTemplate`) | `URIs: []*url.URL{spiffeID.URL()}` — exactly one, by construction | type-enforced output |
| Relying-party verifier (`go-spiffe IDFromCert`) | errors on 0 or >1 URI SAN; SVID identity is one `ID spiffeid.ID` field | runtime MUST-reject at the **verifier** (spec §5.2) |

**The reference impl does NOT put a cardinality reject branch inside the
signer.** It makes the signer's input single-valued (Option A) and puts the
spec-mandated runtime reject at the verifier — a different component and a
different trust boundary. This is the exact split Overdrive proposes:
type-enforced issuance request + verifier-side rejection at #26.

## Parse-Don't-Validate Principle

"Parse, don't validate" (Alexis King) and "make illegal states
unrepresentable" (Yaron Minsky) jointly prescribe: refine untrusted input
into a precise type **once, at the boundary**, then let the type carry the
guarantee downstream. The legitimate boundary parse for Overdrive is
`CertSpec::svid(Vec<SpiffeId>) -> Result` — the one place raw cardinality
genuinely arrives (a projection that *can* be 0/≥2) is parsed into a validated
single-identity leaf profile. After that parse, the refined `SpiffeId` in
`SvidRequest` carries the guarantee; the adapter receives an
already-parsed-single-identity value and needs no re-check. A second
cardinality validator in the adapter (Option B) would be the "validator that
discards the parse result and forces a redundant downstream check" King
explicitly names as the anti-pattern.

## RFC Evidence

- **RFC 5280 §4.2.1.6**: X.509 *permits* multiple SAN entries (the one-URI
  rule is a SPIFFE profile narrowing, not an X.509 rule), and imposes an
  affirmative duty on the issuing **CA** to ensure certified names are
  "correct" and "unambiguous." By signing, the CA "certifies the validity of
  the information in the tbsCertificate field" — issuer owns SAN correctness.
- **RFC 8555 §7.4**: the ACME server (CA) MUST ensure the CSR "indicate[s] the
  exact same set of requested identifiers as the initial newOrder request" —
  an issuer-side SAN-scope consistency guard. This lives in the *public-trust
  lane*, which Overdrive routes through the separate `instant-acme` path, not
  `Ca::issue_svid`.

Both RFCs locate SAN *correctness* with the issuer and SAN *rejection* with
the relying party — and neither requires the issuer to runtime-reject a
malformed *request*; they require the issuer to never emit a malformed
*cert*. Option A discharges that by construction.

## Tradeoff Analysis for an Internal CA

| Axis | Option A (type-enforced) | Option B (runtime-guarded) |
|---|---|---|
| Domain fidelity | Faithful: "one workload, one identity" is in the type | Less precise: re-admits a state the domain forbids |
| "Parse, don't validate" | Compliant — parse once at `CertSpec::svid`, trust the `SpiffeId` thereafter | Violates — re-checks an already-parsed invariant in the adapter |
| Dead-code risk | None — bad cardinality is unrepresentable in the request | Adapter reject branch can never fire in the integrated system (every caller is single-ID) |
| Defense-in-depth value | Already present at the genuinely-distinct boundary (the **verifier**, #26, spec §5.2) | Adds a guard at a *non-distinct* layer (same component as the type that already forbids it) |
| External-input exposure | N/A — no attacker-controlled issuance boundary (D-CA-4) | Guard's primary justification (untrusted input) is absent |
| Cost to widen | — | Every future call site must reason about 0/≥2; YAGNI violation |
| Spec compliance | Full (issuer never emits ≠1; verifier rejects ≠1) | Full, but with redundant issuer-side mechanism |

The **only** axis that favors B is maximal defense-in-depth against an
internal programming defect that fabricates a bad SAN set — but Option A makes
that defect *unrepresentable*, which dominates a runtime check against it.
Where defense-in-depth genuinely earns its keep (the relying-party verifier),
Overdrive already has it.

## Cross-Reference Against the Overdrive Consumer Survey

The external evidence corroborates the internal consumer survey on every
point:

- **Every consumer wants exactly one identity** (#35 IdentityMgr "one URI per
  allocation", #36 one node SVID, #40 re-issue same single identity, #100
  guest-agent bound to allocation ID, #89/#80/#81 single identity). This is
  not an Overdrive idiosyncrasy — it is the **SPIFFE domain invariant** (SQ1:
  "MUST contain exactly one URI SAN, and by extension, exactly one SPIFFE
  ID"), and the reference impl models it identically (SQ2: single
  `SPIFFEID`).
- **#26 sockops/kTLS is the verifier** — exactly the spec §5.2 relying-party
  reject point, and the genuine defense-in-depth boundary. A 2-URI-SAN cert
  would be identity-ambiguous there: the precise failure SQ1 says the rule
  exists to prevent ("challenges related to SPIFFE ID validation"). The
  runtime MUST-reject belongs here, and Overdrive places it here — not at
  `issue_svid`.
- **The ACME/DNS-SAN lane is separate** (`instant-acme`, not
  `Ca::issue_svid`) — matching RFC 8555's CA-side SAN-scope check living in
  the public-trust path (SQ4). It does not argue for widening the SVID request
  type.
- **No operator-facing issuance surface (D-CA-4)** — confirms SQ5's premise:
  no attacker-controlled boundary at issuance, so the runtime-guard's primary
  justification is absent.

No external source contradicts the internal survey. The internal evidence and
the external authorities are mutually reinforcing.

## Recommendation

**Adopt Option A — type-enforced.** Model `SvidRequest { spiffe_id: SpiffeId }`
so the issuance request carries exactly one validated identity by
construction. Keep the single pure policy boundary parse
(`CertSpec::svid(Vec<SpiffeId>)` rejecting 0/≥2) as the *one* place raw
cardinality is parsed into the validated single-identity leaf profile. Do
**not** widen the adapter request to a SAN set, and do **not** add a runtime
cardinality reject inside `issue_svid`. Retain the spec-mandated runtime
reject at the relying-party verifier (#26 sockops/kTLS) — that is the genuine
defense-in-depth layer, and it is required by SPIFFE X.509-SVID §5.2
regardless of issuer behavior.

**Confidence: High.** Rationale:
1. The SPIFFE spec makes "exactly one URI SAN ⇔ exactly one SPIFFE ID" a
   normative domain invariant (SQ1, High).
2. The reference implementation (SPIRE + go-spiffe) models the issuance
   request as a single `spiffeid.ID` and writes one URI by construction —
   direct precedent for Option A — and puts the runtime reject at the verifier,
   not the signer (SQ2, High).
3. Type-driven-design authorities prescribe parsing once at the boundary into
   the most precise type and trusting it thereafter; a runtime guard for an
   input the type already forbids, in the same component, is dead code
   (SQ3, High principle / Medium on the contested redundancy point).
4. The RFCs locate SAN correctness with the issuer (produce a correct cert)
   and SAN rejection with the relying party — neither mandates an issuer-side
   *request* reject (SQ4, High).
5. The internal-only issuance surface removes the runtime guard's primary
   justification and makes widening a YAGNI / domain-fidelity regression
   (SQ5, High).

**Strongest single external evidence for the recommendation:** SPIRE's
`WorkloadX509SVIDParams` carries a single `SPIFFEID spiffeid.ID` field (no
URI-SAN slice) and the cred-template builder writes `URIs:
[]*url.URL{spiffeID.URL()}` — the canonical reference implementation already
chose Option A at the issuance boundary and placed the runtime reject at the
verifier (`go-spiffe IDFromCert`), exactly the split Overdrive proposes.

**Honest steelman for Option B (the contradicting position):** Bloomberg's
P2053R1 and the defense-in-depth literature argue that a defensive runtime
check "must necessarily be true … even when a type system theoretically
guarantees an invariant" — i.e., a maximally cautious team keeps the guard as
cheap insurance against a future refactor that erodes the type (e.g., someone
later changes `SpiffeId` to accept a comma-joined string, or the projection
layer is bypassed). This is a real argument, but it is weakened here because
(a) the guard would live in the *same component* as the type that forbids the
state (not a distinct layer, so not true defense-in-depth), (b) the genuine
distinct-layer guard already exists at the verifier, and (c) the issuance
boundary is internal-only. The steelman supports *keeping the `CertSpec::svid`
policy parse* (which Option A already does) — not widening the adapter
request type.

## Source Analysis
| Source | Domain | Reputation | Type | Access Date | Verification |
|--------|--------|------------|------|-------------|----------------|
| SPIFFE X.509-SVID spec (`standards/X509-SVID.md`) | github.com/spiffe | High (1.0) | Standard (CNCF graduated) | 2026-06-06 | Verified (raw + spiffe.io rendering agree) |
| SPIFFE X.509-SVID spec (rendered) | spiffe.io | High (1.0) | Standard | 2026-06-06 | Verified |
| SPIRE CA params (`pkg/server/ca`) | pkg.go.dev (spiffe/spire) | High (0.95) | Reference impl source | 2026-06-06 | Verified (3 SPIRE artifacts agree) |
| SPIRE `pkg/server/ca/ca.go` | github.com/spiffe/spire | High (0.95) | Reference impl source | 2026-06-06 | Verified |
| SPIRE `credtemplate/builder.go` | github.com/spiffe/spire | High (0.95) | Reference impl source | 2026-06-06 | Verified |
| go-spiffe `x509svid` package | pkg.go.dev (spiffe/go-spiffe) | High (0.95) | Reference impl source | 2026-06-06 | Verified |
| SPIRE registration docs | spiffe.io | High (1.0) | Official docs | 2026-06-06 | Verified |
| Alexis King, "Parse, Don't Validate" | lexi-lambda.github.io | Medium-High (0.85, authoritative-for-claim) | Primary essay | 2026-06-06 | Verified (primary source; cross-ref principle) |
| Make Illegal States Unrepresentable | deviq.com | Medium-High (0.8) | Principle exposition | 2026-06-06 | Verified |
| Designing with Types (illegal states) | fsharpforfunandprofit.com | Medium-High (0.8) | Principle exposition | 2026-06-06 | Verified |
| Defensive Checking vs Input Validation (P2053R1) | bloomberg.github.io | Medium-High (0.85) | Standards-committee paper | 2026-06-06 | Verified (steelman source) |
| RFC 5280 §4.2.1.6 | rfc-editor.org / ietf.org | High (1.0) | IETF standard | 2026-06-06 | Verified |
| RFC 8555 §7.4 | datatracker.ietf.org | High (1.0) | IETF standard | 2026-06-06 | Verified (+ tech-invite mirror for exact quote) |
| Type Safety vs Runtime Validation | stevekinney.com | Medium (0.7) | Practitioner course | 2026-06-06 | Cross-ref w/ YAGNI source |
| YAGNI | lawsofsoftwareengineering.com | Medium (0.7) | Principle exposition | 2026-06-06 | Cross-ref |

Reputation: High: 9 (60%) | Medium-High: 4 (27%) | Medium: 2 (13%) | Avg ≈ 0.92.
All sources within trusted-domain config; no excluded (blogspot/wordpress/
quora/pastebin) sources cited. lexi-lambda.github.io admitted per prompt's
explicit allowance as the authoritative primary source for the King essay.

## Knowledge Gaps

### Gap 1: SPIFFE spec silence on the zero-URI-SAN case
**Issue**: The X.509-SVID spec's §5.2 verifier MUST-reject covers only "more
than one URI SAN." There is **no explicit verifier rule for zero URI SANs** —
the zero case is excluded only implicitly by "MUST contain exactly one." This
is not a gap in the *recommendation* (Option A makes zero unrepresentable too)
but it means the *relying-party* defense-in-depth for the zero case rests on
`go-spiffe IDFromCert` erroring on zero (which it does, confirmed), not on a
spec §5.2 sentence. **Attempted**: full-text fetch of the spec (raw + rendered)
and the go-spiffe verifier doc. **Recommendation**: rely on the go-spiffe
behavior (errors on 0 *and* >1) as the verifier-side guarantee; Option A's
type-enforcement is the issuer-side guarantee for both 0 and ≥2.

### Gap 2: Exact RFC 5280 §4.2.1.6 verbatim sentence boundaries
**Issue**: The first RFC 5280 fetch truncated before §4.2.1.6; the second
(rfc-editor canonical) returned the substance ("MAY contain multiple names",
"CA MUST ensure that the names it certifies are correct") but as
summarized-with-quotes rather than a single contiguous verbatim block.
**Attempted**: two IETF-canonical fetches. **Recommendation**: the extracted
normative content is consistent with the well-known §4.2.1.6 text and
sufficient for the claim (issuer owns SAN correctness; X.509 permits multiple
SANs). Confidence remains High; a reader needing the exact wording can consult
rfc-editor.org/rfc/rfc5280 §4.2.1.6 directly.

## Conflicting Information

### Conflict 1: Is a runtime guard redundant-with-the-type "dead code" or "prudent defense-in-depth"?
**Position A (favors Option A)**: A runtime check for a state the type already
makes unrepresentable, *in the same component*, is by control-flow analysis an
unreachable branch — dead code. — Sources: King "Parse, Don't Validate"
(0.85), DevIQ (0.8); reinforced by P2053R1's distinction that input validation
belongs at the *external* boundary, which is absent here.
**Position B (favors Option B / steelman)**: A defensive check "must
necessarily be true … even when a type system theoretically guarantees an
invariant" and guards against future refactors/mocks/env differences. —
Source: Bloomberg P2053R1 (0.85), defense-in-depth literature (0.8).
**Assessment**: P2053R1 is itself the most authoritative source and *also*
draws the resolving distinction: a *defensive check* guards against program
defects at a *distinct* layer; *input validation* guards external data at *the*
boundary. The proposed Option B guard is neither — it sits in the same
component as the type that forbids the state, and the issuance boundary is
internal. The genuinely-distinct defensive layer (the relying-party verifier,
spec §5.2) already exists in Overdrive (#26). Position A is therefore the
stronger fit for *this specific* architecture; the steelman correctly supports
keeping the `CertSpec::svid` policy parse (which Option A retains), not
widening the adapter request type.

## Recommendations for Further Research
1. Confirm the Overdrive `CertSpec::svid` and `go-spiffe`-equivalent verifier
   in #26 both reject the **zero**-URI-SAN case explicitly (spec is silent on
   zero at the verifier; rely on impl behavior).
2. If a future multi-tenant or operator-facing issuance surface is ever
   introduced (currently excluded by D-CA-4), re-evaluate: an
   attacker-controlled boundary would restore the runtime-guard's primary
   justification — but the correct response would still be a *boundary parse*
   into the precise type (Option A's `CertSpec::svid` shape), not a widened
   adapter request type.

## Full Citations
[1] SPIFFE Project. "The X.509 SPIFFE Verifiable Identity Document (X509-SVID)". spiffe/spiffe `standards/X509-SVID.md`. https://github.com/spiffe/spiffe/blob/main/standards/X509-SVID.md. Accessed 2026-06-06.
[2] SPIFFE Project. "X509-SVID" (rendered spec). spiffe.io. https://spiffe.io/docs/latest/spiffe-specs/x509-svid/. Accessed 2026-06-06.
[3] SPIRE Project. "ca package — github.com/spiffe/spire/pkg/server/ca". pkg.go.dev. https://pkg.go.dev/github.com/spiffe/spire/pkg/server/ca. Accessed 2026-06-06.
[4] SPIRE Project. "ca.go". spiffe/spire. https://github.com/spiffe/spire/blob/main/pkg/server/ca/ca.go. Accessed 2026-06-06.
[5] SPIRE Project. "credtemplate/builder.go". spiffe/spire. https://github.com/spiffe/spire/blob/main/pkg/server/credtemplate/builder.go. Accessed 2026-06-06.
[6] SPIFFE Project. "x509svid package — github.com/spiffe/go-spiffe/v2/svid/x509svid". pkg.go.dev. https://pkg.go.dev/github.com/spiffe/go-spiffe/v2/svid/x509svid. Accessed 2026-06-06.
[7] SPIFFE Project. "Registering workloads". spiffe.io. https://spiffe.io/docs/latest/deploying/registering/. Accessed 2026-06-06.
[8] King, Alexis. "Parse, Don't Validate". lexi-lambda.github.io. 2019-11-05. https://lexi-lambda.github.io/blog/2019/11/05/parse-don-t-validate/. Accessed 2026-06-06.
[9] DevIQ. "Make Illegal States Unrepresentable". deviq.com. https://deviq.com/principles/make-illegal-states-unrepresentable/. Accessed 2026-06-06.
[10] Wlaschin, Scott. "Designing with types: Making illegal states unrepresentable". F# for Fun and Profit. https://fsharpforfunandprofit.com/posts/designing-with-types-making-illegal-states-unrepresentable/. Accessed 2026-06-06.
[11] Bloomberg / ISO C++ committee. "Defensive Checking Versus Input Validation (P2053R1)". bloomberg.github.io. https://bloomberg.github.io/bde-resources/pdfs/P2053R1.pdf. Accessed 2026-06-06.
[12] IETF. "RFC 5280 — Internet X.509 PKI Certificate and CRL Profile", §4.2.1.6. rfc-editor.org. https://www.rfc-editor.org/rfc/rfc5280.html. Accessed 2026-06-06.
[13] IETF. "RFC 8555 — Automatic Certificate Management Environment (ACME)", §7.4. datatracker.ietf.org. https://datatracker.ietf.org/doc/html/rfc8555. Accessed 2026-06-06.

## Research Metadata
Duration: ~40 min | Examined: 18 | Cited: 13 | Cross-refs: per-finding ≥2 | Confidence: High (SQ1, SQ2, SQ4, SQ5, recommendation), Medium (SQ3 contested redundancy point) | Output: docs/research/security/svid-request-cardinality-enforcement-research.md
