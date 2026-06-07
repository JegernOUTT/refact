# Refact UI Kit

`src/components/ui` is the reusable, presentational UI kit for Refact GUI. It provides shared primitives and styling helpers that can be composed by feature code without importing feature state, services, or app wiring back into the kit.

## Public surface

`index.ts` is the canonical barrel for the kit. Add new exports there in alphabetical order, one per line, so downstream imports have one stable entry point and merge conflicts stay small.

## Folder convention

Each component owns a folder:

```text
ui/<Name>/
├── <Name>.tsx
├── <Name>.module.css
├── <Name>.stories.tsx
└── index.ts
```

Use `PascalCase` for component files and CSS modules. Keep component-local styles beside the component.

## Boundary rule

The kit is presentational only. Files under `src/components/ui/**` and shared style files under `src/styles/**` must not import from `features`, `services`, or `app`. Boundary lint enforces this rule.

Connected widgets live in their owning feature folder and compose kit pieces. If a component needs Redux, RTK Query, chat selectors, or service calls, keep that wiring outside `src/components/ui`.

## Design conventions

- Use `var(--rf-*)` tokens for colors, spacing, radii, shadows, typography, sizing, z-index, blur, and motion.
- Do not add hardcoded colors, spacing, radii, or magic layout numbers.
- Keep content panel-less by default. Reserve surfaces for overlays, fields, selected or active state, and true containment.
- Use CSS Modules for component styles.
- Use Lucide outline icons through the shared `Icon` wrapper. Icons inherit `currentColor` and communicate state by color only.
- Do not use emoji as icons. Data or content emoji are allowed when they are part of the product content.
- Prefer CSS-only motion using `--rf-dur-*`, `--rf-ease-*`, and `--rf-stagger`.
- Motion must honor `prefers-reduced-motion` and `prefers-reduced-transparency`.
- Use shared motion and responsive utilities before adding one-off CSS.
- Keep shrinkable flex and grid children able to shrink with `min-width: 0`; horizontal overflow belongs only in explicit scroll islands.
- Every component ships a `.stories.tsx` file.

## Verification

Run these checks before merging UI kit changes:

```bash
npm run types && npm run lint && npm run lint:css && npm run lint:boundaries && npm run build-storybook
```
