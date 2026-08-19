# UI Audit Report — Findings Ledger (living document)

> **Purpose:** single durable ledger for the Refact GUI UI audit (Storybook isolation sweep + live-app findings).
> **How to update:** append new findings with the next sequential `N-###` id; when a finding is fixed, change its Status to `fixed@<commit>` — do not delete rows. Measurements stay with the finding.
> **Created:** 2026-08-19, audit session 3 (post issue-#26 fix mission, post squash-merge `409701095e`).
> **Audit rig:** Storybook 7.6.20 dev server (`cd refact-agent/gui && npm run storybook`, session used :6007) · viewport 1280×900 @2× DPR · 8px/32px gridline overlays + red outlines on interactives · alignment detector (row-center drift >2.5px, stacked left-edge drift, uneven gaps >3px, svg excluded) · size census (control heights vs {20,24,26,28,30,36}, fonts vs {12,13,14,15,19}, radii vs {4,6,8,10,999}, paddings vs {2,4,6,8,12,16,22,32}) · forced-open overlays · 360px sheet-branch checks · reduced-motion computed-style probes.

**Sweep progress: 45/171 stories at full rigor.** Covered: all Design System galleries, complete UI kit (incl. overlays, ModelSelector suite, tables, virtualization, reduced-motion suite), Accordion, Callout, ErrorCallout. Remaining: ~126 chat-land + component stories (queue in Part 5).

**Audit-infra fixes landed this session:** N-19 (MSW noise bypass was port-hardcoded) and the Callout slice of N-06 (hooks-barrel import cascade). See Part 6.

---

## Part 1 — Carried unsolved ledger (issue-#26 audit, sessions 1–2)

Verified still-unfixed against merged `main` (`409701095e`) unless noted.

| ID | Sev | Finding | Evidence / location |
|---|---|---|---|
| L-01 | S2 | Workspace shell paints nothing at 420px width | GroupSplitView/SurfacePane; live-app repro at 420×900 |
| L-02 | S3 | Button heights outside {26,28,30,36}: chat-links pills ~17, Settings nav 36.75, worktree action buttons 28.75, composer "Set a goal" row (**re-measured session 4: exactly 34.0px** at `chat--configuration`, `_header_30xgb_5`) | live measurements; still open post integer type scale |
| L-03 | S3 | Radix 12/14px type leaks (ToolCard toggle wrapper, user-bubble root, ThreadInfo labels) | live measurements |
| L-04 | S3 | Odd type sizes 13/15/19 × `--rf-line` 1.5 → half-pixel line boxes (19.5/22.5/28.5); needs paired `--rf-line-N` px tokens | fresh evidence: kit Sheet h=342.5, DataTable row pitch 23/22 alternating (N-21) |
| L-05 | S3 | Glass recipe divergence + light-theme elevation collapse (overlay ≡ glass ≡ rgba(250,250,251,.92)) + intermittent black-glass compositing ghost | tokens.css light block |
| L-06 | S3 | Unowned light-theme label color rgba(0,0,0,.608) (AAA fail); full contrast sweep still owed (contrast_audit failed closed twice) | Providers labels |
| L-07 | S3 | Trajectory tab strip +5px off-grid from content top | Trajectory popover |
| L-08 | S3 | `@radix-ui/react-icons` remnants beyond TrajectoryButton | grep `@radix-ui/react-icons` |
| L-09 | S3 | Composer hand-rolled 520→260 container-query breakpoint ladder | ChatForm.module.css:331-375 |
| L-10 | S3 | TextArea consumer-dependent font size (composer vs RetryForm) | TextArea.module.css consumers — **session 4:** both now compute **14px** (composer `4px 12px` pad / RetryForm `12px` pad), so the FONT split appears resolved by the integer type scale; the PADDING still differs per consumer because TextArea ships none by design. |
| L-11 | S3 | ErrorCallout store-coupled in `components/` (useAppSelector in generic component) | Callout.tsx:111 |
| L-12 | S3 | Callout dead-prop API: `color`/`hex`/`mx`/`mt`/`mb`/`size` accepted and discarded; live callers still pass them | Callout.tsx:20-26,36-42,116,147,173 |
| L-13 | S4 | Worktree panel three alignment grids (13/17/21px edge insets) | Worktrees popover |
| L-14 | S4 | Mode list 3 row species (111/95.5/48px) | ModeSelect list |
| L-15 | S4 | Scheduler group labels split (now 12 vs 13 after integer scale) | Scheduler form |
| L-16 | S4 | Trajectory checkbox rows non-uniform (31/33.3/31 pre-merge; re-measure) | Trajectory popover |
| L-17 | S4 | RetryForm mixes 26px and 30px control rows | RetryForm — **RE-MEASURED session 4** at `chat-form-retryform--text-only`: Cancel **76.3x26 @13px**, gpt-4o **93.1x30 @14px**, Add image **100.5x26 @13px**, Submit **78.8x26 @13px**. Post-integer-scale the split is 26/13 vs 30/14 (was 26/12.5 vs 30/13.5). In its favour: `cySpread = 0` (all four centred at cy=154), radii all 8, weights all 650 — the row is perfectly aligned, only the model trigger is a size/type outlier. |
| L-18 | S4 | Home CHATS overflow rows unreachable, no scroll affordance | Dashboard ChatsSection |
| L-19 | S4 | Login H2 28px/700 off-scale; mixed copy sizes on one screen | LoginPage (story exists: `login--primary`) |
| L-20 | S4 | Reselect "input selector returned a different result" warning in ChatContent selector path | transcript stories console |
| L-21 | infra | ChatStoryHarness hardcodes `appearance: "dark"` — Storybook light toolbar never reaches harness stories | src/__stories__/ChatStoryHarness.tsx:21 |
| L-22 | infra | `.storybook/preview.tsx` uses raw Radix Theme; no ThemePropsContext → portal/host propagation untested in stories | preview.tsx:31 |
| L-23 | infra | Deprecated `@storybook/testing-library` retained (repo eslint rule `storybook/use-storybook-testing-library` forces it; migration = lint-config decision) | package.json:103 |
| L-24 | S4 | ThreadInfoButton 280px min-width Flex inside popover → clips as Sheet <304px viewport | ThreadInfoButton.tsx:186-194 |
| L-25 | gap | DialogImage lightbox: zero story coverage (last unmeasured surface) | no story file |
| L-26 | infra | ChatContent legacy stories use local MockedStore instead of shared harness; stale-fixture suspicion | ChatContent.stories.tsx:41-101 |
| L-27 | S4 | TerminalPanel uses `--rf-overlay-popover-min` (a MIN token) as `max-width` | TerminalPanel.module.css:72 |

---

## Part 2 — New findings (audit session 3, Storybook sweep)

| ID | Sev | Status | Finding + measurements |
|---|---|---|---|
| N-01 | **S2** | **fixed@40c012087** | **Button gallery "Variants × sizes" matrix SCRAMBLED.** DOM order = 4 headers → 5 variant labels → 15 buttons flowed row-major into a 4-col grid: labels Ghost/Soft/Primary/Danger fill row 2 across the sm/md/lg header columns, "Plain" wraps to row 3 col 1, all buttons land offset (buttons under "Variant" column, rows mixing variants). The canonical gallery lies about every variant×size cell. Fix: interleave `label,btn,btn,btn` per row or explicit grid placement. `ui-button--variants-sizes-states` |
| N-02 | **S2** | **fixed@40c012087** | **Icon gallery scrambled identically** — tone labels muted/faint/accent/warning/danger occupy the sm/md/lg columns; trailing icon rows unlabeled. Same signature ⇒ shared matrix story-helper bug; one fix heals both. `ui-icon--sizes-and-tones` |
| N-03 | **S2** | **fixed@40c012087** | **Light-pair helper split.** Broken no-background token-flipping wrapper (light half illegible on dark canvas): Button, Icon, Field controls, Field settings-page ×2 (100% illegible — zero coverage value), Switch, Slider, Select, Combobox, SegmentedControl, Tabs, VirtualList, EditableTable, overlay trigger cards. CORRECT painting helper exists and is used by: ToolCard, ModelSelector, DataTable light-dark stories. Fix: converge every gallery on the painting helper. **MECHANISM PROVEN session 4** at `ui-overlays-dialog--light-dark` (computed colours, not eyeballing): the "light" `_panel_1psvz_11` keeps **`color-scheme: dark`** and never gets a light page background — it sits on `rgb(12,13,15)` with only a `rgba(0,0,0,0.024)` wash. Token flipping is PARTIAL: description resolves to `rgba(0,0,0,0.55)` and the button label to `rgba(0,0,0,0.88)` (light tokens) over that near-black surface = **~1:1 contrast, literally invisible**, while the heading stays `rgba(255,255,255,0.92)` (dark token) and therefore still reads. So the wrapper flips SOME text/surface tokens and not others, and never flips the canvas. Fix must set the light canvas background + `color-scheme: light` on the wrapper, not just override token values. Side-observation in the same story: both trigger buttons render **640 x 65.5px** (stretched by the story wrapper, 30px off every rung) — same story-harness family as N-27. **SHARED-WRAPPER PROOF:** the cross-component consistency fingerprint of `ui-overlays-dialog--light-dark` and `ui-overlays-popover--light-dark` is **byte-for-byte identical** (ctrlH [65.5x2] / radius [8,10] / font [19px-700, 13px-400, 13px-500] / the same six backgrounds incl. both light `rgba(0,0,0,0.024|0.05)` and dark `rgba(255,255,255,0.035|0.043)` / the same four borders / gap [12,4,22] / pad [22,16,`0 12`]). Identical fingerprints across two different components = ONE shared harness (`_panel_1psvz_11`) is the single fix point for the whole light-half family. | **Session-4 third reproduction + mechanism proof:** `ui-field--settings-page` (a DARK story, page bg `rgb(12,13,15)`, `color-scheme: dark`) renders a panel `_panel_163vl_11` whose census mixes LIGHT tokens (`rgba(0,0,0,0.024)` surface, `rgba(0,0,0,0.12)` border, light selected pair `rgba(0,106,220,0.1)`/`rgba(0,106,220,0.7)`) with DARK accent `rgb(127,147,216)`. Measured: input border `rgba(0,0,0,0.12)` over panel `rgba(0,0,0,0.024)` composites to pure black with **contrast ratio 1.00** (WCAG UI minimum 3.0) — the field border is literally invisible. Root cause is now clear from `styles/tokens.css`: the periwinkle palette (l.19-28, `#7f93d8`) and the Radix-blue palette (l.227-232, `var(--accent-9, #006adc)`) are the dark/light variants of the SAME token names, so any surface that resolves the wrong scope silently swaps its whole colour system.
| N-04 | **S2** | **fixed@40c012087** | **Portaled overlays escape the light wrapper** — light-dark Menu story renders BOTH menus as dark glass (portal mounts at body under dark preview Theme). No overlay primitive has any light-mode coverage. Fix: per-story appearance global (render story twice via toolbar/global) instead of side-by-side wrappers, or portal-aware theme wrapper. | **Session-4 measurement:** at `ui-overlays-menu--light-dark` the two Menu overlays are byte-identical — both `386x189`, `padding: 12`, `radius: 10`, `rgba(28,28,31,0.94)` + `blur(14px)`, shadow `0 14px 40px` — confirming zero light-overlay coverage. The wrapper panels are also broken independently of the portal: the "light" panel computes `background: rgba(0,0,0,0.024)` (light token) while keeping `color-scheme: dark` and `color: rgba(255,255,255,0.92)`, so both halves render dark. The same `_1psvz_` story-helper module also ships **`640 x 65.5` trigger buttons** (off every rung of 26/30/36), so the light-pair helper is low-quality throughout. Menu items themselves are clean: all 8 at exactly 30px.
| N-05 | **S2** | **fixed@40c012087** | **`ui-virtuallist--large-list` renders blank.** DOM holds "1000 memories" + exactly 1 item + footer; painted output is empty (virtuoso container height collapse). Sibling VirtualizedGrid sizes correctly ⇒ story/container-height fix. Also `ui-virtuallist--light-dark` is in the broken light family. |
| N-06 | **S2** | partial fix | **Story import cascade via barrels.** `callout--default` load fetches ~100+ CSS modules app-wide (Buddy panels, Workspace terminal, Integrations, every tool card) because `Callout.tsx` imported the `../../hooks` barrel (≈70 hook re-exports → services/app/features). Slows loads enough to break automation waits; floods MSW warnings; likely the historical cold-start dual-React culprit. **Fixed for Callout this session (leaf import).** Chat-land stories inherently import the app; remaining work: audit other `components/*` for barrel imports. |
| N-07 | **S2/S3** | **fixed@40c012087** | **kit Dialog demo scroll defects** (opened via click): scroll owner is the WHOLE dialog — native chunky scrollbar runs through the title row, title/description scroll away; **Close button (30px) sits INSIDE the scroll area** (below fold at scroll-top) — the S2-9 "unreachable footer" disease in the kit's own flagship. Geometry is perfect (342px=340+borders, radius 10, overlay bg rgba(28,28,31,.94), centered dx=dy=0). Sheet does it right (Close pinned outside scroll) — copy Sheet's structure. |
| N-08 | **S3** | **fixed@40c012087** | **Tabs strip overhangs its alignment column by exactly 10px.** Title/desc/panel span x 61→701; tablist spans 61→711 (its 4px padding + 1px border per side uncompensated); last tab right edge 706. Tabs are fractional 213.33px wide → sub-pixel indicator positions. Fix: −5px side margins on the list or matching panel padding. `ui-tabs--states` **RE-VERIFIED session 4 (fresh rig, 1280x900@2x, DevTools-style layout overlay):** tablist border-box **61 -> 711 (650)** vs tabpanel **61 -> 701 (640)** — left edges agree, right edges differ by exactly **10px = 2x(4px list padding + 1px border)**. Reproduced on the second Tabs group in the same story (list 290 vs tabs 280). Everything else in the component is exact: tabs **213.3x30** with gaps **0,0** (equal widths), `_indicator_` width **213.3 = active tab width** and h=2 flush at the tab bottom edge (166->168), height 30 / font 13 / radius 8 / padding `0 12px` all on-contract. **This is the defect the user reported as "tabs not aligned with each other".** Fix: give the panel the same 4px+1px horizontal inset, or move the strip padding inside its content box (`box-sizing` / negative margin), so both border-boxes are 640. | **Session-4 narrow-viewport confirmation (360x780 @2x):** tablist `x=61 -> right=294` (w=233) vs tabpanel `x=61 -> right=284` (w=223): `deltaLeft=0`, **`deltaRight=10`** — the overhang is EXACTLY 10px at 360 and EXACTLY 10px at 1280, confirming it is structural (`2 x (4px list padding + 1px border)`) and not a responsive/flex artifact. Proportionally it is far worse when narrow: **10/223 = 4.5% of panel width at 360** vs 10/640 = 1.6% at desktop, i.e. ~3x more visually prominent — which is why it reads as "tabs not aligned with the content". Reproduces on both tab groups in the story (3-tab and 4-tab). Tabs themselves are uniform (74.3x30 x3 / 55.8x30 x4) and `scrollWidth == clientWidth`, so nothing is scrolling or wrapping — the strip is simply wider than its panel at every viewport.
| N-09 | **S3** | **fixed@40c012087** | **`.rf-popover-motion` keyframe (`rf-scale-fade`) survives the reduced-motion helper class** — under the Storybook RM toggle, transitions zero out but the overlay enter keyframe still runs (1 animated node; identified `_content_* rf-popover-motion`). Form components measure fully clean (0 animated nodes across all 6 RM stories). Disable rule likely lives only in the media query; the helper class (and any class-driven host toggle) misses overlays. |
| N-10 | **S3** | **fixed@40c012087** | **DataTable narrow-stacked mode produces page-level horizontal scroll at 360px** (`document.scrollWidth > innerWidth`, right edges clipped) — the no-scroll fallback violating the doctrine it exists for. Also table-mode numeric right-align (Latency) leaks into stacked cards, reading as misalignment. `ui-datatable--narrow-stacked` |
| N-11 | **S3** | **fixed@40c012087** | **EditableTable same-row cell inputs measure 30px (Name) vs 52px (Description)** — ragged rows; contract controls are 26/30/36. Remove-row 28px ✓, Add 26px ✓, validation display ✓. `ui-editabletable--add-remove-enter-validation` |
| N-12 | **S3** | **fixed@40c012087** | **Chip remove buttons 18×18px** on removable + disabled chips — under the kit's ≥28px tap-target floor (`--rf-control-h-icon-sm`). Fix: padded pseudo-element hit area. `design-system-chip--gallery` |
| N-13 | S4 | **fixed@40c012087** | Button gallery icon-only sizes row: 4px vertical-center drift across 15 buttons (edge-aligned, wants `align-items:center`); both light+dark halves. |
| N-14 | S4 | **fixed@40c012087** | LoadingState compact tile spinner renders ~2px, nearly invisible on dark — "Loading providers" reads as bare text. Check Spinner size/tone defaults in that composition. |
| N-15 | S4/story | **fixed@40c012087** | kit Select `states` story never opens the select (its description promises grouped items/selected tint/hover) while its **reduced-motion twin auto-opens it** — open-state coverage lives in the wrong story. |
| N-16 | S4/story | **fixed@40c012087** | Dialog and Popover light-dark + narrow stories are trigger-only (no play/defaultOpen); Menu auto-opens — inconsistent overlay-story quality. Sheet/popover-narrow needed manual clicks too. |
| N-17 | S4/doc | **fixed@40c012087** | Badge measures 24px h / 12px font / pad 2×6 / r6 — single size only in gallery; prior ledger text referenced a "badge scale 16/18/22" that matches nothing measured. Reconcile docs or add size variants + stories. |
| N-18 | S4 | **fixed@40c012087** | Kit Select light-section "Small" variant measures 56.6px tall (expected 26) — hidden by the illegible light half; re-measure after N-03 fix. |
| N-19 | S3/infra | **fixed (this session)** | **MSW `onUnhandledRequest` bypass hardcoded to `http://localhost:6006/src/`** — on any other port (audit ran :6007) every dev-server asset fetch warned, flooding consoles (~100+ warnings/page on cascade stories). Fixed: origin-relative pathname check (warn only `/v1/`+`/p/` API paths). `.storybook/preview.tsx` |
| N-20 | S4/doc | **fixed@40c012087** | DataTable wide-mode horizontal scroll container does not use the sanctioned `.scrollX` class (own overflow container; behavior fine, contract vocabulary diverges). |
| N-21 | S3 | **partial@40c012087** | Half-pixel line-box evidence pack for L-04: Sheet content h=342.5; DataTable row pitch alternates 23/22 (13px×1.5=19.5 boxes). Paired `--rf-line-N` px tokens close it. |
| N-22 | S4/a11y | **fixed@40c012087** | SegmentedControl segments match neither `button` nor `[role=radio]` in role probes — semantics opaque; verify keyboard/AT pattern (likely label+hidden-input; confirm in source). |
| N-23 | S4 | **fixed@40c012087** | Kit Dialog/Popover internal scrollbars are native chunky, not the app's thin styled scrollbars — cosmetic inconsistency inside overlays. |
| N-24 | S4 | **fixed@40c012087** | **Callout + ErrorCallout text has no owned font-size** — `.callout_text` (Callout.module.css) declares no `font-size`; message renders at Radix-default **16px** in isolation (census `rt-Text:16` at `callout--default` and `error-callout--default`). Same consumer-dependent-typography family as L-10. Fix: `font-size: var(--rf-text-2)` or explicit size prop. |
| N-25 | S4 | **fixed@40c012087** | **Buddy mascot speech-bubble text renders at 11px** — below the 12px type floor (`_content_16moe_*` span, `chat--primary`). Sprite pixel-coord exemption covers canvas geometry, not bubble typography. |
| N-26 | — | **retracted (session 4)** | ~~Workspace bottomDock asymmetric padding 20R/12L~~ — FALSE POSITIVE. `_bottomDock_` is the **chat composer dock** (Chat.module.css:5), and the 12/20 split is intentional + token-composed: `padding-left: calc(--rf-space-1 + --rf-space-2)` matches the transcript's left gutter, `padding-right: calc(--rf-space-1 + --rf-space-4)` compensates scrollbar clearance so dock glass panels align with the message column. Documented in a source comment. |
| N-27 | infra | open | **Legacy `chat--*` story suite audit-hostile**: (a) missing MSW handlers → console flood + real 5s polling churn (`/v1/exec/list`, `/v1/voice/status`, `/v1/chats/test/skills-status`, `/v1/buddy/opportunities`, `/v1/worktrees`); (b) story chrome (rt-Container p48) overflows the chat shell 44px past the viewport, slicing the composer bottom in isolation renders. Harness stories fixed this in session 3; legacy suite was not migrated. |
| N-28 | S4 | **fixed@40c012087** | **Project-path label unowned mono font-size 13.3px** — `_path_1a3ab_*` renders "/Users/marc/Projects/refact-lsp" at 13.3px `ui-monospace` (UA mono-default cascade: no owned `font-size`, off-scale). Measured at `chat--configuration`. Ambient-typography family (N-24/L-10). |
| N-29 | S3 | **fixed@40c012087** | **Diff hunk file buttons 21px tall** — `_hunkFileButton_14pba_*` measured 21.0px ×10 at `chat--knowledge`; far under the 26px control minimum and the 28px `--rf-control-h-icon-sm` tap floor. Every diff hunk header ships an undersized interactive target. **Session-4 addition:** collapsed diff header pills `_diffHeader_t4p2a_*` measure **16.0px** ×2 at `chat-content--with-diffs` — even further under floor; treat hunk buttons + header pills as one diff-controls sizing fix. **Session-4 add 2:** `_showMoreButton_14pba_176` = **22.0px** x3 at `chat-content--markdown-issue`. Diff controls therefore ship THREE sub-floor heights: 16 (header pill) / 21 (hunk file btn) / 22 (show more), none on the 26/28/30/36 ladder. |
| N-30 | S4 | **fixed@40c012087** | **Transcript reveal triggers 29px** — `_trigger_1hjpa_*` ("System prompt", "frog.py:1-39, holiday.py:1-21, …" context-files toggles) measure 29.0px; on no control rung (26/30 neighbors). **Session-4:** at `chat-content--notes` the 29px context-file rows sit directly adjacent to 26px tool rows (labels 1102×29 vs 1102×26 in the layout overlay) — the 3px species split is visible in one glance. |
| N-31 | S3? | needs-reverify | **Unbreakable bold mono path line overflows the message column and gets clipped** (no wrap, no `.scrollX`) — observed visually at `chat--knowledge` ("…/frog.py /Users/marc/Projects/refact-lsp/tests/emergency_frog_…" cut at both clip edges, docW stays 1265 ⇒ hidden overflow). Element unmounted (virtualization) before rect probe; re-verify in `chat-content--*`/transcript stories and record the owning container. |
| N-32 | S4 | **fixed@40c012087** | **Inline-image lightbox triggers are micro tap targets** — `_trigger_14sym_*` buttons wrap inline markdown images at content size: measured **76×16** and **83×16** at `chat-content--assistant-markdown` (the 480×360 block image trigger is fine). Sub-20px interactive height for inline images. Consider a padded hit area. |
| N-33 | S2/story | **fixed@40c012087** | **`chat-content--tool-images` renders ZERO images** — story exists to showcase multimodal tool results ("Browser … 2 screenshots" summary renders) but DOM contains 0 `<img>`; only an empty padded band where `MultiModalToolContent` previews belong. Residual S2-10 fixture rot: the multimodal payload never reaches the renderer (fixture shape predates `MultiModalToolResult`?). Same at `chat-content--multi-modal`: two "1 screenshot" tool rows + "Here are the screenshots" copy, imgs:0. Also `_url_33dmt:13.3` = another UA-mono ambient font (N-28 family). |
| N-34 | **S3** | **fixed@40c012087** | **PlanBanner control cluster is entirely off-contract** (`PlanBanner.module.css`, measured at `chat-content-plan-banner--plan-v-1`, 1280x900@2x): `_toggleButton_dl3a7_34` = the full-width disclosure header, **1054.4 x 16px with `padding: 0`** — 10px under the 26px control minimum, 12px under the `--rf-control-h-icon-sm` 28px tap floor, and with zero padding it has no hit slack whatsoever; `_actionButton_dl3a7_95` ("History") = **22 x 22, `padding: 0`, `border-radius: 3px`** — off the height ladder AND off the radius scale (4/6/8/10/pill); (~~header icon 13x13~~ — **retracted session 4**: `ui-icon--sizes-and-tones` shows the kit icon scale is **13/15/18**, so 13 is the legitimate `sm` rung.) Credit where due: toggle/History vertical centers both land on exactly **37.0**, and toggle/body/H2 left edges are all flush at **74.5**, so alignment is correct — only control sizing is wrong. |
| N-35 | **S3** | **fixed@40c012087** | **Markdown `<hr>` in PlanBanner overhangs its siblings and is untokenized** (measured `chat-content-plan-banner--plan-with-deltas`): rule spans **74.5 -> 1200.5 = 1126px** while the sibling header row `_toggleButton_` spans **74.5 -> 1128.9 = 1054.4px** — a **71.6px right overhang** past every other element in the banner (and 41.6px past the History button edge at 1158.9); visually reads as "the divider sticks out". Styling is raw UA-ish: `border-top: 1px rgb(128,128,128)` (hardcoded #808080, no `--rf-color-border`) and `margin: 7px 0` (7 is on no spacing rung). Fix: constrain `hr` to the content column and tokenize colour + margins in the markdown stylesheet. |
| N-36 | S4 | **fixed@40c012087** | **PlanBanner markdown heading hierarchy is effectively flat** — H2 **15px/700** vs H3 **14px/700** (measured, same story): a 1px step at identical weight makes "Plan updates" (section) and "Progress update" (subsection) read as the same level. Consider weight or colour differentiation rather than a 1px size step. |
| N-37 | S4/perf | **fixed@40c012087** | **Reselect instability warning fires on plan/transcript render** — console: *"An input selector returned a different result when passed same arguments"* (reproduced at `chat-content-plan-banner--plan-with-deltas`, previously seen on transcript stories). An unstable input selector in the ChatContent render path causes needless recomputation. |
| N-38 | **S3/consistency** | **fixed@40c012087** | **SegmentedControl and Tabs disagree on every token for the same "pick one of N" job** (measured session 4, `ui-segmentedcontrol--states` vs `ui-tabs--states`, 1280x900@2x). Item height: Tabs **30** (on ladder) vs SegmentedControl **33.3** (default) and **40.6** (roomy) — both fractional and off the 26/28/30/36 ladder. Item font: Tabs **13px** vs SegmentedControl **16px** — 16 is off the 12/13/14/15/19 type scale entirely. Item padding `0 12px` vs `0px`; container padding **4px** vs **2px**; item radius 8 vs 0 (indicator carries it). Primitive differs too: `button[role=tab]` vs visually-hidden `input` + `label` inside `div[role=radiogroup]`, so segmented items are invisible to control-height/a11y tooling and get different focus styling. NOTE the useful contrast: SegmentedControl spans **61 -> 701 = 640**, exactly matching the content column, while the Tabs strip is 650 (N-08) — the sibling component proves the tab overhang is a bug, not house style. Indicator sizing is exact in both (indicator width == active item width). |
| N-39 | **S3/consistency** | **fixed@weight-sweep** | **Typography is only half-tokenized: 12 distinct font-weights app-wide, zero weight tokens.** `styles/tokens.css` defines `--rf-text-*` sizes but NO weight tokens, so every weight is a raw literal. App-wide census (`grep -rhoE "font-weight: *[0-9]+" components/ styles/ features/ --include=*.css`): **650 x60, 600 x54**, 700 x25, 500 x14, 400 x4, 800 x2, 750 x2, 900 x1, 760 x1, 620 x1, 560 x1, 450 x1. The two dominant values are visually indistinguishable and serve the same "semibold label" intent, and the split runs INSIDE the kit boundary: `ui/ListRow` + `ui/Text` use **600** while `ui/Badge`, `ui/Button`, `ui/DataTable`, `ui/EditableTable`, `ui/EmptyState`, `ui/ErrorState`, `ui/ModelSelector` use **650**. One-off oddities (760, 750, 620, 560, 450) have no possible design rationale. Live confirmation: `chat-form--primary` renders `12px/450` alongside `13px/650`; `chat-composer-modeselect--open` renders **`13px/650` and `13px/600` inside ONE popover** — same size, same surface, two indistinguishable weights. Fix: add `--rf-weight-regular/medium/semibold/bold` tokens, collapse 600+650 to one, and re-point the one-offs. |
| N-40 | **S2/story** | **fixed@40c012087** | **`chat-form--with-attached-images` shows no attachments** — the story renders byte-identically to `chat-form--primary` (same 1219x31 textarea, same 3 pills 108/124/81 x26, same 8 icon buttons x28, identical consistency fingerprint apart from one border alpha .18 vs .10). DOM proof: `img` count **0**, no attachment/tray/thumb element, full textContent = `"0gpt-4o.128KAgent.12 toolsNo branch0"`. The composer attachment tray never receives `attached_images` (known session-3 open item, now measured). Either wire the tray or rename the story — as-is the attachment UI has ZERO visual coverage. Same "story lies about its content" family as N-33. |
| N-41 | **S3** | **fixed@40c012087** | **Model-selector search field is a 21px input with zero padding** — `_searchInput_1ts6g_41` measures **377 x 21**, `padding: 0px`, `font-size: 14px`, native `<input>` (measured `chat-composer-chatsettingsdropdown--open`). 5px under the 26px control minimum, 7px under the 28px tap floor, and 14px type inside a 21px box leaves no vertical breathing room. Every other control in the same popover is on-contract (Token limits disclosure 416x26, trigger 26). | **Session-4 root cause + second host:** `components/ui/ModelSelector/ModelSelector.module.css:41` declares `.searchInput` with `padding: 0`, `border: 0`, `font: inherit` and **no `height`/`min-height` at all**, so the field height is purely the inherited font line box. Measured in two hosts: app `chat-composer-chatsettingsdropdown--open` renders **377 x 21 @ 14px**; kit `ui-modelselector--popover-grouped` renders **357 x 24 @ 16px** — the same class, two sizes, because the size is inherited from ambient typography rather than declared. Both are below the 26px control minimum and the documented 28px tap-target floor. This also makes it an instance of the N-47 height-strategy theme. Fix: give `.searchInput` a fixed control height + `--rf-text-*` size instead of `font: inherit`.
| N-42 | **S4** | **fixed@40c012087** | **One row in the 20-row model list is 3px taller than the other 19** — all rows share `x=814, w=416, padding: 8px`, but the row carrying the `Default` badge (`gpt-4o` + `$2.50 / $10.00`) measures **62px** while the other 19 measure **59px** (measured `chat-composer-chatsettingsdropdown--open`). The badge/price line pushes that single row out of the list rhythm. Fix: give rows a fixed min-height so optional badges never change row height. |
| N-43 | **S3** | **fixed@40c012087** | **ToolConfirmation type hierarchy is inverted — the muted footnote is the largest text in the panel.** Measured every text node at `toolconfirmation--default`: title "Model wants to run:" **14px/650**, primary body "Command needs confirmation due to `*` rule." **14px/400**, command preview "SELECT *" 12px/400, actions Confirm/Stop 13px/650 — but the muted footnote "You can modify the ruleset on **Configuration Page**" renders **15px/400** in `rgba(255,255,255,0.48)`. The least important, de-emphasised line is one step LARGER than both the panel title and the primary body. Fix: drop the footnote to `--rf-text-1`/`--rf-text-2`. |
| N-44 | **S2/S3 — systemic root cause** | **fixed@40c012087** | **The GUI has no global `box-sizing: border-box` reset; only 4 of ~24 kit components opt in, and the ones that forgot silently inflate.** Evidence: `grep -rn "box-sizing" --include=*.css src/` = **67 hand-written declarations** (28 in `components/`, 38 in `features/`, 1 in `styles/`) — the same workaround retyped 67 times instead of one reset. The only non-component rule is `styles/responsive.css:10`, scoped to `[data-element="app-root"]` (a single element, no `*` selector). Inside `components/ui/` only **Field, Switch, SettingsShell, SegmentedControl** declare it; **Badge, Chip, Button, Card, Surface, ListRow, DataTable, EditableTable, EmptyState/ErrorState/LoadingState, Dialog, Popover, Menu, Sheet, Select, ComboBox, ModelSelector, VirtualList, Icon, Tabs** do not. **Measured consequence — the entire Badge scale is wrong.** `ui/Badge` declares `border: var(--rf-hairline) solid transparent` (+2px) plus min-height + vertical padding, so under content-box: `size-xs` 16+0+2 = **18** (intended 16), `size-sm` 18+(2x2)+2 = **24** (intended 18), `size-md` 22+(4x2)+2 = **32** (intended 22). Live proof at two independent stories: `toolconfirmation--default` badge computes `height/min-height: 22px`, `padding: 4px 8px`, `box-sizing: content-box` and renders **72.5 x 32** (parent is `align-items: flex-start`, so not flex stretch); `design-system-badge--gallery` renders **7/7 badges at 24px** with `min-height: 18px` + `padding: 2px 6px` + content-box. The canonical kit gallery ships the bug, and it only exercises `size-sm` — never `xs`/`md` — which is why nobody caught it. This single cause explains both historical numbers: S3-22 measured a 24px chip (that was `size-sm`) and session 4 measured 32px (`size-md`), and it means the Wave-4 "badge-scale chips" fix could never have worked. `ui/Button` is immune only by accident — it uses `height:` + `padding: 0 var(--rf-space-*)` (zero vertical padding), which is why buttons measure exactly 26/30/36. **Fix: add `*, *::before, *::after { box-sizing: border-box }` to `styles/`, then re-measure every `min-height`-based component and delete the 67 redundant declarations.** |
| N-45 | **S4** | **fixed@40c012087** | **`Chip` has no height floor, so chip rows are ragged.** `ui/Chip/Chip.module.css` declares **`min-height: auto`** with `line-height: 1`, so a chip is only as tall as its tallest child. Measured at `design-system-chip--gallery`: **5 chips render 28px and 1 renders 23px** in the same row — chips carrying a leading icon or an 18px remove button are 28, the text-only "chip radius" chip is 23. A 5px step between neighbours. Chip is also `box-sizing: content-box` (see N-44) and its 18x18 remove buttons sit under the 28px tap floor. Cross-component note: the kit now has three pill families at three heights — Badge **24**, Chip **23/28**, Button **26/28/30/36**. Fix: give Chip a `min-height` rung tied to the control scale so removable and plain chips align. |
| N-46 | **S3** | **fixed@40c012087** | **`ui/Select` trigger has no fixed height, so it stretches — 30px in one section, 56.6px in another, in the SAME story.** At `ui-select--states` all three triggers carry identical CSS (`min-height: 30px`, `padding: 0 8px 0 12px`, `align-items: center`, `display: flex`) and identical children (19.5px label + 13px icon), yet computed `height` is `30px`, `30px`, **`56.625px`**. The tall one sits in `_panel_irfx4_11` which is **`display: grid` with `align-items: normal`** (= stretch for grid items), so the trigger fills its row. Because Select declares only `min-height` and never `height`, it is stretchable by any grid/stretch parent. `ui/Button` is immune precisely because it declares `height: var(--rf-control-h-*)`. 56.6 is on no rung of the 26/30/36 ladder. Fix: set `height` (or `align-self: center`) on the Select trigger. |
| N-47 | **S3 — theme** | **fixed@40c012087** | **The kit has no single strategy for control height, and only the fixed-`height` strategy survives real layouts.** Four kit components, four different mechanisms, three of them measurably broken: `ui/Button` uses `height: var(--rf-control-h-*)` + zero vertical padding and measures exactly **26/30/36 everywhere** (immune to both box-sizing and stretch); `ui/Select` uses `min-height` only and **stretches to 56.6px** in a grid parent (N-46); `ui/Badge` uses `min-height` + vertical padding + border under content-box and **inflates to 24/32 instead of 18/22** (N-44); `ui/Chip` uses `min-height: auto` and renders **ragged 23/28 rows** (N-45). Every "wrong size" symptom in this audit traces to one of these three deviations from the Button pattern. Recommended: document a single control-height recipe (fixed `height` + `border-box` + zero vertical padding) in `gui/AGENTS.md` and migrate Badge/Chip/Select onto it. |
| N-48 | **S4** | **fixed@40c012087** | **No global heading reset — raw `<h*>` tags fall back to UA sizes that exist on no type scale.** `grep` finds no `h1`-`h6` rule anywhere in `src/styles/*.css`, and `ui/Card/Card.module.css` declares no `font-size` at all. Measured at `design-system-card--gallery`: the card titles render **18.72px/700** — the browser default `1.17em x 16px` — while the section heading beside them renders the tokenized **19px/700**. Two near-identical heading sizes in one story, one intentional and one accidental, 0.28px apart. UA heading sizes (32/24/18.72/16/13.28/10.72) are all off the 12/13/14/15/19 scale and drift with root font-size. Raw `<h*>` also appears in production kit files (`SettingsShell.tsx`, `SettingItem.tsx`) — those DO style them (Settings page measures a clean 19/700), so today the leak is confined to unstyled call sites. Fix: add a heading reset mapping `h1`-`h6` onto `--rf-text-*` so an unstyled heading can never fall off-scale. | **Session-4 generalization — the gap is not limited to headings.** `grep` confirms there is **no `font-size` declared on `body`, `html`, or `:root`** anywhere in `src/styles/*.css`, and no `code`/`kbd`/`samp`/`pre` rule either. So every unstyled element falls to a UA size that exists nowhere on the 12/13/14/15/19 scale: **plain text 16px**, **headings 32/24/18.72/16/13.28/10.72px**, **monospace 13.333px**. Measured instances: `design-system-surface--gallery` renders all **7 surface labels at 16px/400**; `design-system-card--gallery` renders card titles at **18.72px/700**; `many-tools-grouped` renders inline paths at **13.3px**. Fix: declare a base `font-size: var(--rf-text-2)` plus a heading and monospace map so falling off the scale is impossible by default.
| N-49 | **S3** | **fixed@40c012087** | **Tool-card inline code uses relative `em` font sizes, producing off-scale 13.3px / 12.6px text — and the SAME element has two implementations in one folder.** Measured at `tool-cards-file-ops-basics--many-tools-grouped`: `_path_1a3ab_1` and `_query_1e777_1` both render **13.3px** (on no rung of the 12/13/14/15/19 scale). Source: `ChatContent/ToolCard/ListTool.module.css:3` declares `font-size: 0.95em` (x 14px parent = 13.3px), while its sibling `ChatContent/ToolCard/PageSnapshot.module.css:11` declares the same `.path` element with `font-size: var(--rf-text-1)` (12px). One semantic element (a file path in a tool header), two sizes, same folder. Repo-wide the ratio is **436 tokenized `font-size: var(--rf-text-*)` vs 10 relative**, and **7 of the 10 are in `ChatContent/ToolCard/`**: `SearchTool` `0.95em`, `ChromeTool` `0.95em`, `ListTool` `0.95em` x2, `ContextFileList` `0.9em` x2, `WebTool` `0.95em` (plus `Markdown.module.css` `0.9em` and `ToolMarkdown.module.css` `1em !important`). There is also no `code`/`kbd`/`samp`/`pre` font-size reset (same gap as N-48 for headings), so unstyled monospace falls to the 13.333px UA default independently. Fix: replace the 7 ToolCard `em` sizes with `--rf-text-*` tokens. |
| N-50 | **S3** | **fixed@40c012087** | **A validation message in one EditableTable cell deforms the neighbouring input (30px -> 52px) and knocks it 11px out of alignment.** Measured at `ui-editabletable--add-remove-enter-validation`: row 1 is clean — Name `262.4x30` and Description `381.6x30`, both `cy=152.5`. Row 2 renders a validation message ("Use snake_case.") under the Name cell, which grows the grid row; because the cell input has no fixed height it **stretches to `381.6x52`** and its centre moves to `cy=210.5` while its neighbour stays at `cy=199.5` — an **11px baseline drift between two inputs on the same row**. Same root mechanism as N-46 (min-height-only control inside a stretching grid parent), so it is covered by the N-47 theme. User-visible consequence: typing an invalid value visibly deforms an adjacent field. Fix: fixed control height + `align-self: start` on cell inputs. Detector note: my row-drift detector did NOT flag this (11px drift across differently-sized siblings fell outside its row-banding), so sibling-height comparison must stay part of the manual protocol. |
| N-51 | **S3 — cross-cutting** | **partial@40c012087** | **Sub-28px interactive targets recur across the kit and app, despite the kit documenting 28px as its tap-target floor.** The 28px floor is the stated justification for `--rf-control-h-icon-sm: 28px` (see S3-21 resolution), yet measured targets below it are widespread: **20x20** Home "Delete chat" (S1-1, also the most destructive action in the app), **18x18** Chip remove buttons (N-45, measured at `design-system-chip--gallery`), **377x21 / 357x24** ModelSelector search input in its two hosts (N-41), **20x20** Slider thumb (measured at `ui-slider--states`; mitigated on one axis by the 640px track, but still a 20px grab target), **23px** text-only Chip (N-45). These are individually minor but collectively mean the documented floor is not enforced anywhere. Recommend a lint/test that asserts every interactive element resolves to >= 28px in at least its smaller dimension, or an explicit documented exemption list. |
| N-52 | **S3** | **fixed@40c012087** | **Two adjacent surface elevation tiers are visually indistinguishable, collapsing a 3-tier ladder to 2.** Measured at `design-system-surface--gallery`: `surface-1` = `rgba(255,255,255,0.035)` and `surface-2` = `rgba(255,255,255,0.043)` — a **0.008 alpha difference**. Composited over the `rgb(12,13,15)` page base these resolve to **RGB 20.5 vs 22.4**, a contrast ratio of about **1.03:1**, i.e. imperceptible. `surface-3` (`0.07`, border `0.18`) is the first genuinely distinct step. Everything else in this gallery is exemplary — 6 surfaces all `186.2x114`, uniform **12px** gaps, radius 10, padding 12, correct `plain -> surface-1/2/3 -> overlay -> selected` recipe. Fix: widen tier 1 vs tier 2, or delete one tier and renumber. |
| N-53 | **S3/a11y** | **fixed@40c012087** | **ToolConfirmation encodes the confirm-vs-deny verdict in colour alone, and spends its redundant text channel on the wrong field.** Measured at `toolconfirmation--mixed`. `getConfirmationMessage` (`ToolConfirmation.tsx:50-53`) builds its summary from `confirmationToolNames` / `denialToolNames`, i.e. `r.tool_name` — not the command. The fixture's two reasons both carry `tool_name: "postgres"`, so the panel renders verbatim: *"Command needs confirmation: **postgres**. Following command was denied: **postgres**."* — two sentences naming the same string, from which the verdict of each listed command is unrecoverable. The orders also disagree: the command list is denial-first (`SELECT *`, `DROP *`) while the prose is confirmation-first, so positional inference fails too. That leaves the **badge tone as the sole carrier**: `rgb(216,115,109)` (danger) vs `rgb(205,160,78)` (warning), both at 13px/650, both reading the identical text `postgres`, with no legend and the same `AlertTriangle tone="warning"` header icon in either case. Red-vs-amber is the deuteranomaly confusion axis, so a security decision is encoded in a channel a meaningful share of users cannot read. Fix: label the badge with the verdict (`Denied` / `Needs confirmation`), or build the prose from `command`, and align the two orderings. **Retraction attached:** I first read this as inverted tone mapping (denied getting the softer colour). The fixture (`__fixtures__/confirmation.ts` `MIXED_PAUSE_REASONS`) proves `SELECT *` is the denial and `DROP *` the confirmation, and `ToolConfirmation.tsx:271` maps `r.type === "denial" ? "danger" : "warning"` — **the tones are correct**; only the text channel is broken. |
| N-54 | **S3/consistency** | **fixed@40c012087** | **The transition-dialog family leaks raw internal mode identifiers into user-facing chrome, in three different formats.** Measured across four dialogs at 1280x900@2x. `chat-dialogs-modetransitiondialog--switch-mode` renders its title row as **`agent -> Ask`** — lowercase raw id on the left, title-cased display name on the right, 8px apart, joined by an arrow whose entire purpose is direct comparison. This is structural, not a fixture accident: `ModeTransitionDialog.tsx:46-49` declares `currentMode: string; targetMode: string; targetModeTitle: string; targetModeDescription: string` — there is **`targetModeTitle` but no `currentModeTitle`**, so the component cannot render a display name for the source mode. Corroborated by `--restart-current-mode`, which receives only `targetModeTitle` and correctly renders **`Agent`**. Both TaskPlanner stories go further and render the bare snake_case enum member **`task_planner`** at 12px/650 beside a 15px/600 title. Three formats for one concept across one family: `agent` / `Ask` / `task_planner`. **BROADENED session 5 — this is not confined to mode chips.** `TaskProgressWidget` renders raw internal identifiers as user-facing labels throughout: **`needs_work`** (12/650, accent), **`goal_pursuit`** x2, **`nudge`**, **`checkpoint`** (measured at `task-progress-widget--paused-goal`). Two unrelated components now confirmed leaking enum members into chrome, so the fix is a shared display-name mapping, not a per-component prop. **BROADENED AGAIN session 6 — now three components, and one leak crosses the language boundary.** `chat-transcript-elements--error-card` renders its category chip as the literal **`ProviderTransient`** (12/650, warning tone `rgb(205,160,78)`) — PascalCase, which matches no GUI naming convention but exactly matches the engine's Rust `UserErrorCategory` variant in `retry_policy.rs`. So the transcript is printing a **Rust enum variant name** verbatim to the end user, in the one surface (an error) where comprehension matters most. Also added at `--completed-goal`: `needs_work`, `checkpoint`, `goal_pursuit` x2, `nudge`, `completed`. The identifier zoo is now three formats across three components plus a cross-language leak: `agent` / `task_planner` / `needs_work` / `ProviderTransient`. Fix: add `currentModeTitle`, and route mode/status/event-kind/error-category identifiers through one humanising lookup — including a display-name map for engine-supplied categories at the API boundary. |
| N-55 | **S2/safety** | **fixed@40c012087** | **`DeletePopover` reverses the action order used by every other confirmation surface, putting the irreversible button where the escape hatch lives everywhere else.** Measured at `components-deletepopover--open`: `Delete 71.2x30 @ x=25.0` (LEFT), `Cancel 70.5x30 @ x=104.2` (RIGHT). Every other confirm surface measured this session is `Cancel` LEFT / primary RIGHT: ModeTransition switch (`Cancel@687.1`, `Switch Mode@769.6`), ModeTransition restart (`Cancel@665.3`... `Restart Mode@747.8`), TaskPlanner new-task (`Cancel@687.1`, `Create Task@769.6`), TaskPlanner add-planner (`Cancel@665.3`, `Create Planner@747.8`). Four surfaces train "left slot = safe exit"; the single surface that inverts it is the only **irreversible** one, so a user reaching for the learned Cancel position hits **Delete**. Not theoretical on this codebase: ledger item S1-1 records that this audit permanently destroyed a real chat via an unguarded delete affordance in session 1, and the popover built to prevent that failure places the destructive control in the safe slot. Fix: reorder to `Cancel | Delete`. |
| N-56 | **S3** | **fixed@40c012087** | **Confirmation actions-row gap splits 8px vs 12px across the same family** (supersedes and quantifies S3-11's two-point observation). **CORRECTED session 5:** Measured: **8px** — `DeletePopover` only (104.2 - 96.2). **12px** — ModeTransitionDialog x2, TaskPlannerDialog x2, **CreateWorktreeModal (re-measured live: `Cancel@680.7` w70.5 -> `Create@763.1` = 11.9 ~ 12)**, AddProviderInstance (prior ledger). The old ledger's "CreateWorktree 8" datum is STALE — Wave 3 fixed it. DeletePopover is therefore the lone outlier against five surfaces at 12, which makes this a single-site fix rather than a split. Both values sit on the spacing scale, so no off-scale census can catch it — it needs one `--rf-actions-gap` decision. |
| N-57 | **S3** | **fixed@40c012087** | **`margin-top` stacked on a `gap`-spaced flex column produces two off-scale, mutually inconsistent section gaps.** Measured at `features-worktrees-createworktreemodal--open`: description->fields = **24px**, fields->actions = **28px**. Source-confirmed in `features/Worktrees/Worktrees.module.css`: `.modalFields { margin-top: var(--rf-space-3) }` (12) at l.148-150 and `.modalActions { margin-top: var(--rf-space-4) }` (16) at l.424-429 — both children sit in a flex column that already applies `gap: 12px`, so each margin ADDS to the gap (12+12=24, 12+16=28) instead of replacing it. Neither 24 nor 28 is on the spacing scale (2/4/6/8/12/16/22/32), and the two separators disagree despite playing the same structural role. Same failure mode as the S3-11 bug Wave 3 repaired (a local wrapper fighting the Dialog column gap), one level further down. `MergeWorktreeModal.tsx:245` reuses both classes, so at least two modals inherit it. Everything else in this modal is on-contract: inputs 388x30, label->control gap 4, field->field gap 12, buttons 30, content inset 17 = 16 pad + 1 border, census clean. **Partial retraction:** I first suspected the modal hand-rolled its fields; it does not — `CreateWorktreeModal.tsx:2` imports and uses kit `FieldText` (l.94, l.109). This is a spacing-composition bug only. |
| N-58 | **S3/a11y** | **fixed@40c012087** | **`CreateWorktreeModal` renders its form error unattributed, positioned under the wrong field, and bypasses the kit's entire error apparatus.** Measured at `features-worktrees-createworktreemodal--with-error`: `_errorText_` top **527.5**; Base-branch input bottom **515.5** (distance **12px**); Branch-name input bottom **428.0** (distance **99.5px**) — the message is 8x closer to the field it does not describe, and proximity is the dominant grouping cue, so "A worktree for this branch already exists." reads as a Base-branch error. **Neither input shows an invalid state**: both compute `background: rgba(255,255,255,0.035)` (surface-1), identical to the no-error story — no border, no tint, offending field never identified. The kit already solves this: `components/ui/Field/Field.tsx` exposes `error?: React.ReactNode` (l.22), sets `data-invalid` (l.113), mints `errorId = useId()` (l.107) and renders `<FieldError role="alert">` (l.130, l.150). `CreateWorktreeModal.tsx:152` bypasses all of it with a bare form-level `<p className={styles.errorText}>` — no `data-invalid`, no `role="alert"`, no `aria-describedby`, so screen-reader users get neither an announcement nor a field association. **Colour-only differentiation (N-53 family):** `Worktrees.module.css:168-176` gives `.errorText` and `.helpText` the same `font-size: var(--rf-text-1)` + `line-height`, and l.453-456 changes only `color` — both measure **12px/400**, distinguished solely by red-vs-muted. Dialog growth itself is clean: 311 -> 341 = exactly +30 (18px line + 12px gap). Fix: pass `error` into the owning `FieldText`. |
| N-59 | **S3** | **fixed@40c012087** | **`TaskProgressWidget`'s two stacked collapse headers are both an off-ladder 34px, with mismatched horizontal padding — and this is the root cause of L-02.** Measured at `task-progress-widget--paused-goal` (recurs identically in `--budget-exhausted`, `--no-budget-limits`, `--with-tasks`): `_header_30xgb_5` w=1221 **h=34** `height:34px` `padding: 8px 12px`; `_goalHeader_30xgb_25` w=1195 **h=34** `height:34px` `padding: 8px`. Both set an explicit `height` (`min-height: auto`), so this is deliberate rather than flex stretch, and **34 is on no rung of 26/28/30/36**. L-02 recorded this class at 34.0px at `chat--configuration` and attributed it to the composer's "Set a goal" row — it is in fact TaskProgressWidget's own header, rendered **twice per widget, nested**, in every host. The two also disagree on horizontal padding (**12 vs 8**), which is why the inner header is 26px narrower and the nesting reads as an arbitrary indent rather than a deliberate step. Fix: one header height on the ladder + one padding value. |
| N-60 | **S4** | **fixed@40c012087** | **Budget progress text has no state-dependent emphasis — the saturated value is styled exactly like a mid-run value.** Measured: `--paused-goal` renders `5/12 turns · 8450/24000 tokens` and `--budget-exhausted` renders `6/6 turns · 10000/10000 tokens`, both at **12px/400 in `rgba(255,255,255,0.48)`** — byte-identical typography. The number that *caused the goal to stop* (100% of turns and tokens consumed) appears in the widget's most de-emphasised register, with no bar, colour shift, or weight change at saturation. The only escalation is the status chip, which names the condition but not which limit was hit. Credit where due: the chip tone ladder itself is correct — Active `rgb(127,147,216)` / Paused `rgb(205,160,78)` / Budget exhausted `rgb(216,115,109)`. |
| N-61 | **S2/correctness** | **fixed@40c012087** | **`goalControlAvailability` treats only 2 of the 7 documented terminal states as terminal, so a goal owned by a *different chat* still renders an enabled `Resume`.** `TaskProgressWidget.tsx:191` declares `const isTerminal = goal.status === "completed" \|\| goal.status === "stopped";` — but `gui/AGENTS.md` documents the terminal/held set as `verifying`, `paused`, `completed`, `stopped`, `budget_exhausted`, `no_progress`, **`transferred`**. Because `canResume = !isTerminal && (status === "paused" \|\| !goal.active)` (l.194) and a transferred goal is by definition `active: false`, the arithmetic for `status: "transferred"` is `canPause:false, canResume:**true**, canStop:**true**` — the widget offers to resume and stop a goal this chat no longer owns, dispatching `goal_control{action:"resume"}` against non-owned state. `transferred` appears in the file only twice, in `GOAL_STATUS_LABEL` (l.74) and as a tone `case` that falls through to `default → "muted"` (l.101-103); there is **no ownership guard anywhere in the component**. `no_progress` takes the same path. **Live proof of the exact code path** (measured this session, same-pass): `--budget-exhausted` (`active:false`, a documented terminal state) renders `Apply budget 102x26`, `Edit 43x26`, **`Resume 84x26`**, **`Stop 64x26`** — a control set byte-identical to `--paused-goal` — while `--completed-goal` correctly renders only `Apply budget` + `Edit` because `GoalControls` returns `null` when all three flags are false (l.288, l.592). So the component demonstrably knows how to hide the controls; `isTerminal` just does not list the states that should trigger it. Resume-after-raising-the-budget is defensible for `budget_exhausted`; resume-a-transferred-goal is not. Fix: derive `isTerminal` from the documented status set (or from `active` + status), and treat `transferred` as ownership-terminal regardless. |
| N-62 | **S3/coverage** | **fixed@40c012087** | **Half the goal status machine has zero visual coverage.** `GOAL_STATUS_LABEL` (`TaskProgressWidget.tsx:67-75`) enumerates 8 statuses; the story suite covers 3 (`active`, `paused`, `completed`) plus `budget_exhausted`. **`verifying`, `stopped`, `no_progress`, and `transferred` have no story at all** — which is exactly why N-61 survived: the one state that exposes it is the one nobody can look at. `transferred` is also the only status whose tone is not explicitly assigned (it shares the `default` arm → `muted`, l.101-103), so its chip is unstyled-by-fallthrough and unreviewed. Note the contrast with the states that *are* storied: the tone ladder there is exemplary and complete — `Active` `rgb(127,147,216)` accent / `Paused` `rgb(205,160,78)` warning / `Budget exhausted` `rgb(216,115,109)` danger / `Completed` `rgb(95,174,139)` success (measured). Fix: add the four missing status stories; they are pure-prop args like the existing four. |
| N-63 | **S4** | **fixed@40c012087** | **The transcript error card renders the raw upstream string LARGER than the actionable guidance.** Measured every text node at `chat-transcript-elements--error-card`: headline "Temporary provider interruption" **14/500** `rgba(255,255,255,0.92)`; primary body "The model provider ended the request unexpectedly…" **14/400** `@0.92`; guidance "Retrying may succeed after the condition clears" **12/400** `@0.48`; raw detail "upstream connection closed" **14/400** `@0.48`. The two muted lines sit at the same 0.48 opacity but different sizes, and the larger of the two is the opaque upstream string the user can do nothing with, while the sentence telling them what to *do* is the smaller. **This makes N-43 a pattern rather than a one-off**: ToolConfirmation puts its muted footnote (15px) above its own title (14px), and ErrorCard puts its raw detail (14px) above its guidance (12px) — two unrelated components independently ranking the least actionable line highest. Fix: guidance should outrank raw provider text; consider `--rf-text-2` for the detail and `--rf-text-3` for the hint. |
| N-64 | **S2/coverage** | **fixed@40c012087** | **All three image-rendering stories are visually empty — the GUI's entire image pipeline has zero real coverage.** Consolidates and extends N-33/N-40 with a third mechanism: (a) `chat-content--tool-images` and `chat-content--multi-modal` render **`img` count 0** (N-33); (b) `chat-form--with-attached-images` renders byte-identically to `--primary` with **`img` count 0** and no tray element (N-40); (c) **new this session** — `chat-transcript-elements--user-with-images` does render an `<img>`, correctly sized at **80x80** inside an `_trigger_14sym_1` 80x80 lightbox trigger, but its source is a **1x1 base64 PNG** (`naturalWidth 1 x naturalHeight 1`, `data:image/png;base64,iVBORw0KGgo…`), i.e. a single pixel stretched 80x. So the one story that reaches the renderer still exercises nothing: aspect-ratio handling, `object-fit`, letterboxing, overflow of a wide screenshot, and load/error states are all invisible because every candidate image is degenerate. Credit: the 80x80 trigger is well clear of the 28px tap floor, unlike the 76x16 / 83x16 inline triggers in N-32. Fix: put one real multi-hundred-pixel non-square asset in the fixtures; it would cover all three stories at once. |
| N-65 | **S3 — systemic** | **fixed@40c012087** | **`.actionButton` is an unowned convention reimplemented in 6 CSS modules with 4 sizing strategies and 2 different radius token families — and this is the root cause of N-34's mystery `radius: 3px`.** `grep -rn "^\.actionButton" --include=*.module.css src/` returns six independent definitions of the same semantic element (a small square icon action on a card/row header): **`Dashboard/…/RecentItem.module.css:111`** `width/height: var(--rf-control-h-icon-sm)` (28) + `--rf-radius-chip` — token-correct and on the tap floor; **`ToolCard/ReportToolCard.module.css:99`** `width/height: var(--rf-control-h-sm)` (26) + `--rf-icon-sm` svg — tokenized but a *different rung*; **`PlanBanner.module.css:95`** hardcoded **`22px`** + **`border-radius: var(--radius-1)`**; **`SkillReportCard.module.css:26`** hardcoded **`22px`** + `--rf-radius-chip`; **`Worktrees.module.css:274`** padding-derived + `--rf-radius-ctl`; **`Knowledge/MemoryDetailsEditor.module.css:102`** `min-width: 0` only. Both 22px variants are measured live — PlanBanner `_actionButton_dl3a7_95` **22x22** at `chat-content-plan-banner--plan-v-1`, SkillReportCard `_actionButton_j2b83_26` **22x22** at `chat-transcript-elements--skill-cards` — putting two of six **6px under the documented 28px tap floor** (N-51 family). The radius split is the sharper defect: PlanBanner reaches for **`var(--radius-1)`, a Radix token**, where every sibling uses `--rf-radius-*`; Radix `--radius-1` resolves to **3px**, which is exactly the off-scale radius N-34 measured and could not explain. Fix: promote `.actionButton` to a kit primitive (or route all six through `ui/IconButton size="sm"`), and lint for `var(--radius-*)` usage outside the Radix adapter layer. |
| — | — | **VERIFIED FIXED** | **S3-11 title->description gap** re-measured at CreateWorktreeModal: title 311.5->334, description starts **346.0** = clean **12px** gap (was 0px pre-Wave-3). Wave 3's fix holds. |

---

## Part 3 — Per-story audit log (45/171)

| Story | Verdict | Key numbers / notes |
|---|---|---|
| ui-button--variants-sizes-states | 🔴 N-01, N-03, N-13 | heights exactly {26×12, 28×10, 30×74, 36×20} ✓; fonts 13/14/15/19 ✓ |
| design-system-badge--gallery | ✅ (+N-17) | 24px/12px/2×6/r6 uniform; on 8px grid; muted tone → contrast pass later |
| design-system-chip--gallery | ⚠️ N-12 | chips 28px/13px/r999 + r6 variant ✓; truncation ✓; ×=18×18 |
| design-system-statusdot--gallery | ✅ | 9 states, dot/label centers aligned |
| design-system-surface--gallery | ✅ | all 186×114 r10 pad12; alphas .035/.043/.07; overlay 28,28,31,.94; plain transparent/borderless ✓ (note: s1↔s2 visually indistinguishable — token design question) |
| design-system-card--gallery | ✅ | default hairline+surface-1+r10; selected accent-soft ✓ |
| ui-icon--sizes-and-tones | 🔴 N-02, N-03 | svg sizes exactly {13×28, 15×14, 18×14} ✓ but placement scrambled |
| design-system-skeleton--gallery | ✅ | rhythm 8px, varied last line, avatar rows centered |
| design-system-emptystate--gallery | ✅ | compact 14px vs full-page 19px hierarchy intentional |
| design-system-errorstate--gallery | ✅ | danger color-only ✓; Retry 30×65.9 both |
| design-system-loadingstate--gallery | ⚠️ N-14 | skeleton tiles fine |
| design-system-overview--overview | ✅ | token workbench renders all sections |
| ui-field--controls | ⚠️ N-03 | inputs 30 ✓, switches 20 ✓, fonts 12/13/19 ✓; light half illegible |
| ui-field--settings-page | 🔴 N-03 (worst) | entire story illegible in dark canvas |
| ui-field--narrow-settings-page | 🔴 N-03 | same; narrow reflow itself correct |
| ui-switch--states | ✅ (+N-03) | all 36×20 track/16 thumb exact; no disabled+checked sample |
| ui-slider--states | ✅ (+N-03) | single/range/disabled render; no focus/edge samples |
| ui-select--states | ⚠️ N-15, N-18 | trigger 30/13/r8 exact; disabled ✓ |
| ui-segmentedcontrol--states | ⚠️ N-22 | visual ✓, semantics unprobeable |
| ui-tabs--states | 🔴 N-08 | 7 tabs 30px@13 ✓, aria-selected ✓; strip overhang 10px |
| ui-combobox--states | ✅ thin | open list = 1 option only; 19.5px inner input normal |
| ui-{combobox,select,segmentedcontrol,tabs,slider,switch}--reduced-motion (6) | ✅ | withMotion=0 across 20/26/56/27/45/27 elements; Select RM auto-opens (→N-15) |
| ui-overlays-dialog--light-dark | 🔴 N-07, N-16 | opened manually: 342×362, r10, centered 0/0, title 15/desc 13, Close 30 |
| ui-overlays-dialog--reduced-motion | ⚠️ N-09 | 1 of 18 nodes still animated (rf-scale-fade) |
| ui-overlays-menu--light-dark | ✅ geometry / 🔴 N-04 | 386px w, pad 12, r10, itemH 30@13 — contract exact; both menus dark |
| ui-overlays-menu--reduced-motion | ✅ | (auto-opens) |
| ui-overlays-popover--light-dark | ⚠️ N-16 | opened: 362×159, r10, blur 14, scrollX island ✓ |
| ui-overlays-popover--narrow-sheet @360 | ✅ | sheet 336w, margins 12/12/12, bottom-anchored |
| ui-overlays-sheet--light-dark | ✅ | 1256w, 12/12/12, h342.5<50dvh cap, Close pinned OUTSIDE scroll ✓ |
| ui-overlays-sheet--narrow @360 | ✅ | 336w 12/12/12, h362≤390 clamp; content never forces the promised scroll |
| ui-overlays-tooltip--light-dark | ✅ | bubble wraps ~190px < 280 max; a11y-node probe caveat |
| ui-modelselector--popover-grouped | ✅ | scrollOwners=1; search pinned; truncation-before-badge-wrap ✓; rows 48/53, names 24, pricing 20 |
| ui-modelselector--inline-with-unset | ✅ | scrollOwners=1; unset row; Add-new pinned bottom |
| ui-modelselector--custom-unset-label | ✅ | default label absent ⇒ prop works |
| ui-modelselector--disabled-rows-and-all-badges | ✅ | all 6 badges; disabled=muted (aria unverified) |
| ui-modelselector--panel-less-single-scroll | ✅ | scrollOwners=1, borderedRows=0 |
| ui-modelselector--narrow-popover-sheet @360 | ✅ | sheet 336/12/12/12, scrollOwners=1 |
| ui-modelselector--light-and-dark | ✅ | light half PAINTED correctly (correct-helper family) |
| ui-datatable--wide | ✅ (+N-20, N-21) | header↔cell drift [0,0,0,0,0]; Latency right-aligned ✓ |
| ui-datatable--narrow-stacked @360 | 🔴 N-10 | pageHScroll=true |
| ui-datatable--light-dark | ✅ | wrapping ✓, sort indicators ✓; census fully on-scale |
| ui-editabletable--add-remove-enter-validation | ⚠️ N-11 | validation UI ✓, JSON preview ✓ |
| ui-editabletable--light-dark | 🔴 N-03 | lightPainted=false |
| ui-toolcard--states | ✅ | 6 states; all toggles 26px; census clean |
| ui-toolcard--light-and-dark | ✅✅ | light half painted correctly — reference implementation for N-03 |
| ui-virtuallist--large-list | 🔴 N-05 | blank render |
| ui-virtuallist--light-dark | 🔴 N-03/N-05 | |
| ui-virtualizedgrid--responsive-grid | ✅ | 3 cols @ 153/477/801, w312, gutters exactly 12 |
| ui-virtualizedgrid--virtualized | ✅ | 30 DOM items, windowing works |
| ui-virtualizedgrid--single-column | ✅ | collapses to 1 col |
| accordion--primary | ✅ | rows 36px, chevrons aligned, content indented flush |
| callout--default | ✅ (+L-12, N-06) | h42, pad 8×12, r10 |
| error-callout--default | ✅ (+L-11) | renders, no Redux crash (old S2-10 crash fixed); danger callout with retry hint |

## Part 3b — Session-4 full-restart per-story log (fresh rig: DevTools-style layout overlay + size labels + 8/32 grid, 1280×900@2x full-frame)

| # | Story | Verdict | Notes |
|---|---|---|---|
| 1 | accordion--primary | ✅ | rows 36, aligned triggers/chevrons; story-chrome pT48 only |
| 2 | chat-composer-threadinfobutton--open | ✅ | popover 362×172 r10 pad12 overlay-bg blur14; value rows 26, copy btns 28×28 aligned; fonts 12/13 |
| 3 | callout--default | ⚠️ N-24 | h42 r10 pad ok; 16px ambient font |
| 4 | error-callout--default | ⚠️ N-24 | h56 icon centered; retry hint 12px ✓ |
| 5 | chat--primary | 🔴 N-25 · N-27 | bubble 11px; composer dock pad 12/20 = intentional (N-26 retracted); textarea 31 auto-grow ok |
| 6 | chat--configuration | 🔴 L-02(34px) · N-28 | Set-a-goal 34px; path 13.3px mono; Read-rows aligned ✓ |
| 7 | chat--ide | ✅ | no workspace chrome in IDE host ✓; same N-25 recurrence |
| 8 | chat--knowledge | 🔴 N-29 · N-30 · N-31? | hunk btns 21px ×10; triggers 29px; bold path overflow needs re-verify |
| 9 | chat--empty-space-at-bottom | ✅ | clearance fixture renders as designed |
| 10 | chat--user-message-empty-space-at-bottom | ✅ | shield gapvar = space-between (benign) |
| 11 | chat--compress-button | ✅ | auto-compression toggle 28×28 ✓; "Compress or Handoff" 0×0 closed-menu artifact |
| 12 | chat-content--primary | ✅ | single message + mascot (N-25) |
| 13 | chat-content--with-functions | ✅ | ToolCards render; N-28/N-30 recurrences; S2-10 fixture rot FIXED |
| 14 | chat-content--with-diffs | 🔴 N-29(16px) | diff header pills 16px; blocks left-aligned ✓ |
| 15 | chat-content--notes | ⚠️ N-30 | 26 vs 29 row species adjacent; copy 28×28 ✓ |
| 16 | chat-content--with-diff-actions | ✅ | pill 89×16 (N-29); actions bar 30 ✓; gutters aligned |
| 17 | chat-content--large-diff | ✅ | 11× hunk btns 21px (N-29); diff columns rule-straight ✓ |
| 18 | chat-content--empty | ✅ | mascot-only; N-25 |
| 19 | chat-content--assistant-markdown | ⚠️ N-32 | KaTeX exempt-clean; inline-image triggers 76/83×16 |
| 20 | chat-content--tool-images | 🔴 N-33 | 0 `<img>` in DOM; empty preview band; `_url_:13.3` |
| 21 | chat-content--multi-modal | 🔴 N-33 | 2 "1 screenshot" rows, imgs:0 |
| 22 | chat-content--integration-chat | ✅ | swept |
| 23 | chat-content--text-doc | ✅ | content column 171→1273 in 1462 root — gutters 171/189 symmetric; earlier "right-edge clip" suspicion RETRACTED (clip-rect artifact) |
| 24 | chat-content--markdown-issue | 🔴 N-29 | `_showMoreButton_` 22px ×3; diff gutter row-drift benign (top-aligned) |
| 25 | chat-content--tool-waiting | ✅ | Inspect 64×26, tool row 1102×26, align [] , off.h [] |
| 26 | chat-content--with-queued-messages | ✅ | queued chips right-edge 1135.5 both, h 20/20; tetris loader em-sprite (1.75px radius EXEMPT, retracted) |
| 27 | chat-content-plan-banner--plan-v-1 | 🔴 N-34 | toggle 1054.4×16 pad 0; History 22×22 r3; icon 13; centers 37.0 ✓ lefts 74.5 ✓ |
| 28 | chat-content-plan-banner--plan-with-deltas | 🔴 N-35 N-36 N-37 | hr 1126 vs siblings 1054.4 (71.6px overhang), #808080, margin 7; H2 15/700 vs H3 14/700; reselect warning |
| 29 | tool-cards-agentic--subagent | ✅ | body rule lands exactly on title x=103.5; header+button width == body 1174.5; badge 24 (S3-22 family) |
| 30 | tool-cards-agentic--set-tasks | ✅ | Tasks row 1102×26, off.h [] |
| 31 | tool-cards-agentic--patch-with-diff | 🔴 N-29 | hunk btns 143×21 + 185×21; line-number/marker/code columns align across hunks |
| 32 | **ui-tabs--states** | 🔴 **N-08 RE-VERIFIED** | **tablist 61→711 (650) vs tabpanel 61→701 (640) = 10px right overhang**; tabs 213.3×30 gaps 0,0; indicator w == active tab ✓ |
| 33 | ui-segmentedcontrol--states | 🔴 N-38 | items 33.3/40.6 @16px vs Tabs 30 @13px; radiogroup+label vs button[role=tab]; group width 640 == column ✓ (proves N-08 is a bug) |
| 34 | ui-overlays-dialog--light-dark | 🔴 N-03 (mechanism proven) | light panel keeps `color-scheme: dark`, partial token flip → desc `rgba(0,0,0,.55)` + btn `rgba(0,0,0,.88)` on near-black = ~1:1; triggers 640×65.5 |
| 35 | ui-overlays-popover--light-dark | 🔴 N-03 | identical fingerprint to #34 ⇒ shared wrapper `_panel_1psvz_11` |
| 36 | ui-icon--sizes-and-tones | 🔴 N-03 (3rd confirmation) | **kit icon scale established = 13/15/18** (28/14/14 instances); fonts 14-400,13-650,19-700 ✓; gaps 8/16/4/12/22 ✓; pad 22/32 ✓; light half illegible |

## Part 4 — Verified clean (do not re-litigate without new cause)

Surface & Card galleries · StatusDot · Skeleton · EmptyState · ErrorState · Overview workbench · Switch geometry (36×20/16 exact) · Menu overlay geometry (386/12/10/30@13) · popover→Sheet responsive contract @360 (12/12/12 margins; old S4-7 asymmetry definitively closed) · Sheet pinned footer · Tooltip clamp · ModelSelector single-scroll-owner + truncation contracts (all modes) · DataTable column alignment + numeric alignment · VirtualizedGrid geometry + windowing · ToolCard shell (26px toggles, on-scale census, correct light story) · reduced-motion form sextet (0 animated nodes) · Accordion.

## Part 4b — SESSION-4 HANDOFF (read this first when resuming)

**Progress this session: 36 / 171 stories at full rigor.** Restarted from story 1 as instructed; prior session-3 coverage was NOT reused.

### Rig (re-create it exactly — init scripts die with the browser context)
Storybook: `cd refact-agent/gui && npm run storybook -- --ci` → **:6006**. It has OOM'd once mid-sweep; just restart it.
Viewport `set_viewport 1280x900 device_scale_factor 2`; screenshots `scale:"device"` **with `clip {x:0,y:0,width:1280,height:<=900}`** (a clip equal to the viewport = full frame; WITHOUT a clip the shot includes dead black window area and the page renders half-size).
Four init scripts, added via `add_init_script` before navigating:
1. **console filter** — swallows `[MSW]` / React-DevTools noise.
2. **WebSocket shim** — returns a dead stub for `storybook-server-channel`. **Do not skip this:** without it every navigation dumps the entire story fixture (the whole `storyPrepared` frame) into the tool result and burns ~20k tokens per story.
3. **`window.__a()`** — alignment detector (row-centre drift / gap variance) + size census vs H[20,24,26,28,30,36] F[12,13,14,15,19] R[4,6,8,10,pill] P[0,2,4,6,8,12,16,22,32]; returns `{align, off:{h,f,r,p}, ov, docW, vw}`.
4. **`window.__L()`** — DevTools-Layout-style overlay: green padding boxes, purple dashed flex containers + hatched gap fills, blue element outlines, red interactive outlines with `w x h` labels, plus the 8px magenta / 32px cyan grid.
5. **`window.__c(name)`** — cross-component consistency fingerprint (control heights, radii, font+weight, panel backgrounds incl. backdrop-filter, borders, shadows, icon sizes, gaps, paddings). **Compare fingerprints ACROSS stories** — identical fingerprints proved the light/dark harness is one shared wrapper.
Per story: navigate → `wait_for_function` on `textContent.length>20 && performance.now()>2500` → `eval __a()+'|'+__L()` → screenshot → comment → append findings here.

### Established reference facts (do not re-derive)
- Kit **icon scale = 13 / 15 / 18**. 13px icons are legitimate `sm`.
- Kit **control ladder = 26 / 28(icon-sm) / 30 / 36**; type scale **12/13/14/15/19**; radius **4/6/8/10/pill**; spacing **2/4/6/8/12/16/22/32**.
- ToolCard contract: title sits **31px** off the card edge; the body's hairline rule must land on the title x.
- `rt-Flex:B48` and `rt-Container p48` are **story chrome**, not app padding — ignore.
- The bottom strip with red "0" badges in screenshots is the **engine-injected browser toolbar**, not app UI.
- LogoAnimation dots (`0.24em`/`0.05em`) and the mascot sprite are **em-scaled sprite geometry — exempt** from the radius/size scales.
- Diff gutters are intentionally **top-aligned**, so `__a()` row-centre drift on `_lineContent_` is benign.

### Queue for the next session (in priority order)
1. **Remaining `ui-overlays-*` light-dark + reduced-motion** (menu, sheet, tooltip) — expect the same N-03 wrapper; confirm and stop re-filing.
2. **Composer land**: `chat-form--*` (4), `chat-composer-chatsettingsdropdown--open`, `chat-composer-modeselect--trigger|open`, `chat-form-retryform--*` (2) — highest user-visible value, and RetryForm carries known S1-3/S1-4 history.
3. **Dialog land**: `chat-dialogs-modetransitiondialog--*`, `chat-dialogs-taskplannerdialog--*`, `toolconfirmation--*` (4), `features-providers-addcustommodelmodal--open`, `features-worktrees-createworktreemodal--*`, `integrations-mcpimportdialog--open-with-pasted-json`, `components-deletepopover--*` (3).
4. **Kit galleries**: `ui-button--variants-sizes-states`, `design-system-{badge,card,chip,surface,statusdot,skeleton,emptystate,errorstate,loadingstate}--gallery`, `ui-field--*` (3), `ui-datatable--*` (3), `ui-editabletable--*` (2), `ui-modelselector--*` (7), `ui-{select,slider,switch,combobox,virtuallist,virtualizedgrid,toolcard}--*`.
5. **Remaining tool cards / transcript**: `tool-cards-file-ops-basics--*` (10), `tool-cards-agentic--{delegate,finish,sleep-ask,chrome,engine-analysis}`, `chat-transcript-elements--*` (10).
6. **Leftovers**: `task-progress-widget--*` (6), `usagecounter--*` (3), `features-checkpoints--*` (3), `login--primary`, `privacy-chat-shield--default`, `scroll-area*`, `logo-animation--*`, `accordion`, `checkbox`, `collapsible`, `combobox-v2`, `reveal`, `select`, `spinner`, `textarea`, `components-chatlinks`, `components-text-animated`, `chat-composer-trajectorybutton--*`.
7. **Not yet done at all: the 360x780 narrow pass and the light-theme pass** for everything above (blocked on N-03 for the kit light stories, but app stories can use the Storybook `appearance` global).

### Session-4 rule reminders
Append new findings as sequential `N-###`; flip Status to `fixed@<commit>`; never delete a row — retractions get struck through with the reason (see N-26, and the icon sub-claim in N-34).

## Part 5 — Remaining sweep queue (~126)

Chat land (priority): `chat-form` ×4, `chat-form-retryform` ×2, `toolconfirmation` ×4, `task-progress-widget` ×6, composer popovers (`chat-composer-*` ThreadInfo/ChatSettings/ModeSelect×2/Trajectory×2) ×6, dialogs (`chat-dialogs-*` ×4, `components-deletepopover` ×3, `integrations-mcpimportdialog` ×1), `chat-content-plan-banner` ×2, transcript `chat-transcript-elements` ×10, tool cards `tool-cards-*` ×18, `chat-content` ×15, `chat` ×7.
Components: `components-chatlinks`, `checkbox`, `collapsible`, `combobox-v2`, `select` ×2, `spinner`, `textarea`, `reveal`, `scroll-area` ×4, `logo-animation` ×3, `components-text-animated`, `usagecounter` ×3.
Features: `features-checkpoints` ×3, `login`, `privacy-chat-shield`, `features-providers-addcustommodelmodal`, `features-worktrees-createworktreemodal` ×2.

Per-story protocol for continuation: navigate → wait `#storybook-root` content → alignment detector + size census evals → gridline overlay → 2× screenshot → forced-open for overlay/popover stories → 360px pass for responsive stories → per-story report with numbers.

## Part 6 — Audit-infra fixes applied (session 3)

1. **`.storybook/preview.tsx`** — `onUnhandledRequest` bypass rewritten from hardcoded `http://localhost:6006/src/` prefix to origin-relative check: same-origin requests warn only for API-shaped paths (`/v1/`, `/p/`); all dev-server asset fetches bypass silently on any port. (N-19)
2. **`src/components/Callout/Callout.tsx`** — `useAppSelector` now imported from its leaf module instead of the ~70-export `hooks` barrel, cutting the app-wide import cascade for Callout/ErrorCallout stories and every Callout consumer. (N-06 partial; full decoupling of ErrorCallout from the store remains L-11.)

## Appendix — Methodology probes (for continuation)

- **Alignment detector:** group visible non-svg elements by parent; same-row (tops within 14px): flag vertical-center spread >2.5px and, for ≥3 children, gap spread >3px; stacked (≥3): flag left-edge spread >2.5px (<80px).
- **Size census:** interactive elements (button/input/select/[role=switch|tab]) heights vs {20,24,26,28,30,36} ±0.6; text nodes' font-size vs {12,13,14,15,19} ±0.26; boxed elements' border-radius vs {4,6,8,10,999}; each padding component vs {0,2,4,6,8,12,16,22,32}; report offender class names.
- **Gridlines:** fixed overlay div, repeating-linear-gradients 8px magenta + 32px cyan; red 1px outlines on interactives; inject before screenshots (`scale: device` at 2× DPR).
- **Overlays:** click triggers yourself (many stories are trigger-only); measure `[role=dialog]` / popper-wrapper children (NOT `[role=tooltip]` — that's a 1×1 a11y node); check margins vs 12/12/12, radius 10, single scroll owner, pinned chrome.
- **Matrix galleries:** map grid children to columns via left-offset vs `gridTemplateColumns`; print the cell-text matrix — heights-only audits pass scrambled layouts.
- **Reduced motion:** count nodes with non-zero computed `transition-duration` or running `animation`; forms should be 0; overlays currently leak 1 (N-09).

### Session-4 continued story log (stories 37-53, full rigor: 1280x900 @2x + 8/32 grid overlay + size/type/radius census + cross-component fingerprint)

| # | story | verdict | key measurements |
|---|---|---|---|
| 37 | `chat-form--primary` | **PASS (reference)** | ALL controls share `cy=76.0` exactly (26px pills + 28px icon buttons). Textarea 23->1242 with `4px 12px` pad => text box 35->1230; first pill left edge **35**, Send right edge **1230** — pixel-perfect bracketing. Icon cluster pitch 36 (28+8), deliberate 16px group break. Panel glass `rgba(20,20,22,.82)`+blur14 ✓ |
| 38 | `chat-form--with-attached-images` | **FAIL — N-40** | byte-identical to `--primary`; `img` count 0, no tray |
| 39 | `chat-composer-modeselect--open` | PASS | popover 360x390.1 r10 overlay glass ✓ shadow `0 14px 40px` ✓; 3 mode cards all **334x91.7** (improves old S4-5); Create-new-mode 334x30; chips r6 `2px 6px`; Wave-3 12px pad holding. Evidence for N-39 (13/650 + 13/600 in one popover) |
| 40 | `chat-composer-chatsettingsdropdown--open` | 2 findings | popover 442x597 r10 ✓; icons **13x13 x47 uniform** ✓; Token-limits disclosure 416x26 ✓; **N-41** search input 377x21 pad 0; **N-42** 19 rows at 59 + 1 at 62 (Default badge row) |
| 41 | `chat-form-retryform--text-only` | PASS + L-17 | **S1-3 confirmed FIXED**: textarea `padding:12px`, `min-height:100px`, renders 1219x102 @14px. L-17 re-measured: Cancel 76.3x26@13 / gpt-4o 93.1x30@14 / Add image 100.5x26@13 / Submit 78.8x26@13, `cySpread=0` |
| 42 | `toolconfirmation--default` | 2 findings | census fully clean; panel-pad now **12** (Wave-4 fix ✓); **N-43** muted footnote 15px > title 14px; **N-44** badge renders 32px |
| 43 | `design-system-badge--gallery` | **FAIL — N-44** | 7/7 badges render **24px** from `min-height:18` + `2px 6px` + border under content-box; gallery only covers `size-sm` |
| 44 | `design-system-chip--gallery` | **FAIL — N-45** | 5 chips 28px + 1 chip 23px; `min-height:auto`; 18x18 remove targets |
| 45 | `ui-select--states` | **FAIL — N-46** | identical triggers render 30 / 30 / **56.625** px; grid parent stretches the third |
| 46 | `ui-switch--states` | **PASS (exemplary)** | 4 switches all **36x20 r999 border-box** ✓, row shares `cy=195.7`; census fully clean. One of only 4 kit components declaring `border-box` |
| 47 | `ui-datatable--narrow-stacked` | PASS | 20 cells uniform `8px 12px`; headers uniform 19.5. Row heights 56/75.5/153.5 are content-driven. **2 self-retractions** (row 3 not invisible: `color rgba(255,255,255,.92)`, `opacity 1`; the 4 zero-height rows are Storybook's own `sb-argstableBlock`) |
| 48 | `design-system-statusdot--gallery` | **PASS (exemplary)** | 9 dots all **8x8 r999**, row shares `cy=94.3`; all 5 colours are real tokens. Observation: 9 states map onto 5 colours (running/in_progress, warning/needs_attention, success/completed, idle/paused are visually identical) |
| 49 | `design-system-card--gallery` | **FAIL — N-48** | cards 582.5x81.6 aligned, 12px gap ✓ pad 12 ✓ r10 ✓; card titles render **18.72px UA default** beside a tokenized 19px heading |
| 50 | `ui-overlays-menu--light-dark` | PASS + N-04 | both Menu overlays identical 386x189 pad12 r10 ✓; all 8 items exactly 30px ✓; light panel keeps `color-scheme:dark` => both halves dark; helper ships 640x65.5 buttons |
| 51 | `ui-toolcard--states` | **PASS (best in kit)** | 5 cards all 1143x145.5, collapsed 1143x42; **all 12 controls 26px**; panel-less `rgba(0,0,0,0)` pad 0 r10 ✓; documented 31px body offset ✓ |
| 52 | `tool-cards-file-ops-basics--many-tools-grouped` | **FAIL — N-49** | 4 tool rows uniform 1102x26 ✓; `_path_`/`_query_` render **13.3px** from `0.95em` |
| 53 | `task-progress-widget--active-goal` | **PASS (exemplary)** | 3 budget inputs all **387.7x30**, `cy=237.8` identical, gaps exactly **8 / 8**, and each label left edge matches its input left edge exactly (43 / 438.7 / 834.3). **Self-retraction**: suspected overlap was my overlay-label misread |

**Session-4 self-retractions (kept for honesty):** DataTable "invisible row 3" (computed styles prove it is visible); DataTable "4 zero-height rows" (Storybook's own args table); TaskProgressWidget "overlapping budget inputs" (gaps are exactly 8/8).

### Session-4 continued story log (stories 54-58)

| # | story | verdict | key measurements |
|---|---|---|---|
| 54 | `ui-editabletable--add-remove-enter-validation` | **FAIL — N-50** | row 1 clean (both cells 30px, `cy=152.5`); row 2 Name `262.4x30 cy=199.5` vs Description **`381.6x52 cy=210.5`** — 11px baseline drift caused by a validation message stretching the grid row |
| 55 | `ui-modelselector--popover-grouped` | **FAIL — N-41 root cause** | story ships CLOSED (no `play()`), so the popover it is named after has zero coverage until opened manually. Opened: popover **422x522** r10 overlay glass ✓ shadow ✓, chips uniform `2px 6px` r6 x15 ✓. Search field **357x24 @16px** vs **377x21 @14px** in the app host — same class, two sizes |
| 56 | `chat-form--primary` **@360x780** | **PASS** | `scrollWidth 345 == clientWidth 345` (no h-scroll) ✓; glass `rgba(20,20,22,.82)`+blur14 preserved ✓; hide-ladder collapses 8 icon buttons + 3 pills down to textarea `299x31` + one 28px button ✓ |
| 57 | `ui-tabs--states` **@360x780** | **N-08 CONFIRMED** | tablist `61->294` vs tabpanel `61->284`: `deltaLeft=0`, **`deltaRight=10`** — identical 10px at 360 AND 1280 => structural, not responsive. 4.5% of panel width at 360 vs 1.6% at desktop |
| 58 | `ui-overlays-popover--narrow-sheet` **@360x780** | **PASS** (closes S4-7) | converts to bottom Sheet: **336x159**, margins **12 / 12 / 12** symmetric, 93.3% of viewport, r10, overlay glass ✓ |
| 59 | `chat-transcript-elements--compression-cards` | **PASS** | census fully clean; 5 metric labels 12/400 + 5 values 14/650; action icons 28x28 x3; Inspect 64x26 |

| 60 | `ui-slider--states` | **PASS** | 3 tracks all `640x8` r999 `rgba(255,255,255,.043)`; ranges 6px accent (2px inset); thumbs all `20x20` r999; per-row `cy` exact (175.5/245/314.5); census clean. Thumb feeds N-51 |
| 61 | `design-system-surface--gallery` | **FAIL — N-52** | 6 surfaces all `186.2x114`, uniform **12px** gaps, r10, pad12 ✓, correct plain->1/2/3->overlay->selected recipe ✓ — but surface-1 vs surface-2 differ by 0.008 alpha (~1.03:1, imperceptible), and all 7 labels render at UA **16px** |

---

## Part 4b - SESSION-4 HANDOFF (read this first on restart)

### Where the sweep stands
**61 of 171 stories** audited at full rigor this session (progress was reset to 0 by user instruction; the earlier 45-story pass is superseded and its "clean" verdicts are NOT trusted).

### Findings added this session
`N-39` .. `N-52`, plus substantial upgrades to `N-03`, `N-04`, `N-08`, `N-41`, `N-44`.

**The three that matter most:**
1. **N-44 / N-47 — the height-strategy root cause.** No global `box-sizing: border-box` reset exists (67 hand-written per-element declarations instead; only 4 of ~24 kit components opt in). Combined with four different height mechanisms across the kit, this single cause explains nearly every "wrong size" symptom in the whole ledger. `ui/Button` is the one correct pattern (fixed `height` + zero vertical padding) and is measurably immune.
2. **N-08 — the user's original tab complaint, now proven structural.** Exactly 10px overhang at both 1280 and 360 => `2 x (4px list padding + 1px border)`. `SegmentedControl` gets the same layout right (640 = panel width), which proves it is a bug and not a design choice.
3. **N-03 — light/dark token leakage.** Reproduced on 5 independent surfaces. Root cause identified in `styles/tokens.css`: the periwinkle palette (l.19-28) and the Radix-blue palette (l.227-232) are the dark/light variants of the SAME token names, so any surface resolving the wrong scope silently swaps its entire colour system. Worst measured case: an input border at **contrast ratio 1.00**.

### Rig reconstruction (init scripts are LOST on every browser context reset)
Re-add via `add_init_script` before sweeping:
- `window.__a()` - alignment + off-scale census. Returns `{align, off:{h,f,r,p}, ov, docW, vw}`.
- `window.__c(name)` - cross-component fingerprint: `ctrlH, radius, font, bg, border, shadow, icon, gap, pad`.
- `window.__L()` - DevTools-style overlay: 8px minor / 32px major grid + per-element size labels.
- **WebSocket shim stubbing `ws://.../storybook-server-channel`** - without it every navigation dumps ~20k tokens of story fixture into the tool output.
- Console-noise filter (MSW logging still leaks through; two filter attempts failed - low priority).

### Reference scales (verified against `styles/tokens.css` this session)
- control heights **26 / 30 / 36** (+ `--rf-control-h-icon-sm: 28` documented tap floor)
- type **12 / 13 / 14 / 15 / 19** (`--rf-text-1/2/3` = 12/13/14)
- icons **13 / 15 / 18**; radii **6 / 8 / 10 / 999**; spacing **2 / 4 / 6 / 8 / 12 / 16 / 22 / 32**
- overlay glass `rgba(28,28,31,.94)` + `blur(14px)` + `0 14px 40px rgba(0,0,0,.4)`; panel glass `rgba(20,20,22,.82)` + `blur(14px)`
- accent `#7f93d8` (dark) / `#006adc` (light) - both legitimate, see N-03

### Method notes that earned their keep
- **Always open interactive surfaces.** `ui-modelselector--popover-grouped` and `ui-select--states` only revealed defects after a manual click; several stories ship closed with no `play()`.
- **Compare siblings, not just absolutes.** N-50's 11px drift and N-46's 56.6px stretch both passed the off-scale detector and were caught only by comparing neighbours.
- **Verify suspicions with computed styles before filing.** 4 self-retractions this session came from trusting the screenshot over the DOM.

### Queue for next session (110 stories remain)
1. Finish kit: `surface`, `emptystate`, `errorstate`, `loadingstate`, `skeleton`, `combobox` x2, `slider` x2, `tooltip` x2, `sheet` x3, `dialog` x2, `virtuallist` x2, `virtualizedgrid` x3, remaining `modelselector` x6, `toolcard--light-and-dark`, `design-system-overview`.
2. Product: remaining `tool-cards-*` (16), `chat-transcript-elements` (9), `task-progress-widget` (5), `chat-dialogs-*`, `privacy-chat-shield`.
3. **Entire light-theme pass still outstanding** (blocked in practice by N-03 - fix that first or results are meaningless).
4. Reduced-motion stories (7) - check computed `transition-duration`/`animation`, not appearance.

---

## Part 4c — SESSION-5 LOG (fresh rig, portal-aware) — stories 62-70

### ⚠️ RIG DEFECT FOUND AND FIXED — previous sessions' overlay verdicts are suspect

The session-4 probes (`__a`, `__c`, `__L`) all rooted their element queries at
`#storybook-root` / `#root`. **Radix portals every overlay to `document.body`**, so
for every dialog, popover, menu, sheet, tooltip and select-content story those probes
were measuring *the trigger page only* and silently reporting the overlay as absent.
First caught at `chat-dialogs-modetransitiondialog--switch-mode`, where `__L()` returned
`0interactives` and `__c()` returned empty `ctrlH`/`font`/`radius` arrays on a story whose
entire content is a dialog.

**Fix applied this session:** new shared `window.__els()` roots at `body *` and excludes
(a) the audit overlay itself, (b) `#__refact_toolbar_host` (engine-injected browser toolbar),
(c) sub-1px boxes and hidden nodes. `__a`/`__c`/`__L` now all consume it. Verified: the same
story went from `0interactives` to `2inter/22els` with the dialog fully instrumented.

**Consequence for the ledger:** any *clean* verdict recorded in session 4 for an overlay story
was produced by a probe that could not see the overlay. Overlay stories marked ✅ before
this fix should be re-measured before being trusted. Findings *filed* from those stories
remain valid (they came from targeted `[role=dialog]` queries), only the "nothing else found"
half is unreliable.

Second detector caveat recorded: `__a`'s stacked-left-edge branch flagged 12 false positives on
flex-**row** buttons at `ui-button--variants-sizes-states` (icon+label children). The branch now
requires the parent to be `flex-direction: column` or `display: block`.

### Story log

| # | story | verdict | key measurements |
|---|---|---|---|
| 62 | `ui-button--variants-sizes-states` | 🔴 **N-01 proven** | 4-col grid `226.3/282.9/282.9/282.9`, gap 12, row-major flow of *4 headers -> 5 labels -> 15 buttons*. Cell map: row1 = all five variant LABELS spread across the sm/md/lg columns; row2 = label `Plain` + the three **ghost** buttons; rows 3-5 each offset one further cell, so the size headers read 30/36/26, then 36/26/30, then 26/30/36. Only the ghost row lands under correct size headers — and it is captioned "Plain". Three buttons render inside the 226px label column. **Census fully clean** (`off.h/f/r/p` all empty across 59 interactives; heights exactly 26/28/30/36) — the exact scenario the Appendix warns about. **N-13 mechanism confirmed**: icon row `cy` spread = **4.0px** = (36-28)/2, the arithmetic signature of edge-alignment, not rounding |
| 63 | `toolconfirmation--mixed` | 🔴 **N-53** | census clean; panel glass `rgba(20,20,22,.82)+blur(14)` ✓ gaps 16/12/8/4 ✓ pad 22/12 ✓ icons 13 ✓ Continue 74x**26** ✓. Full text census: title 14/650, body 14/400 x3, commands 12/400, **footnote 15/400 @ 48% opacity** (N-43 2nd repro — largest text in the panel is the muted one). Badges **32px** (N-44 3rd repro). Chips `rgb(216,115,109)` danger vs `rgb(205,160,78)` warning = the only verdict signal |
| 64 | `chat-dialogs-modetransitiondialog--switch-mode` | ⚠️ **N-54** | 502x167 @ **dx=0 dy=0**; overlay glass + `0 14px 40px rgba(0,0,0,.4)` + backdrop `rgba(0,0,0,.55)` ✓ r10 ✓ gaps 12/8/4 ✓ pad 22/16/`2 6`/`0 12` ✓; Cancel **70.5x30**@680.3 / Switch Mode **111.2x30**@762.8, same `cy=501.5`, gap **12**; census clean. Title row renders `agent -> Ask`. Weights 15/**600** title vs 14/**650** buttons vs 12/**650** chips = N-39 at maximum density |
| 65 | `chat-dialogs-modetransitiondialog--restart-current-mode` | ✅ PASS | box **identical** 502x167 @(389,366.5); inner pad 16; content left edge computes 389+1+16 = **406.0** and title + description both measure exactly 406.0; actions right-align to 874 = 390+500-16. Cancel 70x30 / Restart Mode 117x30, gap 12. Chip renders **`Agent`** (correct display name) — corroborates N-54 as a missing-prop, not a call-site error |
| 66 | `chat-dialogs-taskplannerdialog--create-new-task` | ⚠️ N-54 | 502x167 @(389,366.5) again; title + desc both x=**406.0**; Cancel 70.5x30@687.1 / Create Task 104.4x30@769.6, gap **12**; census clean. Chip = raw **`task_planner`** at 12/650, h=**24** (N-44 4th repro) |
| 67 | `chat-dialogs-taskplannerdialog--add-planner-to-task` | ✅ PASS | 4th consecutive 502x167 @(389,366.5); title "New Planner" 15/600 @406.0; Cancel 70.5x30@665.3 / Create Planner 126.2x30@747.8, gap **12**; chip `task_planner` h=24 |
| 68 | `components-deletepopover--open` | 🔴 **N-55, N-56** | popover 362x122, r10, `rgba(28,28,31,.94)`+blur14, shadow `0 14px 40px` ✓; inner inset computes 12+1+12 = **25.0** and title/desc/first-button all measure x=25.0; both buttons 30px, same `cy=154.0`. **`Delete@25.0` LEFT / `Cancel@104.2` RIGHT** (N-55) with gap **8** vs the dialogs' 12 (N-56). Title **15/700** vs dialog titles **15/600** (N-39 head-to-head) |
| 69 | `components-deletepopover--small` | ✅ PASS | trigger **28x28** = `--rf-control-h-icon-sm` floor exactly; popover body byte-identical to `--open` (size prop scopes to trigger only — correct for a destructive prompt); buttons r8 + `padding: 0 12px` = the N-47-immune Button recipe; census clean |
| 70 | `components-deletepopover--closed` | ✅ PASS | trigger **30x30** r8, icon svg **15x15** (kit `md` rung), `openPopover:false` ✓, icon centred on both axes |

### Tracked, not yet filed
Dialog title/primary-action voice is drifting: "Switch Mode" / "Restart Mode" / "Switch to Task
Planner" / "New Planner" (three verb-phrases, one bare noun-phrase), and the primary button echoes
the title verbatim in 2 of 4. Holding until CreateWorktree / AddCustomModel / MCPImport are measured,
then file one consolidated copy-consistency finding if the pattern holds.

**Session-5 running total: 70 / 171 stories.** New findings `N-53` .. `N-56`; one retraction
(ToolConfirmation tone mapping is correct — verified against fixture + source).

### Session-5 story log continued — stories 71-76

| # | story | verdict | key measurements |
|---|---|---|---|
| 71 | `features-worktrees-createworktreemodal--open` | 🔴 **N-57** · S3-11 **verified fixed** | dialog 422x311; content inset 17 = 16 pad + 1 border; inputs **388x30** x2; label->control gap **4**; field->field gap **12**; buttons 30, gap **11.9~12**; census clean. Title 311.5->334, description starts **346.0** = **12px** gap (was 0px pre-Wave-3) ✓. Section gaps **24** and **28** from `margin-top` stacked on the Dialog's own `gap: 12px` |
| 72 | `features-worktrees-createworktreemodal--with-error` | 🔴 **N-58** | dialog grows 311 -> **341** = exactly +30 (18px line + 12px gap) ✓; `_errorText_` top **527.5** sits **12px** under Base-branch input (bottom 515.5) but **99.5px** under Branch-name input (bottom 428.0); both inputs keep neutral `rgba(255,255,255,0.035)` — no invalid state; error and help both **12px/400**, differing only in colour |
| 73 | `task-progress-widget--paused-goal` | 🔴 **N-59** · N-54 broadened · **1 retraction** | `_header_` 1221x**34**, `_goalHeader_` 1195x**34**, pads 12 vs 8; 3 budget inputs **384x30** gaps **8/8**; 4 action buttons all **26**; chip `Paused` 12/650 amber. Raw ids in UI: `needs_work`, `goal_pursuit` x2, `nudge`, `checkpoint`. **RETRACTION:** suspected Edit/progress-text overlap — measured same-pass, progress right **1166.9** vs Edit left **1178.9** = **12px gap, no overlap**; the collision was my own overlay annotation label (identical trap to session 4's "overlapping budget inputs") |
| 74 | `task-progress-widget--budget-exhausted` | 🔴 **N-60** | pixel-identical to `--paused-goal` except the chip (`Budget exhausted` 12/650 **danger** vs `Paused` **warning**); all four controls same size, **none disabled**; progress `6/6 · 10000/10000` styled identically to `5/12 · 8450/24000` |
| 75 | `task-progress-widget--no-budget-limits` | ✅ **PASS — contract verified** | renders **"5 turns · 8450 tokens · No budget limits"** — exact match to the `gui/AGENTS.md` rule that absent/null/0 limits show plain usage plus `No budget limits`, never a ratio ✓. Control correctly swaps **Pause 71.9x26** for Resume; chip `Active` accent. Observation (not filed): the 3 budget inputs are **384.3x30 with no placeholder** for values of 1-5 chars — `--rf-input-max` exists as an opt-in cap and is not used here; the "leave blank for unlimited" label already covers the semantics, so this is a judgement call, not a defect |
| 76 | `task-progress-widget--with-tasks` | ✅ PASS | task rows 3 x **1187x26** at 597.8/631.8/665.8 = pitch **34** (gap **8**) ✓ uniform; labels all at x=**60.0** (26px icon gutter); event rows 3 x **1169x18** pitch **22** (gap **4**) ✓. Observations (not filed): nested left edges 22/35/43 form a coherent ladder but the task list sits at **34**, 1px off the goal header's 35 — border-box arithmetic (1187 vs 1185 wide), **below the 2.5px detector threshold and below perceptual threshold**; and two list rhythms coexist (26/8 vs 18/4) for two different content classes |

**Not yet swept in this family:** `task-progress-widget--completed-goal` (the one remaining state).

---

## Part 4d — SESSION-5 HANDOFF (read with Part 4b)

### Where the sweep stands
**76 / 171 stories** at full rigor. Session 5 added stories **62-76** and findings **N-53 .. N-60**,
broadened **N-54**, corrected **N-56**, root-caused **L-02** (into N-59), and **verified S3-11 as fixed**.

### THE RIG DEFECT — most important carry-forward
Session 4's probes rooted at `#storybook-root`; **Radix portals every overlay to `document.body`**, so
every dialog/popover/menu/sheet/tooltip story was measured *without its overlay*. Fixed in session 5 via a
shared `window.__els()` (roots at `body *`, excludes the audit overlay, `#__refact_toolbar_host`, sub-1px and
hidden nodes). **Any "clean" verdict recorded for an overlay story before this fix is not trustworthy** —
findings filed from those stories stand (they used targeted `[role=dialog]` queries), but their
"nothing else found" half does not.

Also fixed: `__a`'s stacked-left-edge branch now requires the parent to be `flex-direction: column` or
`display: block` (it was producing false positives on every flex-**row** button with 3+ children).

### Two measurement rules this session earned
1. **Never judge overlap or collision from an overlay screenshot.** The red `WxH` annotation labels sit
   above-left of their element and routinely appear to collide with neighbouring text. Two false positives
   have now come from this (session 4 budget inputs, session 5 Edit/progress). Always re-measure with rects.
2. **Only same-pass measurements are comparable.** At `task-progress-widget--paused-goal` the Edit button
   measured `x=1168.9` then `x=1178.9` seconds apart (late content settling toggling the scrollbar;
   `docW 1265` vs `vw 1280`). Cross-pass absolute coordinates are unreliable; relative gaps within one
   eval are fine.

### Cross-cutting patterns now visible (worth a single fix each)
- **Raw identifiers in chrome** (N-54, broadened): `agent`, `task_planner`, `needs_work`, `goal_pursuit`,
  `nudge`, `checkpoint`. Two unrelated components. Needs one humanising lookup, not per-component props.
- **Colour as the sole information channel** (N-53, N-58): ToolConfirmation's confirm/deny verdict, and
  CreateWorktree's error-vs-help text (identical 12px/400, differing only in colour).
- **Wrapper spacing fighting container `gap`** (N-57, and the already-fixed S3-11): the same mechanism has
  now produced both a 0px gap and 24/28px gaps in the same modal.
- **`box-sizing`/height-strategy family** (N-44/N-47) continues to explain badge inflation everywhere
  (24px and 32px badges observed again at stories 63, 66, 67).

### Queue for next session (95 stories remain), highest value first
1. `task-progress-widget--completed-goal` (finish the family).
2. **Transcript land**: `chat-transcript-elements--*` (9) — the main app surface, entirely unswept this session.
3. **Tool cards**: `tool-cards-file-ops-basics--*` (9), `tool-cards-agentic-analysis--*` (5 remaining).
4. Remaining dialogs: `features-providers-addcustommodelmodal--open` (S2-9 unreachable-footer — re-verify,
   Wave 2 claimed a fix), `integrations-mcpimportdialog--open-with-pasted-json`, `toolconfirmation--patch`
   and `--with-denial`, `chat-composer-trajectorybutton--*` (2), `chat-composer-modeselect--trigger`,
   `chat-form--streaming` / `--with-queued-messages`, `chat-form-retryform--with-images`.
5. Kit leftovers: `ui-field--*` (3, blocked by N-03 for the light halves), `ui-modelselector--*` (6),
   `ui-datatable--wide|light-dark`, `ui-virtuallist--*` (2, N-05), `ui-virtualizedgrid--*` (3),
   `ui-overlays-sheet--*` (3), `ui-overlays-tooltip--*` (2), `ui-combobox--*` (2), galleries
   (`emptystate`, `errorstate`, `loadingstate`, `skeleton`, `overview`), small primitives
   (`checkbox`, `collapsible`, `combobox-v2`, `select` x2, `spinner`, `textarea`, `reveal`,
   `components-chatlinks`, `components-text-animated`, `logo-animation` x3, `scroll-area*` x4,
   `usagecounter` x3, `features-checkpoints` x3, `login`, `privacy-chat-shield`).
6. **Reduced-motion suite (8)** — probe computed `transition-duration`/`animation`, not appearance.
   **Now re-runnable against portaled overlays for the first time** thanks to the `__els()` fix, so
   N-09 (`.rf-popover-motion` keyframe surviving the RM helper) can finally be confirmed properly.
7. **Still entirely outstanding: the 360x780 narrow pass and the light-theme pass** for everything above.
   Light-theme results remain meaningless until N-03 is fixed.

### Environment
Storybook 7.6.20 dev server on **:6006**, launched from `refact-agent/gui` with `npm run storybook -- --ci`;
survived the whole session. Viewport 1280x900 @2x DPR throughout; screenshots use an explicit
`clip` equal to (or a sub-region of) the viewport. Init scripts are LOST on every browser context reset —
rebuild `__els`/`__a`/`__c`/`__L` plus the WebSocket shim and console filter from Part 4b + this section.
**Zero application source files were modified in this session; only `UI_AUDIT_REPORT.md` was touched.**

---

## Part 4e — SESSION-6 LOG (portal-aware rig, rebuilt) — stories 77-86

### Inventory reconciliation (correcting the running total)
Live `index.json` = **171 stories**, confirmed. Parsing every numbered row of the session-4/5 logs
yields **76 rows but only 74 unique story ids** — `chat-form--primary` (rows 37 and 56) and
`ui-tabs--states` (rows 32 and 57) were each measured twice, at 1280 and again at 360. Three ledger
ids also carry an abbreviated prefix (`tool-cards-agentic--subagent|set-tasks|patch-with-diff`); the
live index names them `tool-cards-agentic-analysis--*`. Corrected baseline entering this session:
**74 / 171 audited, 97 remaining.** Session 6 adds 15 → **89 / 171, 82 remaining.**

### Rig
Rebuilt exactly per Part 4d: viewport 1280x900 @2x, WebSocket shim + console filter, and the
portal-aware `__els()` (roots at `body *`, excludes `#__audit_overlay`, `#__refact_toolbar_host`,
sub-1px and hidden nodes) feeding `__a()` / `__c()` / `__L()`. Added `__p()` (one-call combined
census: off-scale h/f/r/p + alignment + control list + font/weight set + `docW` vs `vw` h-scroll flag)
and `__t(n)` (text census: content, size/weight, colour, x/y/width) to cut per-story round trips.

### Story log

| # | story | verdict | key measurements |
|---|---|---|---|
| 77 | `task-progress-widget--completed-goal` | 🔴 **N-61, N-62** · N-59 recurs | Controls **correctly** reduce to `Apply budget 102x26` + `Edit 43x26` — `GoalControls` returns `null` when all three flags are false. Chip `Completed` **`rgb(95,174,139)`** completes the tone ladder. `_header_30xgb_5` and `_goalHeader_30xgb_25` both **34px** again (5th host, N-59). Census otherwise clean (`f`/`r`/`p` all empty). Raw ids `needs_work`/`checkpoint`/`goal_pursuit` x2/`nudge`/`completed` (N-54). Observation, not filed: the budget editor and `Apply budget` stay enabled on a completed goal |
| 78 | `chat-transcript-elements--long-user-message` | ✅ **PASS** | census clean except story chrome (`rt-Flex:B48`); fonts 12/400, 13/650, 14/400 all on scale; controls 64x26 + 86x26 on ladder; `hscroll:false`. Message column measures a consistent **118 -> 1162** across `_userInputText_t4p2a_109`, `_markdown_1f8q8_5`, and the action button's left edge. **Self-retraction:** the overflow probe flagged `sw466 > cw435, overflow-x: visible` — chased it and it is Storybook's own `sb-main-fullscreen > div` (empty `textContent`, direct child of the preview root), **not app UI** |
| 79 | `chat-transcript-elements--compressed-user-message` | ⚠️ N-51, N-39 | 🗜️ compression hint renders ✓. `_reveal_button_169jc_1` = **912 x 21** — a new sub-floor class (5px under the 26px control min, 7px under the 28px tap floor), though generous on the long axis. "Click for more" is **exactly centred** on the collapsed text block (label 663->748 centre 705.5 vs block 250->1162 centre 706) — credit where due. New weight instance **13/500** (N-39) |
| 80 | `chat-transcript-elements--reasoning` | ⚠️ N-30 | `_trigger_1hjpa_40` **1102 x 29** (N-30, on no rung); "Thought" 14/700; fonts 12/400, 13/650, 14/400, 14/700 all on scale; n=56 |
| 81 | `chat-transcript-elements--thinking-blocks` | ⚠️ N-30 | **Structurally identical to #80** — same n=56, same control list, same off-scale set, same alignment flags; only the body copy differs. The two engine data paths (`reasoning_content` vs signed Anthropic `thinking_blocks`) converge on one presentation, which is correct behaviour but means the pair yields no differential coverage |
| 82 | `chat-transcript-elements--hidden-events` | ✅ **PASS — contract verified** | Probed `document.body.innerText` for `plan_delta`, `goal_delta`, `goal_pursuit`, `mode_switch`, `tick`, `summarization_marker` → **`LEAK=[]`**. Matches the `selectVisibleMessages` contract (excludes `event`/`goal`/`plan`). Census fully clean; n=45 |
| 83 | `chat-transcript-elements--context-files-attachment` | ✅ PASS | `_toggle_1e58f_26` **1102 x 26** ✓ on ladder; fonts all on scale. `_body_1e58f_97` `padding-left: 31` is **the documented ToolCard body offset**, not an off-scale defect — flagged by the census, cleared by the contract. Sub-threshold observation: "Read" (y=187) and the filename (y=189) are 2px apart on one row, below the 2.5px detector floor and below perceptual threshold |
| 84 | `chat-transcript-elements--error-card` | 🔴 **N-63** · N-54 | `Review error` 98x26 ✓. Full text census: headline 14/500, body 14/400 @0.92, guidance **12/400** @0.48, raw detail **14/400** @0.48 — the muted raw string outranks the muted guidance. Category chip renders the literal Rust variant **`ProviderTransient`** (12/650, warning tone) |
| 85 | `chat-transcript-elements--skill-cards` | 🔴 **N-65** · N-30 | `_actionButton_j2b83_26` **22 x 22** (2nd hardcoded-22 instance, → N-65); `_trigger_1hjpa_40` 1062x29 (N-30); `_toggle_1e58f_26` 1092x26 ✓. Row treatment splits three ways in one line: "Skill active:" 14/700 accent, skill name `storybook-authoring` **14/500** accent, "tools: …" 14/700 muted — the static label is heavier than the variable it introduces |
| 86 | `chat-transcript-elements--user-with-images` | 🔴 **N-64** | `IMGS=1`, trigger `_trigger_14sym_1` **80 x 80** (clear of the tap floor), but `naturalWidth x naturalHeight = **1 x 1**` — a single base64 pixel stretched 80x. `bgimg=0`. Census otherwise clean |

| 87 | `tool-cards-file-ops-basics--cat` | ✅ PASS | census clean; `_toggle_1e58f_26` **1102x26** ✓; fonts 12/400, 13/650, 14/400, 14/700 all on scale; `_body_1e58f_97` L31 = documented ToolCard body offset; n=66, `hscroll:false` |
| 88 | `tool-cards-file-ops-basics--tree` | ⚠️ N-49 | `_path_1a3ab_1` renders **13.3px** (the `0.95em` relative size) — 3rd measured instance of N-49; everything else identical to #87 (same n=66, same control set) |
| 89 | `tool-cards-file-ops-basics--regex-search` | ⚠️ N-49 | `_query_1e777_1` renders **13.3px** — 4th N-49 instance, second distinct class in the same folder; toggle 1102x26 ✓; n=67 |
| 90 | `tool-cards-file-ops-basics--shell` | ✅ PASS | census fully clean. Exec metadata renders correctly and legibly: duration `0.2s` **14/700** @0.48, status `exited` **12/650** @0.48, process id `proc-runtime-versions` **12/650** @0.48; toggle 1102x26 ✓; n=71 |
| 91 | `tool-cards-file-ops-basics--move-remove` | ✅ PASS (screenshot-verified) | Two tool rows **uniform 1102x26**, verbs `Move` / `Delete` at 14/700 with mono accent paths (`app.draft.json -> app.json`, `app.json.bak`); result line "The draft is now the active config…" renders; census clean; 3 interactives / 80 els |

### Session-6 notes
- **Transcript land is in good shape.** Nine of ten stories have a fully clean off-scale census; every
  defect found is either an already-tracked recurrence (N-30 29px triggers, N-51 sub-floor targets,
  N-39 weight sprawl) or a *coverage* problem rather than a rendering one. The main app surface is not
  where the design-system debt lives.
- **Two contracts verified rather than assumed:** hidden-role suppression (`LEAK=[]`, story 82) and
  goal-control nulling on a truly terminal state (story 77).
- **One self-retraction** (story 78 Storybook-chrome overflow), caught before filing by chasing the
  element's parent chain instead of trusting the probe.
- **Grep beats measurement for convention drift.** N-65 was invisible to per-story measurement — two
  22px buttons in unrelated stories look like two one-offs. One `grep` over `.module.css` turned them
  into a six-site systemic finding *and* retroactively explained N-34's unexplained `radius: 3px`
  (PlanBanner reaching for Radix `var(--radius-1)` instead of an `--rf-radius-*` token).

---

## Part 4f — SESSION-6 HANDOFF (read with Part 4b + 4d)

### State
**89 / 171 stories audited, 82 remaining.** Findings **N-01 .. N-65** (validated contiguous, no
duplicates, no malformed rows). Session 6 added stories **77-91** and findings **N-61 .. N-65**,
broadened **N-54** a second time, and corrected the running total (the previous "76" counted two
stories twice; true unique count entering this session was 74).

### Commits this session (local only, nothing pushed, zero source files touched)
`2139715ec` stories 77-86 + N-61..N-65 + N-54 broadened + totals reconciled ·
`b86be6339` stories 87-91.

### Rig — what to rebuild, and one thing that does NOT work
Everything in Part 4d still applies (viewport 1280x900 @2x, portal-aware `__els()` rooted at `body *`).
Two helpers were added this session and are worth recreating first, they cut round trips a lot:
- **`__p()`** — one call returning off-scale census (h/f/r/p) + alignment flags + control list +
  font/weight set + `n` + `docW`/`vw` + an `hscroll` boolean.
- **`__t(n)`** — text census: content, `size/weight`, colour, `x,y`, width for every text-bearing node.

**Known-ineffective, do not waste time on it again:** overriding `console.log/warn/error/info/debug/
group/groupCollapsed/trace` in an init script does **not** suppress MSW's request log from the tool's
console capture (the capture is at the CDP level, below the JS console object). Every batch will carry
~40-200 lines of `[MSW] GET /v1/ping` noise. The only real mitigation is **fewer, shorter batches** —
the noise accumulates with wall-clock time, because the app polls `/v1/ping` every 5s. Do not try to
route/abort `/v1/ping`: the plan history records that killing ping triggers the caps "death spiral"
(offline -> `capsApi.util.resetApiState()` -> caps never refetch) and every model-dependent surface
falls back to "Loading…".

### Measurement rules now in force (cumulative)
1. Never judge overlap/collision from an overlay screenshot — the red `WxH` labels sit above-left of
   their element. (2 false positives historically.)
2. Only same-pass measurements are comparable; late content settling toggles the scrollbar and shifts
   absolute x by ~10px.
3. **Take the screenshot.** Numbers alone cannot tell you a story is scrambled, blank, or illegible.
   Session 6 initially ran measurement-only and had to be corrected mid-session; story 91 was only
   confirmed sound by looking at it.
4. Before filing an overflow/spill finding, walk the element's **parent chain** — Storybook's own
   `sb-main-fullscreen > div` reports `scrollWidth > clientWidth` with `overflow-x: visible` on every
   story and is not app UI.
5. **Grep beats measurement for convention drift.** Two 22px buttons in unrelated stories look like two
   one-offs; one `grep` over `.module.css` turned them into the six-site N-65 and retroactively
   explained N-34's unexplained `radius: 3px`.

### Highest-value leads for the next session
- **N-61 is the most actionable finding in the ledger right now** — a 3-line fix in
  `TaskProgressWidget.tsx:191` (`isTerminal` omits `transferred`/`no_progress`) removes an enabled
  `Resume` from a goal another chat owns. Pair it with N-62 (add the 4 missing status stories).
- The queue below is ordered by expected yield: agentic tool cards and the remaining dialogs are
  product surfaces; the `--reduced-motion` suite (9 stories) is now measurable against portaled
  overlays **for the first time** thanks to the Part 4d `__els()` fix, so N-09 can finally be settled.
- **Still entirely outstanding: the 360x780 narrow pass and the light-theme pass.** Light-theme results
  remain meaningless until N-03 is fixed.

### Remaining queue (82)
- `chat-composer-modeselect--` (1): trigger
- `chat-composer-trajectorybutton--` (2): open-popover, trigger
- `chat-form--` (2): streaming, with-queued-messages
- `chat-form-retryform--` (1): with-images
- `checkbox--` (1): primary
- `collapsible--` (1): default
- `combobox-v2--` (1): default
- `components-chatlinks--` (1): default
- `components-text-animated--` (1): primary
- `design-system-emptystate--` (1): gallery
- `design-system-errorstate--` (1): gallery
- `design-system-loadingstate--` (1): gallery
- `design-system-overview--` (1): overview
- `design-system-skeleton--` (1): gallery
- `features-checkpoints--` (3): default, dialog-closed, with-no-changes
- `features-providers-addcustommodelmodal--` (1): open
- `integrations-mcpimportdialog--` (1): open-with-pasted-json
- `login--` (1): primary
- `logo-animation--` (3): idle, streaming, waiting
- `privacy-chat-shield--` (1): default
- `reveal--` (1): primary
- `scroll-area--` (1): primary
- `scroll-area-anchor--` (3): in-the-middle, primary, short
- `select--` (2): default, option-object
- `spinner--` (1): primary
- `textarea--` (1): primary
- `tool-cards-agentic-analysis--` (5): chrome, delegate, engine-analysis, finish, sleep-ask
- `tool-cards-file-ops-basics--` (4): generic-fallback, knowledge, web, web-search
- `toolconfirmation--` (2): patch, with-denial
- `ui-combobox--` (2): reduced-motion, states
- `ui-datatable--` (2): light-dark, wide
- `ui-editabletable--` (1): light-dark
- `ui-field--` (3): controls, narrow-settings-page, settings-page
- `ui-modelselector--` (6): custom-unset-label, disabled-rows-and-all-badges, inline-with-unset, light-and-dark, narrow-popover-sheet, panel-less-single-scroll
- `ui-overlays-dialog--` (1): reduced-motion
- `ui-overlays-menu--` (1): reduced-motion
- `ui-overlays-popover--` (1): reduced-motion
- `ui-overlays-sheet--` (3): light-dark, narrow, reduced-motion
- `ui-overlays-tooltip--` (2): light-dark, reduced-motion
- `ui-segmentedcontrol--` (1): reduced-motion
- `ui-select--` (1): reduced-motion
- `ui-slider--` (1): reduced-motion
- `ui-switch--` (1): reduced-motion
- `ui-tabs--` (1): reduced-motion
- `ui-toolcard--` (1): light-and-dark
- `ui-virtualizedgrid--` (3): responsive-grid, single-column, virtualized
- `ui-virtuallist--` (2): large-list, light-dark
- `usagecounter--` (3): anthropic-usage-counter, gpt-usage-counter, inline-usage-counter-in-chat-form

---

## Part 7 — REMEDIATION SESSION (post-session-6): fix waves landed

**Commits (local, in order):** `0f47234cb` global-CSS cascade pin (N-66) · `9276c34a0` design-system foundation · `40c012087` remediation waves (60+ findings) · weight-token sweep (N-39) — see git log.
**Gates:** tsc clean · eslint 0 errors · prettier clean · full vitest **323 files / 3668 passed / 6 skipped** (three full runs: post-foundation, post-waves, post-sweep) · live Storybook re-measured with the portal-aware protocol + screenshots.

### N-66 (NEW) · S1/regression · fixed@0f47234cb
**Global stylesheet cascade order floated with the module graph.** All global CSS (radix themes, tokens, glass, motion…) was imported only inside `Theme.tsx`; its emitted position depended on when the bundler first encountered that module. `c978554a0` (an unrelated leaf-import refactor) flipped it to AFTER all CSS modules, so `.rt-Box { display: block }` beat every module's `display: flex` at equal specificity — **0-height task-workspace chat** (user-reported) and intermittent loss of translucent panel overrides ("transparent sometimes, not always"). Measured in dist: module rules at 130K, rt-Box at 2.02MB (RADIX WINS). Fix: global sheets imported first in `src/lib/index.ts` (+ `styles/base.css` added; Theme.tsx keeps duplicates for isolation; `.storybook/preview.tsx` aligned so stories resolve the same cascade), locked by `src/lib/cssOrder.test.ts`. Post-fix dist: rt-Box @166K, tokens @702K, modules after — MODULE WINS. Root `.gitignore` `lib/` (Python template) was silently ignoring `src/lib/` additions; negated.

### Live re-measurements (Storybook :6006, 1280×900, portal-aware evals + screenshots)
| Check | Before (ledger) | After |
|---|---|---|
| Badge gallery heights | 24 (content-box inflation) | **[18]**, box-sizing border-box |
| Chip gallery heights | 23/28 ragged | **[28]** uniform |
| Tabs list vs panel edges | dR=+10px | **dL=0 dR=0** |
| SegmentedControl | 33.4/40.6 @16px | **group 30 / label 26 @13px** ×3 |
| Select triggers | 30/30/56.6 (grid stretch) | **[30,30,30]** |
| task-progress `--transferred` (new story) | n/a (N-62 gap) | **no Resume/Stop**, chip "Transferred", zero raw ids |
| task-progress `--budget-exhausted` | identical typography | Resume+Stop (post-budget-raise path), **2 saturated segments emphasized** |
| DeletePopover order | Delete@25 LEFT | **Cancel@25 / Delete@107** |
| ToolConfirmation `--mixed` | verdict in colour only, footnote 15px | **"Denied"/"Needs confirmation" chips**, footnote **12px** |
| error-card category chip | literal `ProviderTransient` | **"Temporary provider issue"** |
| ModeTransition title row | `agent -> Ask` | **display titles, no raw id** |
| ui-virtuallist--large-list | blank | **paints, 4 windowed items** |
| Live app @420 (engine dist) | files sheet buries chat, no dismiss | **Sheet close button in dock switcher** (L-01) |

### Notable root-cause corrections made during remediation
- **N-33's ledger premise was wrong**: the multimodal fixture shape was current; the real cause is ToolsContent dispatch order (`chrome` branch precedes `isMultiModalToolResult`, and ChromeTool renders images inside the collapsed body). Stories now cover BOTH paths (chrome + non-chrome tool names) plus an expand play.
- **N-40 was a PRODUCT bug, not a story bug**: `useAttachedImages` reset-effect fired while caps were loading (multimodality flag false-by-default) — **wiped user attachments on every caps refetch**. Now gated on resolved caps.
- **N-08 confirmation**: the Tabs overhang was the box-sizing family all along — the global border-box reset alone closes it (a local belt-and-braces declaration was kept for story isolation).

### L-table dispositions
| ID | Disposition |
|---|---|
| L-01 | **fixed@40c012087** — narrow workspace dock is a near-fullscreen left Sheet with no dismiss; added visible Sheet.Close in the dock switcher. Verified live at 420×900 against the engine-served dist |
| L-02 | **fixed via N-59** (34px headers → 36px tokenized) |
| L-03 | partial — ambient 13px base + form-control `font: inherit` landed; remaining rt-Text size-prop leaks are call-site-by-call-site |
| L-04 | partial — `--rf-line-1..5` integer tokens + base pairing landed; component-level adoption incremental |
| L-05 | partial — light overlay/glass split (.97/.88) landed; the intermittent black-glass ghost is plausibly N-66's cascade flip (module glass rules losing) — observe post-fix |
| L-06 | needs-reverify — `rgba(0,0,0,0.608)` no longer greps in source; likely healed by an earlier wave |
| L-07 | **fixed@40c012087** (strip on 4px rung via hairline-compensated padding) |
| L-08 | **fixed@40c012087** — zero `@radix-ui/react-icons` product imports (stories exempt) |
| L-09 | **accepted/wontfix** — CSS container queries cannot consume `var()`; the 520→260 ladder cannot be tokenized, only documented |
| L-10 | resolved (prior session) |
| L-11 | **fixed@40c012087** — ErrorCallout split: presentational `ErrorCalloutView` (store-free) + connected wrapper file |
| L-12 | **fixed@40c012087** — dead props removed from API + all callers (`mx/mt/mb/color/hex/size`); `message` moved onto DiffWarningCallout where it is actually used |
| L-13 | **fixed@40c012087** (single 12px grid) |
| L-14 | verified-already-healed (session-4 story 39 measured uniform 91.7px rows) |
| L-15 | **fixed@40c012087** (12px group-label convention) |
| L-16 | **fixed@40c012087** (uniform 26px checkbox rows) |
| L-17 | open — RetryForm model trigger 30px among 26px controls; row alignment is perfect (cySpread=0); left as a deliberate S4 |
| L-18 | **fixed@40c012087** (scrollable + thin scrollbar + bottom padding) |
| L-19 | **fixed@40c012087** (19px/bold + unified copy) |
| L-20 | **fixed@40c012087** via N-37 (plan selectors memoized on the messages array) |
| L-21 | open — ChatStoryHarness still hardcodes dark appearance |
| L-22 | partial — preview now propagates appearance/color-scheme to `<html>` (portals included); ThemePropsContext propagation still untested in stories |
| L-23 | accepted — lint-config decision, not a bug |
| L-24 | **fixed@40c012087** (viewport-clamped min-width in css module) |
| L-25 | **fixed@40c012087** (DialogImage Trigger + OpenLightbox stories, real PNG) |
| L-26 | open — legacy ChatContent stories still on local MockedStore |
| L-27 | **fixed@40c012087** (`--rf-overlay-popover-max`) |

### Honest residuals (still open after this session)
- **N-06** partial (Callout done; other `components/*` barrel imports unaudited) · **N-21/L-04** line-height adoption incremental · **N-27/L-26** legacy `chat--*` suite unmigrated · **N-31** needs-reverify (element unmounted mid-probe) · **N-51** partial: every measured sub-floor instance fixed, but no enforcement test/lint exists yet · **L-17, L-21, L-22** as above.
- The 360×780 narrow pass and the light-theme pass over the full story inventory remain outstanding (N-03's fix unblocks the light pass).
- Sweep coverage stands at 89/171 stories; the remaining 82-story queue in Part 4f is unchanged.

### New guardrails introduced
- `src/lib/cssOrder.test.ts` — global-CSS cascade order characterization (N-66).
- `src/styles/base.css` — global border-box + KaTeX exemption + ambient type maps (N-44/N-48).
- `--rf-weight-*` and `--rf-line-1..5` tokens (N-39/L-04); zero `font-weight:` literals outside tokens.css; zero `var(--radius-*)` Radix leaks outside the Theme adapter (N-65).
- `src/utils/displayNames.ts` `humanizeIdentifier` — the single lookup for internal identifiers reaching chrome (N-54).
- Storybook preview imports the full global cascade and mirrors reduced-motion/appearance onto `<html>` so portaled overlays obey both (N-04/N-09).

---

## Part 8 — REVIEW ROUND (multi-agent review of the remediation, all findings fixed)

A 90-finding multi-agent review ran over the remediation commits; every accepted finding was fixed in the same session. **Gates after fixes: lint 0 · types 0 · format 0 · full vitest 323 files / 3686 passed (+18 new tests).** Browser-verified: `font: var(--rf-weight-*)` shorthands compute correctly (Select 500/13px, SegmentedControl 650/13px @30px group).

### Accepted & fixed
- **Dialog partitioning was incomplete (rf-2c73eee6 HIGH + 4 siblings):** wrapped actions never pinned (the flagship story itself shipped the N-07 defect), all props beyond 4 were dropped, fragments bypassed partitioning entirely (caught by the NEW tests, not the reviewers), zero partition tests. Fixed: explicit `Dialog.Footer` part, trailing `Close|Footer` lifting, fragment flattening, full prop forwarding + style merge (Sheet parity), story migrated, **5 rendered partition tests**.
- **Weight sweep missed `font:` shorthands (20 findings):** ~200 literals in 87 files swept (incl. 550/720 strays); **zero weight literals remain in any form outside tokens.css** — the AGENTS claim is now true.
- **Goal matrix v3 (rf-73a028d5 verified):** `budget_exhausted`/`no_progress` now offer Stop; transferred/completed/stopped kill all controls; shared action-descriptor list deduplicates GoalControlIcons/GoalControls; `GOAL_STATUS_LABELS` deleted in favor of `humanizeIdentifier`; dead `_isStreaming` removed; scroll bounds named via `--tpw-*` component vars.
- **ToolConfirmation:** empty-reasons vacuous-`every` guard (rf-2e3ca234), inline padding → CSS module, 5-parallel-array signature collapsed, dynamic `app/store` import → static leaf import + `selectConfig`.
- **N-40 follow-through:** dispatch-time insert guard *only when caps resolved as non-multimodal* (my first guard draft silently dropped pre-caps attachments — caught by the restored original test), + 2 new caps-gate tests; test config needed `dev: true` (the documented engine-endpoint gate).
- **Callout:** `preventRetry` off the base props (DOM leak), warning type now renders AlertTriangle/warning tone, ErrorCallout story targets the presentational View store-free.
- **SegmentedControl:** `useId` fallback name restores native arrow-key grouping (rf-f675c58e), `size="sm"` renders 26 total again (rf-4ce0458d) with test assertions for both height rules, light story panel painted, redundant box-sizing removed.
- **EditableTable:** textareas no longer clamped to 30px (rf-39ab2869 verified), `getInputProps` typed (`EditableTableInputAttributes` = input attrs + `data-*` Record; renamed to dodge a pre-existing internal interface collision), LightDark story is a real two-panel composition.
- **ModelSelector:** disabled now disables search + rows; dead searchInputRef and redundant branch removed.
- **Tokens/base:** `--rf-z-sticky: 500` defined (was referenced, never defined); JetBrains **light**-theme panel fallback (was dark #16181d under light text, rf-96725e27); typography `:where()` maps scoped under `.radix-themes` so the engine-injected toolbar is untouched (rf-34dd70f5).
- **Infra:** `parameters.reducedMotion` per-story override reaches portals (RM overlay stories pinned); cssOrder test now cross-checks the entry/Theme/preview sheet lists for drift (rf-6b0bbacf); Dock narrow-sheet Close has a rendered regression test (rf-e5ff3ad1); `humanizeIdentifier` splits acronyms (HTTPRequest → "Http request", rf-ad504584) + tests; Chip remove glyph via kit Icon; Chip.stories on lucide; TasksSection ticks stuck-detection every 60s and closes the narrow Sheet on task navigation.

### Declined / deferred (documented reasons)
- **rf-3f8e3080/rf-fccb0f39** (Dock.test CSS-parsing "unnecessary"): CSS characterization tests are the documented house pattern (Toolbar/TaskWorkspace precedents) — declined.
- **rf-4cd51519/rf-6b0bbacf** (Theme.tsx duplicate import list): kept intentionally for isolated mounts; drift risk now covered by the cssOrder sync test instead of removal.
- **rf-2c6bf74f** (4 legacy humanizer impls unconsolidated): AGENTS wording softened to "route NEW identifier display through humanizeIdentifier; legacy humanizers consolidated opportunistically" — full consolidation deferred.
- **rf-b6a97107** (~50 legacy box-sizing declarations): AGENTS wording now says new code must not add them; legacy removed opportunistically (done in every file touched this round).
- **rf-5088b886** (TaskBoardLoader indirection, 0.35 confidence): pre-existing pattern, deferred.
- **rf-4c8cb831** (isIconOnlyLabel heuristic → explicit API): public API change, deferred.
