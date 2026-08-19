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
| L-02 | S3 | Button heights outside {26,28,30,36}: chat-links pills ~17, Settings nav 36.75, worktree action buttons 28.75, composer "Set a goal" row | live measurements pre-merge; re-measure post integer type scale |
| L-03 | S3 | Radix 12/14px type leaks (ToolCard toggle wrapper, user-bubble root, ThreadInfo labels) | live measurements |
| L-04 | S3 | Odd type sizes 13/15/19 × `--rf-line` 1.5 → half-pixel line boxes (19.5/22.5/28.5); needs paired `--rf-line-N` px tokens | fresh evidence: kit Sheet h=342.5, DataTable row pitch 23/22 alternating (N-21) |
| L-05 | S3 | Glass recipe divergence + light-theme elevation collapse (overlay ≡ glass ≡ rgba(250,250,251,.92)) + intermittent black-glass compositing ghost | tokens.css light block |
| L-06 | S3 | Unowned light-theme label color rgba(0,0,0,.608) (AAA fail); full contrast sweep still owed (contrast_audit failed closed twice) | Providers labels |
| L-07 | S3 | Trajectory tab strip +5px off-grid from content top | Trajectory popover |
| L-08 | S3 | `@radix-ui/react-icons` remnants beyond TrajectoryButton | grep `@radix-ui/react-icons` |
| L-09 | S3 | Composer hand-rolled 520→260 container-query breakpoint ladder | ChatForm.module.css:331-375 |
| L-10 | S3 | TextArea consumer-dependent font size (composer vs RetryForm) | TextArea.module.css consumers |
| L-11 | S3 | ErrorCallout store-coupled in `components/` (useAppSelector in generic component) | Callout.tsx:111 |
| L-12 | S3 | Callout dead-prop API: `color`/`hex`/`mx`/`mt`/`mb`/`size` accepted and discarded; live callers still pass them | Callout.tsx:20-26,36-42,116,147,173 |
| L-13 | S4 | Worktree panel three alignment grids (13/17/21px edge insets) | Worktrees popover |
| L-14 | S4 | Mode list 3 row species (111/95.5/48px) | ModeSelect list |
| L-15 | S4 | Scheduler group labels split (now 12 vs 13 after integer scale) | Scheduler form |
| L-16 | S4 | Trajectory checkbox rows non-uniform (31/33.3/31 pre-merge; re-measure) | Trajectory popover |
| L-17 | S4 | RetryForm mixes 26px and 30px control rows | RetryForm |
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
| N-01 | **S2** | open | **Button gallery "Variants × sizes" matrix SCRAMBLED.** DOM order = 4 headers → 5 variant labels → 15 buttons flowed row-major into a 4-col grid: labels Ghost/Soft/Primary/Danger fill row 2 across the sm/md/lg header columns, "Plain" wraps to row 3 col 1, all buttons land offset (buttons under "Variant" column, rows mixing variants). The canonical gallery lies about every variant×size cell. Fix: interleave `label,btn,btn,btn` per row or explicit grid placement. `ui-button--variants-sizes-states` |
| N-02 | **S2** | open | **Icon gallery scrambled identically** — tone labels muted/faint/accent/warning/danger occupy the sm/md/lg columns; trailing icon rows unlabeled. Same signature ⇒ shared matrix story-helper bug; one fix heals both. `ui-icon--sizes-and-tones` |
| N-03 | **S2** | open | **Light-pair helper split.** Broken no-background token-flipping wrapper (light half illegible on dark canvas): Button, Icon, Field controls, Field settings-page ×2 (100% illegible — zero coverage value), Switch, Slider, Select, Combobox, SegmentedControl, Tabs, VirtualList, EditableTable, overlay trigger cards. CORRECT painting helper exists and is used by: ToolCard, ModelSelector, DataTable light-dark stories. Fix: converge every gallery on the painting helper. |
| N-04 | **S2** | open | **Portaled overlays escape the light wrapper** — light-dark Menu story renders BOTH menus as dark glass (portal mounts at body under dark preview Theme). No overlay primitive has any light-mode coverage. Fix: per-story appearance global (render story twice via toolbar/global) instead of side-by-side wrappers, or portal-aware theme wrapper. |
| N-05 | **S2** | open | **`ui-virtuallist--large-list` renders blank.** DOM holds "1000 memories" + exactly 1 item + footer; painted output is empty (virtuoso container height collapse). Sibling VirtualizedGrid sizes correctly ⇒ story/container-height fix. Also `ui-virtuallist--light-dark` is in the broken light family. |
| N-06 | **S2** | partial fix | **Story import cascade via barrels.** `callout--default` load fetches ~100+ CSS modules app-wide (Buddy panels, Workspace terminal, Integrations, every tool card) because `Callout.tsx` imported the `../../hooks` barrel (≈70 hook re-exports → services/app/features). Slows loads enough to break automation waits; floods MSW warnings; likely the historical cold-start dual-React culprit. **Fixed for Callout this session (leaf import).** Chat-land stories inherently import the app; remaining work: audit other `components/*` for barrel imports. |
| N-07 | **S2/S3** | open | **kit Dialog demo scroll defects** (opened via click): scroll owner is the WHOLE dialog — native chunky scrollbar runs through the title row, title/description scroll away; **Close button (30px) sits INSIDE the scroll area** (below fold at scroll-top) — the S2-9 "unreachable footer" disease in the kit's own flagship. Geometry is perfect (342px=340+borders, radius 10, overlay bg rgba(28,28,31,.94), centered dx=dy=0). Sheet does it right (Close pinned outside scroll) — copy Sheet's structure. |
| N-08 | **S3** | open | **Tabs strip overhangs its alignment column by exactly 10px.** Title/desc/panel span x 61→701; tablist spans 61→711 (its 4px padding + 1px border per side uncompensated); last tab right edge 706. Tabs are fractional 213.33px wide → sub-pixel indicator positions. Fix: −5px side margins on the list or matching panel padding. `ui-tabs--states` |
| N-09 | **S3** | open | **`.rf-popover-motion` keyframe (`rf-scale-fade`) survives the reduced-motion helper class** — under the Storybook RM toggle, transitions zero out but the overlay enter keyframe still runs (1 animated node; identified `_content_* rf-popover-motion`). Form components measure fully clean (0 animated nodes across all 6 RM stories). Disable rule likely lives only in the media query; the helper class (and any class-driven host toggle) misses overlays. |
| N-10 | **S3** | open | **DataTable narrow-stacked mode produces page-level horizontal scroll at 360px** (`document.scrollWidth > innerWidth`, right edges clipped) — the no-scroll fallback violating the doctrine it exists for. Also table-mode numeric right-align (Latency) leaks into stacked cards, reading as misalignment. `ui-datatable--narrow-stacked` |
| N-11 | **S3** | open | **EditableTable same-row cell inputs measure 30px (Name) vs 52px (Description)** — ragged rows; contract controls are 26/30/36. Remove-row 28px ✓, Add 26px ✓, validation display ✓. `ui-editabletable--add-remove-enter-validation` |
| N-12 | **S3** | open | **Chip remove buttons 18×18px** on removable + disabled chips — under the kit's ≥28px tap-target floor (`--rf-control-h-icon-sm`). Fix: padded pseudo-element hit area. `design-system-chip--gallery` |
| N-13 | S4 | open | Button gallery icon-only sizes row: 4px vertical-center drift across 15 buttons (edge-aligned, wants `align-items:center`); both light+dark halves. |
| N-14 | S4 | open | LoadingState compact tile spinner renders ~2px, nearly invisible on dark — "Loading providers" reads as bare text. Check Spinner size/tone defaults in that composition. |
| N-15 | S4/story | open | kit Select `states` story never opens the select (its description promises grouped items/selected tint/hover) while its **reduced-motion twin auto-opens it** — open-state coverage lives in the wrong story. |
| N-16 | S4/story | open | Dialog and Popover light-dark + narrow stories are trigger-only (no play/defaultOpen); Menu auto-opens — inconsistent overlay-story quality. Sheet/popover-narrow needed manual clicks too. |
| N-17 | S4/doc | open | Badge measures 24px h / 12px font / pad 2×6 / r6 — single size only in gallery; prior ledger text referenced a "badge scale 16/18/22" that matches nothing measured. Reconcile docs or add size variants + stories. |
| N-18 | S4 | open | Kit Select light-section "Small" variant measures 56.6px tall (expected 26) — hidden by the illegible light half; re-measure after N-03 fix. |
| N-19 | S3/infra | **fixed (this session)** | **MSW `onUnhandledRequest` bypass hardcoded to `http://localhost:6006/src/`** — on any other port (audit ran :6007) every dev-server asset fetch warned, flooding consoles (~100+ warnings/page on cascade stories). Fixed: origin-relative pathname check (warn only `/v1/`+`/p/` API paths). `.storybook/preview.tsx` |
| N-20 | S4/doc | open | DataTable wide-mode horizontal scroll container does not use the sanctioned `.scrollX` class (own overflow container; behavior fine, contract vocabulary diverges). |
| N-21 | S3 | open | Half-pixel line-box evidence pack for L-04: Sheet content h=342.5; DataTable row pitch alternates 23/22 (13px×1.5=19.5 boxes). Paired `--rf-line-N` px tokens close it. |
| N-22 | S4/a11y | open | SegmentedControl segments match neither `button` nor `[role=radio]` in role probes — semantics opaque; verify keyboard/AT pattern (likely label+hidden-input; confirm in source). |
| N-23 | S4 | open | Kit Dialog/Popover internal scrollbars are native chunky, not the app's thin styled scrollbars — cosmetic inconsistency inside overlays. |

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

## Part 4 — Verified clean (do not re-litigate without new cause)

Surface & Card galleries · StatusDot · Skeleton · EmptyState · ErrorState · Overview workbench · Switch geometry (36×20/16 exact) · Menu overlay geometry (386/12/10/30@13) · popover→Sheet responsive contract @360 (12/12/12 margins; old S4-7 asymmetry definitively closed) · Sheet pinned footer · Tooltip clamp · ModelSelector single-scroll-owner + truncation contracts (all modes) · DataTable column alignment + numeric alignment · VirtualizedGrid geometry + windowing · ToolCard shell (26px toggles, on-scale census, correct light story) · reduced-motion form sextet (0 animated nodes) · Accordion.

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
