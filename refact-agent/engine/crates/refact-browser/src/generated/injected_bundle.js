// @refact-injected-hash 0c45fae096d7f8716e1ca0441bebdaeaf54f463f32faaa7649e88f90fc31f73a

var __export = (target, all) => { for (var name in all) target[name] = all[name]; };
var __toCommonJS = mod => ({ ...mod, __esModule: true });

// src/refactInjected.ts
var refactInjected_exports = {};
__export(refactInjected_exports, {
  RefactInjected: () => RefactInjected,
  bootstrapRefactInjected: () => bootstrapRefactInjected
});
module.exports = __toCommonJS(refactInjected_exports);
var injectedInstanceName = "__refact_injected__";
var bindingName = "__refact_binding";
var RefactInjected = class {
  constructor(global, builtins) {
    this.global = global;
    this.builtinSnapshot = builtins;
  }
  version() {
    return "playwright-1.63.0-next-refact-1";
  }
  builtins() {
    return this.builtinSnapshot;
  }
  resolveSimple(cssSelector) {
    return this.global.document.querySelector(cssSelector);
  }
  dispatchBinding(name, payload) {
    const global = this.global;
    const binding = global[bindingName];
    const stringify = this.builtinSnapshot.jsonStringify;
    if (!binding)
      throw new Error(`${bindingName} is not installed`);
    binding(stringify({ name, payload }));
  }
};
function bootstrapRefactInjected(global, builtins) {
  const refactGlobal = global;
  const existing = refactGlobal[injectedInstanceName];
  if (existing)
    return existing;
  const injected = new RefactInjected(global, builtins);
  refactGlobal[injectedInstanceName] = injected;
  return injected;
}
