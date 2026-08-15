// @refact-injected-hash 0161b06a871c2bc6d34de7bf5c051494b4d6be614624dfe324d16cb50b8d27b8

var __export = (target, all) => { for (var name in all) target[name] = all[name]; };
var __toCommonJS = mod => ({ ...mod, __esModule: true });

// src/refactInjected.ts
var refactInjected_exports = {};
__export(refactInjected_exports, {
  RefactInjected: () => RefactInjected
});
module.exports = __toCommonJS(refactInjected_exports);
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
};
