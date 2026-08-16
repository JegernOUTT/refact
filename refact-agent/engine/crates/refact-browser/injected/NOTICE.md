# Playwright source notice

This directory contains source adapted from [Microsoft Playwright](https://github.com/microsoft/playwright) at commit `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` (`1.63.0-next`). Playwright is licensed under the Apache License 2.0; the license text is in [LICENSE-PLAYWRIGHT](LICENSE-PLAYWRIGHT).

## Vendored files

| Local file | Upstream file | Commit | Local modifications |
|---|---|---|---|
| `src/vendor/injected/domUtils.ts` | `packages/injected/src/domUtils.ts`, `packages/injected/src/roleUtils.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Adds the disabled, readonly, and checked helpers required by element-state predicates. |
| `src/vendor/injected/roleUtils.ts` | `packages/injected/src/roleUtils.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Package aliases are replaced with relative imports; `getImplicitAriaRole` is exported for the injected role API. |
| `src/vendor/injected/selectorUtils.ts` | `packages/injected/src/selectorUtils.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Package aliases are replaced with relative imports; the role-dependent label helper is omitted until the role implementation is vendored. |
| `src/vendor/injected/utilityScript.ts` | `packages/injected/src/utilityScript.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Package aliases are replaced with relative imports. |
| `src/vendor/injected/bindingsController.ts` | `packages/injected/src/bindingsController.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Package aliases are replaced with relative imports. |
| `src/vendor/isomorphic/cssParser.ts` | `packages/isomorphic/cssParser.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | None. |
| `src/vendor/isomorphic/selectorParser.ts` | `packages/isomorphic/selectorParser.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | None. |
| `src/vendor/isomorphic/stringUtils.ts` | `packages/isomorphic/stringUtils.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | None. |
| `src/vendor/isomorphic/cssTokenizer.ts` | `packages/isomorphic/cssTokenizer.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Vendored as a dependency of `cssParser.ts`; otherwise unchanged. |
| `src/vendor/isomorphic/ariaRole.ts` | `packages/isomorphic/ariaSnapshot.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Includes only the `AriaRole` type required by `roleUtils.ts`. |
| `src/vendor/isomorphic/utilityScriptSerializers.ts` | `packages/isomorphic/utilityScriptSerializers.ts` | `d5a185a894ab3ab17ff77a44e116a1339c6bdaed` | Vendored as a dependency of the utility script and bindings controller; otherwise unchanged. |

## Adapted implementation references

The build settings and CommonJS artifact form follow `utils/generate_injected.js`. The early globals snapshot follows `packages/playwright-core/src/server/javascript.ts`. The per-execution-context CommonJS wrapper follows `packages/playwright-core/src/server/dom.ts`. These files are implementation references and are not copied into this directory.
