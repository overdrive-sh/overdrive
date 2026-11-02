# Landing workload diagram restoration review

## Scope and verdict

**APPROVED.** The four-line restoration returns all six original workload-category cards and removes the empty desktop rows and orphaned incoming connectors shown in the user's screenshot. Desktop and mobile browser captures show the intended existing layout.

Reviewed on 2026-11-02 against the user's explicit instruction that the categories should not have been removed, website conventions, and the applicable technical-writer requirements. The microVM-only execution correction did not authorize removal of these category cards. The earlier documentation review accepted that removal too broadly. This review assesses the requested restoration and does not reopen supported execution modes or prescribe a two-card redesign.

The sole product file is `website/app/(home)/page.tsx`. The reviewer writes only this review artifact and preserves user-dirty `AGENTS.md` and the writer's changes. No source, content, styling, policy, dependency, test, or DES edit was made by the reviewer. No commit, push, deployment, publication, native/Rust/model test, kernel, VM, Lima, Metal, or Cloudflare operation was performed.

## Exact diff and preserved behavior

Independent comparison confirms that `workloadKinds` matches its complete array before commit `7224b0f20`, with the original order and capitalization:

1. Services
2. Jobs
3. microVMs
4. Unikernels
5. WASM
6. Sandboxes

Removing only the four added array entries reproduces the current HEAD file byte-for-byte. Thus the rest of the page, including component logic, geometry, SVG paths, caption, logo, six platform actions, examples, and copy, is unchanged. No CSS file changed. Both existing array consumers receive the restored labels without new rendering logic. The retained icon map already contains every restored category.

This does not restore an Exec workload driver, operator syntax, or process-workload documentation. The surrounding microVM execution copy remains intact. No new authored prose or punctuation was introduced.

## Visual evidence

The reviewer inspected the actual images with `view_image`:

- Original regression: `.context/attachments/OX6gPj/Screenshot 2026-11-02 at 21.22.09.png`. It shows Services and Jobs occupying only the first two rows of a six-row board, with four incoming connectors ending in empty space.
- Corrected desktop: `.context/landing-workload-diagram-desktop.png`, captured from a 1440 by 1000 viewport. The board capture shows six populated left cards, six right action cards, and aligned incoming/outgoing connectors around the existing central logo. There are no empty workload rows, orphaned connectors, or overlapping boxes.
- Corrected mobile: `.context/landing-workload-diagram-mobile.png`, captured from a 390 by 844 viewport. The board capture shows all six categories in the existing three-column, two-row grid, the central logo with vertical group connections, and all six actions stacked below. The full board extends vertically through normal scrolling, with no horizontal clipping or overlap.

The source retains the six fixed paths in each SVG wire group. The retained browser readback confirms that desktop wires remain visible and mobile hides those desktop SVG wires under the existing responsive rule. The mobile caption detail is intentionally hidden by that rule, rather than missing from the product data. No layout or connector redesign is necessary.

## Validation

Read `.context/landing-workload-diagram.md` and independently inspected `.context/landing-workload-diagram-browser.json`. Assertions over the retained readbacks confirm the exact six labels, six workload cards, six action cards, six incoming paths, and six outgoing paths at both recorded viewports. Both readbacks report no horizontal document overflow, zero box overlaps, and all cards inside the board. The desktop geometry and inspected image support the writer's connector-alignment result.

The writer records passing current typecheck, lint, production Next build, and diff checks, with the existing index assertion passing for 28 published pages and six blog posts. Builds were not redundantly rerun for this four-line data restoration. Independent exact-source comparison and `git diff --check` pass.

The browser evidence comes from installed headless Chrome with a dedicated temporary profile, as recorded in the ledger. No dependency was added. The writer records closing the dedicated browser and stopping the local Next server. This reviewer inspected the captured browser evidence rather than claiming a second live browser run. Existing unrelated search/analytics limitations were not rerun or represented as resolved.

## Findings and disposition

No blocking finding remains. The visual regression is closed by restoring the original data, and the user-requested six-category design remains intact at the checked desktop and mobile viewports. No unrelated product-copy, execution-contract, CSS, or framework change is requested.

## Final verdict

**APPROVED — restore the six original landing workload categories.** The patch is exact, the screenshots show the requested visual result, and the current microVM-only execution documentation remains preserved. This review does not authorize publication or deployment.
