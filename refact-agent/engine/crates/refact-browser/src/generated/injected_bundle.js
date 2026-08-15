// @refact-injected-hash 181b5f86b5764801f689d4c9bb3bf73f5aa85a25c84859004b51348313fdb7c9

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
  resolveAll(locator) {
    var _a, _b, _c, _d, _e, _f, _g, _h, _i, _j, _k;
    const document = this.global.document;
    const scope = locator.within ? document.querySelector(locator.within) : document;
    if (!scope)
      throw new Error("Scope selector not found");
    let elements;
    switch (locator.by) {
      case "css":
        elements = Array.from(scope.querySelectorAll((_a = locator.value) != null ? _a : ""));
        break;
      case "id": {
        const element = scope.querySelector(`#${CSS.escape((_b = locator.value) != null ? _b : "")}`);
        elements = element ? [element] : [];
        break;
      }
      case "name":
        elements = Array.from(scope.querySelectorAll(`[name=${JSON.stringify((_c = locator.value) != null ? _c : "")}]`));
        break;
      case "test_id":
        elements = Array.from(scope.querySelectorAll(`[data-testid=${JSON.stringify((_d = locator.value) != null ? _d : "")}]`));
        break;
      case "placeholder":
        elements = Array.from(scope.querySelectorAll(`[placeholder=${JSON.stringify((_e = locator.value) != null ? _e : "")}]`));
        break;
      case "autocomplete":
        elements = Array.from(scope.querySelectorAll(`[autocomplete=${JSON.stringify((_f = locator.value) != null ? _f : "")}]`));
        break;
      case "text": {
        const target = (_g = locator.value) != null ? _g : "";
        elements = Array.from(scope.querySelectorAll("*")).filter((element) => {
          const text = element.innerText;
          return locator.exact ? (text == null ? void 0 : text.trim()) === target : !!(text == null ? void 0 : text.includes(target));
        });
        break;
      }
      case "label": {
        const target = (_h = locator.value) != null ? _h : "";
        elements = [];
        for (const label of Array.from(scope.querySelectorAll("label"))) {
          if (!((_i = label.innerText) == null ? void 0 : _i.trim().includes(target)))
            continue;
          const element = label.htmlFor ? document.getElementById(label.htmlFor) : label.querySelector("input,textarea,select");
          if (element)
            elements.push(element);
        }
        if (!elements.length)
          elements = Array.from(scope.querySelectorAll("[aria-label]")).filter(
            (element) => {
              var _a2;
              return (_a2 = element.getAttribute("aria-label")) == null ? void 0 : _a2.includes(target);
            }
          );
        break;
      }
      case "role": {
        const role = (_j = locator.role) != null ? _j : "";
        const candidates = Array.from(scope.querySelectorAll(`[role=${JSON.stringify(role)}]`));
        elements = locator.name ? candidates.filter((element) => {
          var _a2;
          const name = element.getAttribute("aria-label") || element.innerText || "";
          return name.trim().includes((_a2 = locator.name) != null ? _a2 : "");
        }) : candidates;
        break;
      }
      case "xpath": {
        const result = document.evaluate(
          (_k = locator.value) != null ? _k : "",
          scope,
          null,
          XPathResult.ORDERED_NODE_SNAPSHOT_TYPE,
          null
        );
        elements = [];
        for (let index = 0; index < result.snapshotLength; index++) {
          const element = result.snapshotItem(index);
          if (element instanceof Element)
            elements.push(element);
        }
        break;
      }
      default:
        throw new Error(`Unknown locator strategy: ${locator.by}`);
    }
    if (locator.nth !== void 0)
      elements = elements.length > locator.nth ? [elements[locator.nth]] : [];
    return elements;
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
