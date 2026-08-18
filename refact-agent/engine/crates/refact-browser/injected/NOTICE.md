# Playwright source notice

This directory contains source adapted from [Microsoft Playwright](https://github.com/microsoft/playwright) at commit `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` (`1.63.0-next`). Playwright is licensed under the Apache License 2.0; the license text is in [LICENSE-PLAYWRIGHT](LICENSE-PLAYWRIGHT).

## Vendored files

| Local file | Upstream file | Commit | Local modifications |
|---|---|---|---|
| `src/vendor/injected/ariaSnapshot.ts` | `packages/injected/src/ariaSnapshot.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Package aliases are replaced with relative imports; interactable refs require the explicit `refs` option so they remain disabled by default; children dropped at the `depth` limit record a `truncatedChildren` count instead of disappearing silently. |
| `src/vendor/injected/ariaSnapshotDistiller.ts` | `packages/injected/src/ariaSnapshotDistiller.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Package aliases are replaced with relative imports. |
| `src/vendor/injected/domUtils.ts` | `packages/injected/src/domUtils.ts`, `packages/injected/src/roleUtils.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Includes the DOM, visibility, box, and element-state helpers used by the Refact injected runtime. |
| `src/vendor/injected/hitTarget.ts` | `packages/injected/src/injectedScript.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Extracts the composed-root hit test, DOM preview, and capture-phase event interceptor into a standalone module backed by the Refact builtins snapshot. |
| `src/vendor/injected/roleUtils.ts` | `packages/injected/src/roleUtils.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Package aliases are replaced with relative imports; `getImplicitAriaRole` is exported for the injected role API. |
| `src/vendor/injected/selectorUtils.ts` | `packages/injected/src/selectorUtils.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Package aliases are replaced with relative imports. |
| `src/vendor/injected/selectorGenerator.ts` | `packages/injected/src/selectorGenerator.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Package aliases are replaced with relative imports; the runtime interface is narrowed to the Refact selector evaluator and query API; the small test-id attribute splitter is inlined. |
| `src/vendor/injected/domUtils.ts` | `packages/injected/src/domUtils.ts`, `packages/injected/src/roleUtils.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Includes only the disabled, readonly, and checked helpers required by element-state predicates; full role computation remains omitted. |
| `src/vendor/injected/hitTarget.ts` | `packages/injected/src/injectedScript.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Extracts the composed-root hit test, DOM preview, and capture-phase event interceptor into a standalone module backed by the Refact builtins snapshot. |
| `src/vendor/injected/roleUtils.ts` | `packages/injected/src/roleUtils.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Package aliases are replaced with relative imports; `getImplicitAriaRole` is exported for the injected role API. |
| `src/vendor/injected/selectorEvaluator.ts` | `packages/injected/src/selectorEvaluator.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Package aliases are replaced with relative imports. |
| `src/vendor/injected/layoutSelectorUtils.ts` | `packages/injected/src/layoutSelectorUtils.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | None. |
| `src/vendor/injected/xpathSelectorEngine.ts` | `packages/injected/src/xpathSelectorEngine.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | None. |
| `src/vendor/injected/selectorEngine.ts` | `packages/injected/src/selectorEngine.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | None. |
| `src/vendor/injected/selectorUtils.ts` | `packages/injected/src/selectorUtils.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Package aliases are replaced with relative imports; the role-dependent label helper is omitted until the role implementation is vendored. |
| `src/vendor/injected/utilityScript.ts` | `packages/injected/src/utilityScript.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Package aliases are replaced with relative imports. |
| `src/vendor/injected/bindingsController.ts` | `packages/injected/src/bindingsController.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Package aliases are replaced with relative imports. |
| `src/vendor/isomorphic/ariaSnapshot.ts` | `packages/isomorphic/ariaSnapshot.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | `AriaNodeJSON` carries an optional `truncatedChildren` count for nodes collapsed at the `depth` limit. |
| `src/vendor/isomorphic/ariaSnapshotRenderer.ts` | `packages/isomorphic/ariaSnapshotRenderer.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Nodes carrying `truncatedChildren` render a `… (N children truncated)` marker line instead of appearing as childless leaves. |
| `src/vendor/isomorphic/cssParser.ts` | `packages/isomorphic/cssParser.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | None. |
| `src/vendor/isomorphic/selectorParser.ts` | `packages/isomorphic/selectorParser.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | None. |
| `src/vendor/isomorphic/stringUtils.ts` | `packages/isomorphic/stringUtils.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | None. |
| `src/vendor/isomorphic/cssTokenizer.ts` | `packages/isomorphic/cssTokenizer.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Vendored as a dependency of `cssParser.ts`; otherwise unchanged. |
| `src/vendor/isomorphic/ariaRole.ts` | `packages/isomorphic/ariaSnapshot.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Includes only the `AriaRole` type required by `roleUtils.ts`. |
| `src/vendor/isomorphic/utilityScriptSerializers.ts` | `packages/isomorphic/utilityScriptSerializers.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Vendored as a dependency of the utility script and bindings controller; otherwise unchanged. |
| `src/vendor/isomorphic/yaml.ts` | `packages/isomorphic/yaml.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Vendored as a dependency of the ARIA snapshot renderer; otherwise unchanged. |
| `src/vendor/injected/roleSelectorEngine.ts` | `packages/injected/src/roleSelectorEngine.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Package aliases are replaced with relative imports. |
| `src/vendor/injected/selectorUtils.ts` | `packages/injected/src/selectorUtils.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Restores the role-dependent label helper after role computation was vendored. |

## Adapted implementation references

The build settings and CommonJS artifact form follow `utils/generate_injected.js`. The early globals snapshot follows `packages/playwright-core/src/server/javascript.ts`. The per-execution-context CommonJS wrapper follows `packages/playwright-core/src/server/dom.ts`. These files are implementation references and are not copied into this directory.
