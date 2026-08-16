// @refact-injected-hash b1a34cb5276e4c8f8185341e631b28e333041d9c19a921d52704d1e4988b1216

var __export = (target, all) => { for (var name in all) target[name] = all[name]; };
var __toCommonJS = mod => ({ ...mod, __esModule: true });

// src/refactInjected.ts
var refactInjected_exports = {};
__export(refactInjected_exports, {
  RefactInjected: () => RefactInjected,
  bootstrapRefactInjected: () => bootstrapRefactInjected
});
module.exports = __toCommonJS(refactInjected_exports);

// src/vendor/injected/domUtils.ts
var globalOptions = {};
function parentElementOrShadowHost(element) {
  if (element.parentElement)
    return element.parentElement;
  if (!element.parentNode)
    return;
  if (element.parentNode.nodeType === 11 && element.parentNode.host)
    return element.parentNode.host;
}
function getElementComputedStyle(element, pseudo) {
  const cache = pseudo === "::before" ? cacheStyleBefore : pseudo === "::after" ? cacheStyleAfter : cacheStyle;
  if (cache && cache.has(element))
    return cache.get(element);
  const style = element.ownerDocument && element.ownerDocument.defaultView ? element.ownerDocument.defaultView.getComputedStyle(element, pseudo) : void 0;
  cache == null ? void 0 : cache.set(element, style);
  return style;
}
function isElementStyleVisibilityVisible(element, style) {
  const cached = cacheStyleVisibility == null ? void 0 : cacheStyleVisibility.get(element);
  if (cached !== void 0)
    return cached;
  const result = computeElementStyleVisibilityVisible(element, style);
  cacheStyleVisibility == null ? void 0 : cacheStyleVisibility.set(element, result);
  return result;
}
function computeElementStyleVisibilityVisible(element, style) {
  style = style != null ? style : getElementComputedStyle(element);
  if (!style)
    return true;
  if (Element.prototype.checkVisibility && globalOptions.browserNameForWorkarounds !== "webkit") {
    if (!element.checkVisibility())
      return false;
  } else {
    const detailsOrSummary = element.closest("details,summary");
    if (detailsOrSummary !== element && (detailsOrSummary == null ? void 0 : detailsOrSummary.nodeName) === "DETAILS" && !detailsOrSummary.open)
      return false;
  }
  if (style.visibility !== "visible")
    return false;
  return true;
}
function computeBox(element) {
  const style = getElementComputedStyle(element);
  if (!style)
    return { visible: true, inline: false };
  const cursor = style.cursor;
  if (style.display === "contents") {
    for (let child = element.firstChild; child; child = child.nextSibling) {
      if (child.nodeType === 1 && isElementVisible(child))
        return { visible: true, inline: false, cursor };
      if (child.nodeType === 3 && isVisibleTextNode(child))
        return { visible: true, inline: true, cursor };
    }
    return { visible: false, inline: false, cursor };
  }
  if (!isElementStyleVisibilityVisible(element, style))
    return { cursor, visible: false, inline: false };
  const rect = element.getBoundingClientRect();
  return { cursor, visible: rect.width > 0 && rect.height > 0, inline: style.display === "inline" };
}
function isElementVisible(element) {
  return computeBox(element).visible;
}
function isVisibleTextNode(node) {
  const range = node.ownerDocument.createRange();
  range.selectNode(node);
  const rect = range.getBoundingClientRect();
  return rect.width > 0 && rect.height > 0;
}
function elementSafeTagName(element) {
  const tagName = element.tagName;
  if (typeof tagName === "string") {
    const firstCharCode = tagName.charCodeAt(0);
    if (firstCharCode >= 97 && firstCharCode <= 122)
      return tagName.toUpperCase();
    return tagName;
  }
  if (element instanceof HTMLFormElement)
    return "FORM";
  return element.tagName.toUpperCase();
}
var ariaReadonlyRoles = ["checkbox", "combobox", "grid", "gridcell", "listbox", "radiogroup", "slider", "spinbutton", "textbox", "columnheader", "rowheader", "searchbox", "switch", "treegrid"];
function getAriaDisabled(element) {
  return isNativelyDisabled(element) || hasAriaDisabledInChain(element);
}
function isNativelyDisabled(element) {
  const isNativeFormControl = ["BUTTON", "INPUT", "SELECT", "TEXTAREA", "OPTION", "OPTGROUP"].includes(elementSafeTagName(element));
  return isNativeFormControl && (element.hasAttribute("disabled") || belongsToDisabledOptGroup(element) || belongsToDisabledFieldSet(element));
}
function belongsToDisabledOptGroup(element) {
  return elementSafeTagName(element) === "OPTION" && !!element.closest("OPTGROUP[DISABLED]");
}
function belongsToDisabledFieldSet(element) {
  const fieldSetElement = element.closest("FIELDSET[DISABLED]");
  if (!fieldSetElement)
    return false;
  const legendElement = fieldSetElement.querySelector(":scope > LEGEND");
  return !legendElement || !legendElement.contains(element);
}
function hasAriaDisabledInChain(element) {
  const attribute = (element.getAttribute("aria-disabled") || "").toLowerCase();
  if (attribute === "true")
    return true;
  if (attribute === "false")
    return false;
  const parent = parentElementOrShadowHost(element);
  return parent ? hasAriaDisabledInChain(parent) : false;
}
function getReadonly(element) {
  const tagName = elementSafeTagName(element);
  if (["INPUT", "TEXTAREA"].includes(tagName))
    return element.hasAttribute("readonly");
  if (tagName === "SELECT")
    return ariaReadonlyRoles.includes(explicitAriaRole(element)) && element.getAttribute("aria-readonly") === "true";
  const role = explicitAriaRole(element);
  if (ariaReadonlyRoles.includes(role))
    return element.getAttribute("aria-readonly") === "true";
  if (element.isContentEditable)
    return false;
  if (element.hasAttribute("contenteditable"))
    return true;
  return "error";
}
function explicitAriaRole(element) {
  const explicit = (element.getAttribute("role") || "").split(" ").map((role) => role.trim()).find((role) => ariaReadonlyRoles.includes(role));
  if (explicit)
    return explicit;
  const tagName = elementSafeTagName(element);
  if (tagName === "TEXTAREA")
    return "textbox";
  if (tagName === "SELECT")
    return "combobox";
  if (tagName === "INPUT") {
    const type = element.type;
    if (type === "checkbox")
      return "checkbox";
    if (type === "search")
      return "searchbox";
    if (type === "range")
      return "slider";
    if (type === "number")
      return "spinbutton";
    if (!["button", "submit", "reset", "image", "file", "hidden", "radio"].includes(type))
      return "textbox";
  }
  return "";
}
function getCheckedState(element) {
  const tagName = elementSafeTagName(element);
  if (tagName === "INPUT" && element.indeterminate)
    return "mixed";
  if (tagName === "INPUT" && ["checkbox", "radio"].includes(element.type))
    return element.checked ? "checked" : "unchecked";
  const checked = element.getAttribute("aria-checked");
  if (checked === "true")
    return "checked";
  if (checked === "mixed")
    return "mixed";
  if (checked === "false")
    return "unchecked";
  return null;
}
var cacheStyle;
var cacheStyleBefore;
var cacheStyleAfter;
var cacheStyleVisibility;

// src/refactInjected.ts
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
  async elementState(element, state) {
    this.ensureConnected(element);
    if (state === "visible") {
      const visible = isElementVisible(element);
      return { visible, matches: visible };
    }
    if (state === "enabled") {
      const enabled = !getAriaDisabled(element);
      return { enabled, matches: enabled };
    }
    if (state === "editable") {
      const editable = this.editableState(element);
      if (editable === null)
        throw new Error("Element is not an <input>, <textarea>, <select> or [contenteditable] and does not have a role allowing [aria-readonly]");
      return { editable, matches: editable };
    }
    if (state === "checked" || state === "unchecked" || state === "mixed") {
      const checked = getCheckedState(element);
      if (checked === null)
        throw new Error("Not a checkbox or radio button");
      return {
        checked,
        matches: state === "checked" ? checked === "checked" : state === "unchecked" ? checked === "unchecked" : checked === "mixed"
      };
    }
    if (state === "stable") {
      const stable = await this.checkElementIsStable(element);
      return { stable, matches: stable };
    }
    throw new Error(`Unexpected element state "${state}"`);
  }
  async elementStates(element) {
    return {
      visible: this.bestEffort(() => element.isConnected && isElementVisible(element), false),
      enabled: this.bestEffort(() => element.isConnected && !getAriaDisabled(element), false),
      editable: this.bestEffort(() => element.isConnected ? this.editableState(element) : null, null),
      checked: this.bestEffort(() => element.isConnected ? getCheckedState(element) : null, null),
      stable: await this.bestEffortStable(element)
    };
  }
  bestEffort(read, fallback) {
    try {
      return read();
    } catch {
      return fallback;
    }
  }
  async bestEffortStable(element) {
    try {
      return element.isConnected && await this.checkElementIsStable(element);
    } catch {
      return false;
    }
  }
  editableState(element) {
    const readonly = getReadonly(element);
    return readonly === "error" ? null : !getAriaDisabled(element) && !readonly;
  }
  ensureConnected(element) {
    if (!element || !element.isConnected)
      throw new Error("Element is not attached to the DOM");
  }
  async checkElementIsStable(element) {
    const requestAnimationFrame = this.builtinSnapshot.requestAnimationFrame;
    const performanceNow = this.builtinSnapshot.performanceNow;
    let lastRect;
    let lastTime = 0;
    return await new Promise((resolve, reject) => {
      const check = () => {
        try {
          this.ensureConnected(element);
          const time = performanceNow();
          if (lastTime && time - lastTime < 15) {
            requestAnimationFrame(check);
            return;
          }
          lastTime = time;
          const clientRect = element.getBoundingClientRect();
          const rect = { x: clientRect.x, y: clientRect.y, width: clientRect.width, height: clientRect.height };
          if (lastRect) {
            resolve(
              rect.x === lastRect.x && rect.y === lastRect.y && rect.width === lastRect.width && rect.height === lastRect.height
            );
            return;
          }
          lastRect = rect;
          requestAnimationFrame(check);
        } catch (error) {
          reject(error);
        }
      };
      try {
        requestAnimationFrame(check);
      } catch (error) {
        reject(error);
      }
    });
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
