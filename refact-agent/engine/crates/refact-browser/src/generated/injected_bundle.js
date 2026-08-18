// @refact-injected-hash e5845517af25eced17389920255171c81b23757095b32cb8c513d407dee795ae

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
function isInsideScope(scope, element) {
  while (element) {
    if (scope.contains(element))
      return true;
    element = enclosingShadowHost(element);
  }
  return false;
}
function parentElementOrShadowHost(element) {
  if (element.parentElement)
    return element.parentElement;
  if (!element.parentNode)
    return;
  if (element.parentNode.nodeType === 11 && element.parentNode.host)
    return element.parentNode.host;
}
function enclosingShadowRootOrDocument(element) {
  let node = element;
  while (node.parentNode)
    node = node.parentNode;
  if (node.nodeType === 11 || node.nodeType === 9)
    return node;
}
function enclosingShadowHost(element) {
  while (element.parentElement)
    element = element.parentElement;
  return parentElementOrShadowHost(element);
}
function closestCrossShadow(element, css, scope) {
  while (element) {
    const closest = element.closest(css);
    if (scope && closest !== scope && (closest == null ? void 0 : closest.contains(scope)))
      return;
    if (closest)
      return closest;
    element = enclosingShadowHost(element);
  }
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
var cachesCounter = 0;
function beginDOMCaches() {
  ++cachesCounter;
  cacheStyle != null ? cacheStyle : cacheStyle = /* @__PURE__ */ new Map();
  cacheStyleBefore != null ? cacheStyleBefore : cacheStyleBefore = /* @__PURE__ */ new Map();
  cacheStyleAfter != null ? cacheStyleAfter : cacheStyleAfter = /* @__PURE__ */ new Map();
  cacheStyleVisibility != null ? cacheStyleVisibility : cacheStyleVisibility = /* @__PURE__ */ new Map();
}
function endDOMCaches() {
  if (!--cachesCounter) {
    cacheStyle = void 0;
    cacheStyleBefore = void 0;
    cacheStyleAfter = void 0;
    cacheStyleVisibility = void 0;
  }
}

// src/vendor/isomorphic/stringUtils.ts
function quoteCSSAttributeValue(text) {
  return `"${text.replace(/["\\]/g, (char) => "\\" + char)}"`;
}
var normalizedWhitespaceCache;
function normalizeWhiteSpace(text) {
  let result = normalizedWhitespaceCache == null ? void 0 : normalizedWhitespaceCache.get(text);
  if (result === void 0) {
    result = text.replace(/[\u200b\u00ad]/g, "").trim().replace(/\s+/g, " ");
    normalizedWhitespaceCache == null ? void 0 : normalizedWhitespaceCache.set(text, result);
  }
  return result;
}
function escapeRegexForSelector(re) {
  if (re.unicode || re.unicodeSets)
    return String(re);
  return String(re).replace(/(^|[^\\])(\\\\)*(["'`])/g, "$1$2\\$3").replace(/>>/g, "\\>\\>");
}
function escapeForTextSelector(text, exact) {
  if (typeof text !== "string")
    return escapeRegexForSelector(text);
  return `${JSON.stringify(text)}${exact ? "s" : "i"}`;
}
function escapeForAttributeSelector(value, exact) {
  if (typeof value !== "string")
    return escapeRegexForSelector(value);
  return `"${value.replace(/\\/g, "\\\\").replace(/["]/g, '\\"')}"${exact ? "s" : "i"}`;
}
function trimString(input, cap, suffix = "") {
  if (input.length <= cap)
    return input;
  const chars = [...input];
  if (chars.length > cap)
    return chars.slice(0, cap - suffix.length).join("") + suffix;
  return chars.join("");
}
function trimStringWithEllipsis(input, cap) {
  return trimString(input, cap, "…");
}
function truncateDataUrl(url) {
  if (!url.startsWith("data:"))
    return url;
  const comma = url.indexOf(",");
  if (comma === -1)
    return url;
  return url.slice(0, comma + 1) + "…";
}
function escapeRegExp(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
function longestCommonSubstring(s1, s2) {
  const n = s1.length;
  const m = s2.length;
  let maxLen = 0;
  let endingIndex = 0;
  const dp = Array(n + 1).fill(null).map(() => Array(m + 1).fill(0));
  for (let i = 1; i <= n; i++) {
    for (let j = 1; j <= m; j++) {
      if (s1[i - 1] === s2[j - 1]) {
        dp[i][j] = dp[i - 1][j - 1] + 1;
        if (dp[i][j] > maxLen) {
          maxLen = dp[i][j];
          endingIndex = i;
        }
      }
    }
  }
  return s1.slice(endingIndex - maxLen, endingIndex);
}
var ansiRegex = new RegExp("([\\u001B\\u009B][[\\]()#?]*(?:(?:(?:[a-zA-Z\\d]*(?:;[-a-zA-Z\\d\\/#&.:=?%@~_]*)*)?\\u0007)|(?:(?:\\d{0,4}(?:;\\d{0,4})*)?[\\dA-PR-TZcf-ntqry=><~])))", "g");

// src/vendor/injected/hitTarget.ts
var autoClosingTags = [
  "AREA",
  "BASE",
  "BR",
  "COL",
  "COMMAND",
  "EMBED",
  "HR",
  "IMG",
  "INPUT",
  "KEYGEN",
  "LINK",
  "MENUITEM",
  "META",
  "PARAM",
  "SOURCE",
  "TRACK",
  "WBR"
];
var booleanAttributes = ["checked", "selected", "disabled", "readonly", "multiple"];
var HitTargetController = class {
  constructor(global, builtins) {
    this.nextInterceptorId = 1;
    this.global = global;
    this.builtins = builtins;
    this.interceptors = new builtins.Map();
  }
  expectHitTarget(hitPoint, targetElement) {
    if (!(targetElement == null ? void 0 : targetElement.isConnected))
      return { status: "not_connected" };
    return this.toResult(this.checkHitTarget(hitPoint, targetElement));
  }
  install(targetElement, action, hitPoint, blockAllEvents = false) {
    if (!(targetElement == null ? void 0 : targetElement.isConnected))
      return { status: "not_connected" };
    if (hitPoint) {
      const preliminaryResult = this.checkHitTarget(hitPoint, targetElement);
      if (preliminaryResult !== "done")
        return this.toResult(preliminaryResult);
    }
    if (action === "drag")
      return { status: "skipped" };
    const events = new this.builtins.Set(this.eventNames(action));
    const id = this.nextInterceptorId++;
    const interceptor = {
      events,
      blockAllEvents,
      result: void 0,
      listener: (() => {
      })
    };
    const listener = (event) => {
      if (!events.has(event.type) || !event.isTrusted)
        return;
      const point = this.eventPoint(event);
      if (interceptor.result === void 0 && point)
        interceptor.result = this.checkHitTarget(point, targetElement);
      if (interceptor.blockAllEvents || interceptor.result !== "done" && interceptor.result !== void 0) {
        this.builtins.preventDefault(event);
        this.builtins.stopPropagation(event);
        this.builtins.stopImmediatePropagation(event);
      }
    };
    interceptor.listener = listener;
    for (const eventName of events)
      this.builtins.addWindowEventListener(eventName, listener, { capture: true, passive: false });
    this.interceptors.set(id, interceptor);
    return { status: "installed", id };
  }
  take(id) {
    var _a;
    const interceptor = this.interceptors.get(id);
    if (!interceptor)
      throw new Error(`Unknown hit-target interceptor ${id}`);
    this.interceptors.delete(id);
    for (const eventName of interceptor.events)
      this.builtins.removeWindowEventListener(eventName, interceptor.listener, { capture: true });
    return this.toResult((_a = interceptor.result) != null ? _a : "done");
  }
  checkHitTarget(hitPoint, targetElement) {
    var _a;
    const roots = [];
    let parentElement = targetElement;
    while (parentElement) {
      const root = enclosingShadowRootOrDocument2(parentElement);
      if (!root)
        break;
      roots.push(root);
      if (root.nodeType === 9)
        break;
      parentElement = root.host;
    }
    let hitElement;
    for (let index = roots.length - 1; index >= 0; index--) {
      const root = roots[index];
      const elements = this.builtins.arrayFrom(
        root.nodeType === 9 ? this.builtins.documentElementsFromPoint(root, hitPoint.x, hitPoint.y) : this.builtins.shadowElementsFromPoint(root, hitPoint.x, hitPoint.y)
      );
      const singleElement = root.nodeType === 9 ? this.builtins.documentElementFromPoint(root, hitPoint.x, hitPoint.y) : this.builtins.shadowElementFromPoint(root, hitPoint.x, hitPoint.y);
      if (singleElement && elements[0] && parentElementOrShadowHost2(singleElement) === elements[0]) {
        const style = this.builtins.getComputedStyle(singleElement);
        if ((style == null ? void 0 : style.display) === "contents")
          elements.unshift(singleElement);
      }
      if (elements[0] && elements[0].shadowRoot === root && elements[1] === singleElement)
        elements.shift();
      const innerElement = elements[0];
      if (!innerElement)
        break;
      hitElement = innerElement;
      if (index && innerElement !== roots[index - 1].host)
        break;
    }
    const hitParents = [];
    while (hitElement && hitElement !== targetElement) {
      hitParents.push(hitElement);
      hitElement = (_a = hitElement.assignedSlot) != null ? _a : parentElementOrShadowHost2(hitElement);
    }
    if (hitElement === targetElement)
      return "done";
    const hitTargetDescription = previewNode(hitParents[0] || this.global.document.documentElement);
    let rootHitTargetDescription;
    let element = targetElement;
    while (element) {
      const index = hitParents.indexOf(element);
      if (index !== -1) {
        if (index > 1)
          rootHitTargetDescription = previewNode(hitParents[index - 1]);
        break;
      }
      element = parentElementOrShadowHost2(element);
    }
    if (rootHitTargetDescription) {
      return {
        hitTargetDescription: `${hitTargetDescription} from ${rootHitTargetDescription} subtree intercepts pointer events`
      };
    }
    return { hitTargetDescription: `${hitTargetDescription} intercepts pointer events` };
  }
  eventNames(action) {
    if (action === "hover")
      return ["mousemove"];
    if (action === "tap")
      return ["pointerdown", "pointerup", "pointercancel", "touchstart", "touchend", "touchcancel"];
    return [
      "mousedown",
      "mouseup",
      "pointerdown",
      "pointerup",
      "click",
      "auxclick",
      "dblclick",
      "contextmenu"
    ];
  }
  eventPoint(event) {
    if ("touches" in event) {
      const touch = event.touches[0];
      return touch ? { x: touch.clientX, y: touch.clientY } : void 0;
    }
    if ("clientX" in event && "clientY" in event) {
      const pointer = event;
      return { x: pointer.clientX, y: pointer.clientY };
    }
    return void 0;
  }
  toResult(result) {
    if (result === "done")
      return { status: "done" };
    return { status: "intercepted", description: result.hitTargetDescription };
  }
};
function parentElementOrShadowHost2(element) {
  if (element.parentElement)
    return element.parentElement;
  if (!element.parentNode)
    return void 0;
  if (element.parentNode.nodeType === 11 && element.parentNode.host)
    return element.parentNode.host;
  return void 0;
}
function enclosingShadowRootOrDocument2(element) {
  let node = element;
  while (node.parentNode)
    node = node.parentNode;
  if (node.nodeType === 11 || node.nodeType === 9)
    return node;
  return void 0;
}
function previewNode(node) {
  if (node.nodeType === 3)
    return oneLine(`#text=${node.nodeValue || ""}`);
  if (node.nodeType !== 1)
    return oneLine(`<${node.nodeName.toLowerCase()} />`);
  const element = node;
  const attributes = [];
  for (let index = 0; index < element.attributes.length; index++) {
    const { name, value } = element.attributes[index];
    if (name === "style")
      continue;
    attributes.push(!value && booleanAttributes.includes(name) ? ` ${name}` : ` ${name}="${value}"`);
  }
  attributes.sort((left, right) => left.length - right.length);
  const attributeText = trimStringWithEllipsis(attributes.join(""), 500);
  if (autoClosingTags.includes(element.nodeName))
    return oneLine(`<${element.nodeName.toLowerCase()}${attributeText}/>`);
  const children = element.childNodes;
  let onlyText = children.length <= 5;
  for (let index = 0; index < children.length; index++)
    onlyText = onlyText && children[index].nodeType === 3;
  const text = onlyText ? element.textContent || "" : children.length ? "…" : "";
  return oneLine(
    `<${element.nodeName.toLowerCase()}${attributeText}>${trimStringWithEllipsis(text, 50)}</${element.nodeName.toLowerCase()}>`
  );
}
function oneLine(value) {
  return value.replace(/\n/g, "↵").replace(/\t/g, "⇆");
}

// src/vendor/isomorphic/cssTokenizer.ts
var between = function(num, first, last) {
  return num >= first && num <= last;
};
function digit(code) {
  return between(code, 48, 57);
}
function hexdigit(code) {
  return digit(code) || between(code, 65, 70) || between(code, 97, 102);
}
function uppercaseletter(code) {
  return between(code, 65, 90);
}
function lowercaseletter(code) {
  return between(code, 97, 122);
}
function letter(code) {
  return uppercaseletter(code) || lowercaseletter(code);
}
function nonascii(code) {
  return code >= 128;
}
function namestartchar(code) {
  return letter(code) || nonascii(code) || code === 95;
}
function namechar(code) {
  return namestartchar(code) || digit(code) || code === 45;
}
function nonprintable(code) {
  return between(code, 0, 8) || code === 11 || between(code, 14, 31) || code === 127;
}
function newline(code) {
  return code === 10;
}
function whitespace(code) {
  return newline(code) || code === 9 || code === 32;
}
var maximumallowedcodepoint = 1114111;
var InvalidCharacterError = class extends Error {
  constructor(message) {
    super(message);
    this.name = "InvalidCharacterError";
  }
};
function preprocess(str) {
  const codepoints = [];
  for (let i = 0; i < str.length; i++) {
    let code = str.charCodeAt(i);
    if (code === 13 && str.charCodeAt(i + 1) === 10) {
      code = 10;
      i++;
    }
    if (code === 13 || code === 12)
      code = 10;
    if (code === 0)
      code = 65533;
    if (between(code, 55296, 56319) && between(str.charCodeAt(i + 1), 56320, 57343)) {
      const lead = code - 55296;
      const trail = str.charCodeAt(i + 1) - 56320;
      code = Math.pow(2, 16) + lead * Math.pow(2, 10) + trail;
      i++;
    }
    codepoints.push(code);
  }
  return codepoints;
}
function stringFromCode(code) {
  if (code <= 65535)
    return String.fromCharCode(code);
  code -= Math.pow(2, 16);
  const lead = Math.floor(code / Math.pow(2, 10)) + 55296;
  const trail = code % Math.pow(2, 10) + 56320;
  return String.fromCharCode(lead) + String.fromCharCode(trail);
}
function tokenize(str1) {
  const str = preprocess(str1);
  let i = -1;
  const tokens = [];
  let code;
  let line = 0;
  let column = 0;
  let lastLineLength = 0;
  const incrLineno = function() {
    line += 1;
    lastLineLength = column;
    column = 0;
  };
  const locStart = { line, column };
  const codepoint = function(i2) {
    if (i2 >= str.length)
      return -1;
    return str[i2];
  };
  const next = function(num) {
    if (num === void 0)
      num = 1;
    if (num > 3)
      throw "Spec Error: no more than three codepoints of lookahead.";
    return codepoint(i + num);
  };
  const consume = function(num) {
    if (num === void 0)
      num = 1;
    i += num;
    code = codepoint(i);
    if (newline(code))
      incrLineno();
    else
      column += num;
    return true;
  };
  const reconsume = function() {
    i -= 1;
    if (newline(code)) {
      line -= 1;
      column = lastLineLength;
    } else {
      column -= 1;
    }
    locStart.line = line;
    locStart.column = column;
    return true;
  };
  const eof = function(codepoint2) {
    if (codepoint2 === void 0)
      codepoint2 = code;
    return codepoint2 === -1;
  };
  const donothing = function() {
  };
  const parseerror = function() {
  };
  const consumeAToken = function() {
    consumeComments();
    consume();
    if (whitespace(code)) {
      while (whitespace(next()))
        consume();
      return new WhitespaceToken();
    } else if (code === 34) {
      return consumeAStringToken();
    } else if (code === 35) {
      if (namechar(next()) || areAValidEscape(next(1), next(2))) {
        const token = new HashToken("");
        if (wouldStartAnIdentifier(next(1), next(2), next(3)))
          token.type = "id";
        token.value = consumeAName();
        return token;
      } else {
        return new DelimToken(code);
      }
    } else if (code === 36) {
      if (next() === 61) {
        consume();
        return new SuffixMatchToken();
      } else {
        return new DelimToken(code);
      }
    } else if (code === 39) {
      return consumeAStringToken();
    } else if (code === 40) {
      return new OpenParenToken();
    } else if (code === 41) {
      return new CloseParenToken();
    } else if (code === 42) {
      if (next() === 61) {
        consume();
        return new SubstringMatchToken();
      } else {
        return new DelimToken(code);
      }
    } else if (code === 43) {
      if (startsWithANumber()) {
        reconsume();
        return consumeANumericToken();
      } else {
        return new DelimToken(code);
      }
    } else if (code === 44) {
      return new CommaToken();
    } else if (code === 45) {
      if (startsWithANumber()) {
        reconsume();
        return consumeANumericToken();
      } else if (next(1) === 45 && next(2) === 62) {
        consume(2);
        return new CDCToken();
      } else if (startsWithAnIdentifier()) {
        reconsume();
        return consumeAnIdentlikeToken();
      } else {
        return new DelimToken(code);
      }
    } else if (code === 46) {
      if (startsWithANumber()) {
        reconsume();
        return consumeANumericToken();
      } else {
        return new DelimToken(code);
      }
    } else if (code === 58) {
      return new ColonToken();
    } else if (code === 59) {
      return new SemicolonToken();
    } else if (code === 60) {
      if (next(1) === 33 && next(2) === 45 && next(3) === 45) {
        consume(3);
        return new CDOToken();
      } else {
        return new DelimToken(code);
      }
    } else if (code === 64) {
      if (wouldStartAnIdentifier(next(1), next(2), next(3)))
        return new AtKeywordToken(consumeAName());
      else
        return new DelimToken(code);
    } else if (code === 91) {
      return new OpenSquareToken();
    } else if (code === 92) {
      if (startsWithAValidEscape()) {
        reconsume();
        return consumeAnIdentlikeToken();
      } else {
        parseerror();
        return new DelimToken(code);
      }
    } else if (code === 93) {
      return new CloseSquareToken();
    } else if (code === 94) {
      if (next() === 61) {
        consume();
        return new PrefixMatchToken();
      } else {
        return new DelimToken(code);
      }
    } else if (code === 123) {
      return new OpenCurlyToken();
    } else if (code === 124) {
      if (next() === 61) {
        consume();
        return new DashMatchToken();
      } else if (next() === 124) {
        consume();
        return new ColumnToken();
      } else {
        return new DelimToken(code);
      }
    } else if (code === 125) {
      return new CloseCurlyToken();
    } else if (code === 126) {
      if (next() === 61) {
        consume();
        return new IncludeMatchToken();
      } else {
        return new DelimToken(code);
      }
    } else if (digit(code)) {
      reconsume();
      return consumeANumericToken();
    } else if (namestartchar(code)) {
      reconsume();
      return consumeAnIdentlikeToken();
    } else if (eof()) {
      return new EOFToken();
    } else {
      return new DelimToken(code);
    }
  };
  const consumeComments = function() {
    while (next(1) === 47 && next(2) === 42) {
      consume(2);
      while (true) {
        consume();
        if (code === 42 && next() === 47) {
          consume();
          break;
        } else if (eof()) {
          parseerror();
          return;
        }
      }
    }
  };
  const consumeANumericToken = function() {
    const num = consumeANumber();
    if (wouldStartAnIdentifier(next(1), next(2), next(3))) {
      const token = new DimensionToken();
      token.value = num.value;
      token.repr = num.repr;
      token.type = num.type;
      token.unit = consumeAName();
      return token;
    } else if (next() === 37) {
      consume();
      const token = new PercentageToken();
      token.value = num.value;
      token.repr = num.repr;
      return token;
    } else {
      const token = new NumberToken();
      token.value = num.value;
      token.repr = num.repr;
      token.type = num.type;
      return token;
    }
  };
  const consumeAnIdentlikeToken = function() {
    const str2 = consumeAName();
    if (str2.toLowerCase() === "url" && next() === 40) {
      consume();
      while (whitespace(next(1)) && whitespace(next(2)))
        consume();
      if (next() === 34 || next() === 39)
        return new FunctionToken(str2);
      else if (whitespace(next()) && (next(2) === 34 || next(2) === 39))
        return new FunctionToken(str2);
      else
        return consumeAURLToken();
    } else if (next() === 40) {
      consume();
      return new FunctionToken(str2);
    } else {
      return new IdentToken(str2);
    }
  };
  const consumeAStringToken = function(endingCodePoint) {
    if (endingCodePoint === void 0)
      endingCodePoint = code;
    let string = "";
    while (consume()) {
      if (code === endingCodePoint || eof()) {
        return new StringToken(string);
      } else if (newline(code)) {
        parseerror();
        reconsume();
        return new BadStringToken();
      } else if (code === 92) {
        if (eof(next()))
          donothing();
        else if (newline(next()))
          consume();
        else
          string += stringFromCode(consumeEscape());
      } else {
        string += stringFromCode(code);
      }
    }
    throw new Error("Internal error");
  };
  const consumeAURLToken = function() {
    const token = new URLToken("");
    while (whitespace(next()))
      consume();
    if (eof(next()))
      return token;
    while (consume()) {
      if (code === 41 || eof()) {
        return token;
      } else if (whitespace(code)) {
        while (whitespace(next()))
          consume();
        if (next() === 41 || eof(next())) {
          consume();
          return token;
        } else {
          consumeTheRemnantsOfABadURL();
          return new BadURLToken();
        }
      } else if (code === 34 || code === 39 || code === 40 || nonprintable(code)) {
        parseerror();
        consumeTheRemnantsOfABadURL();
        return new BadURLToken();
      } else if (code === 92) {
        if (startsWithAValidEscape()) {
          token.value += stringFromCode(consumeEscape());
        } else {
          parseerror();
          consumeTheRemnantsOfABadURL();
          return new BadURLToken();
        }
      } else {
        token.value += stringFromCode(code);
      }
    }
    throw new Error("Internal error");
  };
  const consumeEscape = function() {
    consume();
    if (hexdigit(code)) {
      const digits = [code];
      for (let total = 0; total < 5; total++) {
        if (hexdigit(next())) {
          consume();
          digits.push(code);
        } else {
          break;
        }
      }
      if (whitespace(next()))
        consume();
      let value = parseInt(digits.map(function(x) {
        return String.fromCharCode(x);
      }).join(""), 16);
      if (value > maximumallowedcodepoint)
        value = 65533;
      return value;
    } else if (eof()) {
      return 65533;
    } else {
      return code;
    }
  };
  const areAValidEscape = function(c1, c2) {
    if (c1 !== 92)
      return false;
    if (newline(c2))
      return false;
    return true;
  };
  const startsWithAValidEscape = function() {
    return areAValidEscape(code, next());
  };
  const wouldStartAnIdentifier = function(c1, c2, c3) {
    if (c1 === 45)
      return namestartchar(c2) || c2 === 45 || areAValidEscape(c2, c3);
    else if (namestartchar(c1))
      return true;
    else if (c1 === 92)
      return areAValidEscape(c1, c2);
    else
      return false;
  };
  const startsWithAnIdentifier = function() {
    return wouldStartAnIdentifier(code, next(1), next(2));
  };
  const wouldStartANumber = function(c1, c2, c3) {
    if (c1 === 43 || c1 === 45) {
      if (digit(c2))
        return true;
      if (c2 === 46 && digit(c3))
        return true;
      return false;
    } else if (c1 === 46) {
      if (digit(c2))
        return true;
      return false;
    } else if (digit(c1)) {
      return true;
    } else {
      return false;
    }
  };
  const startsWithANumber = function() {
    return wouldStartANumber(code, next(1), next(2));
  };
  const consumeAName = function() {
    let result = "";
    while (consume()) {
      if (namechar(code)) {
        result += stringFromCode(code);
      } else if (startsWithAValidEscape()) {
        result += stringFromCode(consumeEscape());
      } else {
        reconsume();
        return result;
      }
    }
    throw new Error("Internal parse error");
  };
  const consumeANumber = function() {
    let repr = "";
    let type = "integer";
    if (next() === 43 || next() === 45) {
      consume();
      repr += stringFromCode(code);
    }
    while (digit(next())) {
      consume();
      repr += stringFromCode(code);
    }
    if (next(1) === 46 && digit(next(2))) {
      consume();
      repr += stringFromCode(code);
      consume();
      repr += stringFromCode(code);
      type = "number";
      while (digit(next())) {
        consume();
        repr += stringFromCode(code);
      }
    }
    const c1 = next(1);
    const c2 = next(2);
    const c3 = next(3);
    if ((c1 === 69 || c1 === 101) && digit(c2)) {
      consume();
      repr += stringFromCode(code);
      consume();
      repr += stringFromCode(code);
      type = "number";
      while (digit(next())) {
        consume();
        repr += stringFromCode(code);
      }
    } else if ((c1 === 69 || c1 === 101) && (c2 === 43 || c2 === 45) && digit(c3)) {
      consume();
      repr += stringFromCode(code);
      consume();
      repr += stringFromCode(code);
      consume();
      repr += stringFromCode(code);
      type = "number";
      while (digit(next())) {
        consume();
        repr += stringFromCode(code);
      }
    }
    const value = convertAStringToANumber(repr);
    return { type, value, repr };
  };
  const convertAStringToANumber = function(string) {
    return +string;
  };
  const consumeTheRemnantsOfABadURL = function() {
    while (consume()) {
      if (code === 41 || eof()) {
        return;
      } else if (startsWithAValidEscape()) {
        consumeEscape();
        donothing();
      } else {
        donothing();
      }
    }
  };
  let iterationCount = 0;
  while (!eof(next())) {
    tokens.push(consumeAToken());
    iterationCount++;
    if (iterationCount > str.length * 2)
      throw new Error("I'm infinite-looping!");
  }
  return tokens;
}
var CSSParserToken = class {
  constructor() {
    this.tokenType = "";
  }
  toJSON() {
    return { token: this.tokenType };
  }
  toString() {
    return this.tokenType;
  }
  toSource() {
    return "" + this;
  }
};
var BadStringToken = class extends CSSParserToken {
  constructor() {
    super(...arguments);
    this.tokenType = "BADSTRING";
  }
};
var BadURLToken = class extends CSSParserToken {
  constructor() {
    super(...arguments);
    this.tokenType = "BADURL";
  }
};
var WhitespaceToken = class extends CSSParserToken {
  constructor() {
    super(...arguments);
    this.tokenType = "WHITESPACE";
  }
  toString() {
    return "WS";
  }
  toSource() {
    return " ";
  }
};
var CDOToken = class extends CSSParserToken {
  constructor() {
    super(...arguments);
    this.tokenType = "CDO";
  }
  toSource() {
    return "<!--";
  }
};
var CDCToken = class extends CSSParserToken {
  constructor() {
    super(...arguments);
    this.tokenType = "CDC";
  }
  toSource() {
    return "-->";
  }
};
var ColonToken = class extends CSSParserToken {
  constructor() {
    super(...arguments);
    this.tokenType = ":";
  }
};
var SemicolonToken = class extends CSSParserToken {
  constructor() {
    super(...arguments);
    this.tokenType = ";";
  }
};
var CommaToken = class extends CSSParserToken {
  constructor() {
    super(...arguments);
    this.tokenType = ",";
  }
};
var GroupingToken = class extends CSSParserToken {
  constructor() {
    super(...arguments);
    this.value = "";
    this.mirror = "";
  }
};
var OpenCurlyToken = class extends GroupingToken {
  constructor() {
    super();
    this.tokenType = "{";
    this.value = "{";
    this.mirror = "}";
  }
};
var CloseCurlyToken = class extends GroupingToken {
  constructor() {
    super();
    this.tokenType = "}";
    this.value = "}";
    this.mirror = "{";
  }
};
var OpenSquareToken = class extends GroupingToken {
  constructor() {
    super();
    this.tokenType = "[";
    this.value = "[";
    this.mirror = "]";
  }
};
var CloseSquareToken = class extends GroupingToken {
  constructor() {
    super();
    this.tokenType = "]";
    this.value = "]";
    this.mirror = "[";
  }
};
var OpenParenToken = class extends GroupingToken {
  constructor() {
    super();
    this.tokenType = "(";
    this.value = "(";
    this.mirror = ")";
  }
};
var CloseParenToken = class extends GroupingToken {
  constructor() {
    super();
    this.tokenType = ")";
    this.value = ")";
    this.mirror = "(";
  }
};
var IncludeMatchToken = class extends CSSParserToken {
  constructor() {
    super(...arguments);
    this.tokenType = "~=";
  }
};
var DashMatchToken = class extends CSSParserToken {
  constructor() {
    super(...arguments);
    this.tokenType = "|=";
  }
};
var PrefixMatchToken = class extends CSSParserToken {
  constructor() {
    super(...arguments);
    this.tokenType = "^=";
  }
};
var SuffixMatchToken = class extends CSSParserToken {
  constructor() {
    super(...arguments);
    this.tokenType = "$=";
  }
};
var SubstringMatchToken = class extends CSSParserToken {
  constructor() {
    super(...arguments);
    this.tokenType = "*=";
  }
};
var ColumnToken = class extends CSSParserToken {
  constructor() {
    super(...arguments);
    this.tokenType = "||";
  }
};
var EOFToken = class extends CSSParserToken {
  constructor() {
    super(...arguments);
    this.tokenType = "EOF";
  }
  toSource() {
    return "";
  }
};
var DelimToken = class extends CSSParserToken {
  constructor(code) {
    super();
    this.tokenType = "DELIM";
    this.value = "";
    this.value = stringFromCode(code);
  }
  toString() {
    return "DELIM(" + this.value + ")";
  }
  toJSON() {
    const json = this.constructor.prototype.constructor.prototype.toJSON.call(this);
    json.value = this.value;
    return json;
  }
  toSource() {
    if (this.value === "\\")
      return "\\\n";
    else
      return this.value;
  }
};
var StringValuedToken = class extends CSSParserToken {
  constructor() {
    super(...arguments);
    this.value = "";
  }
  ASCIIMatch(str) {
    return this.value.toLowerCase() === str.toLowerCase();
  }
  toJSON() {
    const json = this.constructor.prototype.constructor.prototype.toJSON.call(this);
    json.value = this.value;
    return json;
  }
};
var IdentToken = class extends StringValuedToken {
  constructor(val) {
    super();
    this.tokenType = "IDENT";
    this.value = val;
  }
  toString() {
    return "IDENT(" + this.value + ")";
  }
  toSource() {
    return escapeIdent(this.value);
  }
};
var FunctionToken = class extends StringValuedToken {
  constructor(val) {
    super();
    this.tokenType = "FUNCTION";
    this.value = val;
    this.mirror = ")";
  }
  toString() {
    return "FUNCTION(" + this.value + ")";
  }
  toSource() {
    return escapeIdent(this.value) + "(";
  }
};
var AtKeywordToken = class extends StringValuedToken {
  constructor(val) {
    super();
    this.tokenType = "AT-KEYWORD";
    this.value = val;
  }
  toString() {
    return "AT(" + this.value + ")";
  }
  toSource() {
    return "@" + escapeIdent(this.value);
  }
};
var HashToken = class extends StringValuedToken {
  constructor(val) {
    super();
    this.tokenType = "HASH";
    this.value = val;
    this.type = "unrestricted";
  }
  toString() {
    return "HASH(" + this.value + ")";
  }
  toJSON() {
    const json = this.constructor.prototype.constructor.prototype.toJSON.call(this);
    json.value = this.value;
    json.type = this.type;
    return json;
  }
  toSource() {
    if (this.type === "id")
      return "#" + escapeIdent(this.value);
    else
      return "#" + escapeHash(this.value);
  }
};
var StringToken = class extends StringValuedToken {
  constructor(val) {
    super();
    this.tokenType = "STRING";
    this.value = val;
  }
  toString() {
    return '"' + escapeString(this.value) + '"';
  }
};
var URLToken = class extends StringValuedToken {
  constructor(val) {
    super();
    this.tokenType = "URL";
    this.value = val;
  }
  toString() {
    return "URL(" + this.value + ")";
  }
  toSource() {
    return 'url("' + escapeString(this.value) + '")';
  }
};
var NumberToken = class extends CSSParserToken {
  constructor() {
    super();
    this.tokenType = "NUMBER";
    this.type = "integer";
    this.repr = "";
  }
  toString() {
    if (this.type === "integer")
      return "INT(" + this.value + ")";
    return "NUMBER(" + this.value + ")";
  }
  toJSON() {
    const json = super.toJSON();
    json.value = this.value;
    json.type = this.type;
    json.repr = this.repr;
    return json;
  }
  toSource() {
    return this.repr;
  }
};
var PercentageToken = class extends CSSParserToken {
  constructor() {
    super();
    this.tokenType = "PERCENTAGE";
    this.repr = "";
  }
  toString() {
    return "PERCENTAGE(" + this.value + ")";
  }
  toJSON() {
    const json = this.constructor.prototype.constructor.prototype.toJSON.call(this);
    json.value = this.value;
    json.repr = this.repr;
    return json;
  }
  toSource() {
    return this.repr + "%";
  }
};
var DimensionToken = class extends CSSParserToken {
  constructor() {
    super();
    this.tokenType = "DIMENSION";
    this.type = "integer";
    this.repr = "";
    this.unit = "";
  }
  toString() {
    return "DIM(" + this.value + "," + this.unit + ")";
  }
  toJSON() {
    const json = this.constructor.prototype.constructor.prototype.toJSON.call(this);
    json.value = this.value;
    json.type = this.type;
    json.repr = this.repr;
    json.unit = this.unit;
    return json;
  }
  toSource() {
    const source = this.repr;
    let unit = escapeIdent(this.unit);
    if (unit[0].toLowerCase() === "e" && (unit[1] === "-" || between(unit.charCodeAt(1), 48, 57))) {
      unit = "\\65 " + unit.slice(1, unit.length);
    }
    return source + unit;
  }
};
function escapeIdent(string) {
  string = "" + string;
  let result = "";
  const firstcode = string.charCodeAt(0);
  for (let i = 0; i < string.length; i++) {
    const code = string.charCodeAt(i);
    if (code === 0)
      throw new InvalidCharacterError("Invalid character: the input contains U+0000.");
    if (between(code, 1, 31) || code === 127 || i === 0 && between(code, 48, 57) || i === 1 && between(code, 48, 57) && firstcode === 45)
      result += "\\" + code.toString(16) + " ";
    else if (code >= 128 || code === 45 || code === 95 || between(code, 48, 57) || between(code, 65, 90) || between(code, 97, 122))
      result += string[i];
    else
      result += "\\" + string[i];
  }
  return result;
}
function escapeHash(string) {
  string = "" + string;
  let result = "";
  for (let i = 0; i < string.length; i++) {
    const code = string.charCodeAt(i);
    if (code === 0)
      throw new InvalidCharacterError("Invalid character: the input contains U+0000.");
    if (code >= 128 || code === 45 || code === 95 || between(code, 48, 57) || between(code, 65, 90) || between(code, 97, 122))
      result += string[i];
    else
      result += "\\" + code.toString(16) + " ";
  }
  return result;
}
function escapeString(string) {
  string = "" + string;
  let result = "";
  for (let i = 0; i < string.length; i++) {
    const code = string.charCodeAt(i);
    if (code === 0)
      throw new InvalidCharacterError("Invalid character: the input contains U+0000.");
    if (between(code, 1, 31) || code === 127)
      result += "\\" + code.toString(16) + " ";
    else if (code === 34 || code === 92)
      result += "\\" + string[i];
    else
      result += string[i];
  }
  return result;
}

// src/vendor/injected/roleUtils.ts
function hasExplicitAccessibleName(e) {
  return e.hasAttribute("aria-label") || e.hasAttribute("aria-labelledby");
}
var kAncestorPreventingLandmark = "article:not([role]), aside:not([role]), main:not([role]), nav:not([role]), section:not([role]), [role=article], [role=complementary], [role=main], [role=navigation], [role=region]";
var kGlobalAriaAttributes = [
  ["aria-atomic", void 0],
  ["aria-busy", void 0],
  ["aria-controls", void 0],
  ["aria-current", void 0],
  ["aria-describedby", void 0],
  ["aria-details", void 0],
  // Global use deprecated in ARIA 1.2
  // ['aria-disabled', undefined],
  ["aria-dropeffect", void 0],
  // Global use deprecated in ARIA 1.2
  // ['aria-errormessage', undefined],
  ["aria-flowto", void 0],
  ["aria-grabbed", void 0],
  // Global use deprecated in ARIA 1.2
  // ['aria-haspopup', undefined],
  ["aria-hidden", void 0],
  // Global use deprecated in ARIA 1.2
  // ['aria-invalid', undefined],
  ["aria-keyshortcuts", void 0],
  ["aria-label", ["caption", "code", "deletion", "emphasis", "generic", "insertion", "paragraph", "presentation", "strong", "subscript", "superscript"]],
  ["aria-labelledby", ["caption", "code", "deletion", "emphasis", "generic", "insertion", "paragraph", "presentation", "strong", "subscript", "superscript"]],
  ["aria-live", void 0],
  ["aria-owns", void 0],
  ["aria-relevant", void 0],
  ["aria-roledescription", ["generic"]]
];
function hasGlobalAriaAttribute(element, forRole) {
  return kGlobalAriaAttributes.some(([attr, prohibited]) => {
    return !(prohibited == null ? void 0 : prohibited.includes(forRole || "")) && element.hasAttribute(attr);
  });
}
function hasTabIndex(element) {
  return !Number.isNaN(Number(String(element.getAttribute("tabindex"))));
}
function isFocusable(element) {
  return !isNativelyDisabled2(element) && (isNativelyFocusable(element) || hasTabIndex(element));
}
function isNativelyFocusable(element) {
  const tagName = elementSafeTagName(element);
  if (["BUTTON", "DETAILS", "SELECT", "TEXTAREA"].includes(tagName))
    return true;
  if (tagName === "A" || tagName === "AREA")
    return element.hasAttribute("href");
  if (tagName === "INPUT")
    return !element.hidden;
  return false;
}
var kImplicitRoleByTagName = {
  "A": (e) => {
    return e.hasAttribute("href") ? "link" : null;
  },
  "AREA": (e) => {
    return e.hasAttribute("href") ? "link" : null;
  },
  "ARTICLE": () => "article",
  "ASIDE": () => "complementary",
  "BLOCKQUOTE": () => "blockquote",
  "BUTTON": () => "button",
  "CAPTION": () => "caption",
  "CODE": () => "code",
  "DATALIST": () => "listbox",
  "DD": () => "definition",
  "DEL": () => "deletion",
  "DETAILS": () => "group",
  "DFN": () => "term",
  "DIALOG": () => "dialog",
  "DT": () => "term",
  "EM": () => "emphasis",
  "FIELDSET": () => "group",
  "FIGURE": () => "figure",
  "FOOTER": (e) => closestCrossShadow(e, kAncestorPreventingLandmark) ? null : "contentinfo",
  "FORM": (e) => hasExplicitAccessibleName(e) ? "form" : null,
  "H1": () => "heading",
  "H2": () => "heading",
  "H3": () => "heading",
  "H4": () => "heading",
  "H5": () => "heading",
  "H6": () => "heading",
  "HEADER": (e) => closestCrossShadow(e, kAncestorPreventingLandmark) ? null : "banner",
  "HR": () => "separator",
  "HTML": () => "document",
  "IMG": (e) => e.getAttribute("alt") === "" && !e.getAttribute("title") && !hasGlobalAriaAttribute(e) && !hasTabIndex(e) ? "presentation" : "img",
  "INPUT": (e) => {
    const type = e.type.toLowerCase();
    if (["email", "search", "tel", "text", "url", ""].includes(type)) {
      const list = getIdRefs(e, e.getAttribute("list"))[0];
      if (list && elementSafeTagName(list) === "DATALIST")
        return "combobox";
      return type === "search" ? "searchbox" : "textbox";
    }
    if (type === "hidden")
      return null;
    if (type === "file")
      return "button";
    return inputTypeToRole[type] || "textbox";
  },
  "INS": () => "insertion",
  "LI": () => "listitem",
  "MAIN": () => "main",
  "MARK": () => "mark",
  "MATH": () => "math",
  "MENU": () => "list",
  "METER": () => "meter",
  "NAV": () => "navigation",
  "OL": () => "list",
  "OPTGROUP": () => "group",
  "OPTION": () => "option",
  "OUTPUT": () => "status",
  "P": () => "paragraph",
  "PROGRESS": () => "progressbar",
  "SEARCH": () => "search",
  "SECTION": (e) => hasExplicitAccessibleName(e) ? "region" : null,
  "SELECT": (e) => e.hasAttribute("multiple") || e.size > 1 ? "listbox" : "combobox",
  "STRONG": () => "strong",
  "SUB": () => "subscript",
  "SUP": () => "superscript",
  // For <svg> we default to Chrome behavior:
  // - Chrome reports 'img'.
  // - Firefox reports 'diagram' that is not in official ARIA spec yet.
  // - Safari reports 'no role', but still computes accessible name.
  "SVG": () => "img",
  "TABLE": () => "table",
  "TBODY": () => "rowgroup",
  "TD": (e) => {
    const table = closestCrossShadow(e, "table");
    const role = table ? getExplicitAriaRole(table) : "";
    return role === "grid" || role === "treegrid" ? "gridcell" : "cell";
  },
  "TEXTAREA": () => "textbox",
  "TFOOT": () => "rowgroup",
  "TH": (e) => {
    const scope = e.getAttribute("scope");
    if (scope === "col" || scope === "colgroup")
      return "columnheader";
    if (scope === "row" || scope === "rowgroup")
      return "rowheader";
    const nextSibling = e.nextElementSibling;
    const prevSibling = e.previousElementSibling;
    const row = !!e.parentElement && elementSafeTagName(e.parentElement) === "TR" ? e.parentElement : void 0;
    if (!nextSibling && !prevSibling) {
      if (row) {
        const table = closestCrossShadow(row, "table");
        if (table && table.rows.length <= 1)
          return null;
      }
      return "columnheader";
    }
    if (isHeaderCell(nextSibling) && isHeaderCell(prevSibling))
      return "columnheader";
    if (isNonEmptyDataCell(nextSibling) || isNonEmptyDataCell(prevSibling))
      return "rowheader";
    return "columnheader";
  },
  "THEAD": () => "rowgroup",
  "TIME": () => "time",
  "TR": () => "row",
  "UL": () => "list"
};
function isHeaderCell(element) {
  return !!element && elementSafeTagName(element) === "TH";
}
function isNonEmptyDataCell(element) {
  var _a;
  if (!element || elementSafeTagName(element) !== "TD")
    return false;
  return !!(((_a = element.textContent) == null ? void 0 : _a.trim()) || element.children.length > 0);
}
var kPresentationInheritanceParents = {
  "DD": ["DL", "DIV"],
  "DIV": ["DL"],
  "DT": ["DL", "DIV"],
  "LI": ["OL", "UL"],
  "TBODY": ["TABLE"],
  "TD": ["TR"],
  "TFOOT": ["TABLE"],
  "TH": ["TR"],
  "THEAD": ["TABLE"],
  "TR": ["THEAD", "TBODY", "TFOOT", "TABLE"]
};
function getImplicitAriaRole(element) {
  var _a;
  const implicitRole = ((_a = kImplicitRoleByTagName[elementSafeTagName(element)]) == null ? void 0 : _a.call(kImplicitRoleByTagName, element)) || "";
  if (!implicitRole)
    return null;
  let ancestor = element;
  while (ancestor) {
    const parent = parentElementOrShadowHost(ancestor);
    const parents = kPresentationInheritanceParents[elementSafeTagName(ancestor)];
    if (!parents || !parent || !parents.includes(elementSafeTagName(parent)))
      break;
    const parentExplicitRole = getExplicitAriaRole(parent);
    if ((parentExplicitRole === "none" || parentExplicitRole === "presentation") && !hasPresentationConflictResolution(parent, parentExplicitRole))
      return parentExplicitRole;
    ancestor = parent;
  }
  return implicitRole;
}
var validRoles = [
  "alert",
  "alertdialog",
  "application",
  "article",
  "banner",
  "blockquote",
  "button",
  "caption",
  "cell",
  "checkbox",
  "code",
  "columnheader",
  "combobox",
  "complementary",
  "contentinfo",
  "definition",
  "deletion",
  "dialog",
  "directory",
  "document",
  "emphasis",
  "feed",
  "figure",
  "form",
  "generic",
  "grid",
  "gridcell",
  "group",
  "heading",
  "img",
  "insertion",
  "link",
  "list",
  "listbox",
  "listitem",
  "log",
  "main",
  "mark",
  "marquee",
  "math",
  "meter",
  "menu",
  "menubar",
  "menuitem",
  "menuitemcheckbox",
  "menuitemradio",
  "navigation",
  "none",
  "note",
  "option",
  "paragraph",
  "presentation",
  "progressbar",
  "radio",
  "radiogroup",
  "region",
  "row",
  "rowgroup",
  "rowheader",
  "scrollbar",
  "search",
  "searchbox",
  "separator",
  "slider",
  "spinbutton",
  "status",
  "strong",
  "subscript",
  "superscript",
  "switch",
  "tab",
  "table",
  "tablist",
  "tabpanel",
  "term",
  "textbox",
  "time",
  "timer",
  "toolbar",
  "tooltip",
  "tree",
  "treegrid",
  "treeitem"
];
function getExplicitAriaRole(element) {
  const roles = (element.getAttribute("role") || "").split(" ").map((role) => role.trim());
  return roles.find((role) => validRoles.includes(role)) || null;
}
function hasPresentationConflictResolution(element, role) {
  return hasGlobalAriaAttribute(element, role) || isFocusable(element);
}
function getAriaRole(element) {
  const cached = cacheAriaRole == null ? void 0 : cacheAriaRole.get(element);
  if (cached !== void 0)
    return cached;
  const role = computeAriaRole(element);
  cacheAriaRole == null ? void 0 : cacheAriaRole.set(element, role);
  return role;
}
function computeAriaRole(element) {
  const explicitRole = getExplicitAriaRole(element);
  if (!explicitRole)
    return getImplicitAriaRole(element);
  if (explicitRole === "none" || explicitRole === "presentation") {
    const implicitRole = getImplicitAriaRole(element);
    if (hasPresentationConflictResolution(element, implicitRole))
      return implicitRole;
  }
  return explicitRole;
}
function getAriaBoolean(attr) {
  return attr === null ? void 0 : attr.toLowerCase() === "true";
}
function isElementIgnoredForAria(element) {
  return ["STYLE", "SCRIPT", "NOSCRIPT", "TEMPLATE"].includes(elementSafeTagName(element));
}
function isElementHiddenForAria(element) {
  if (isElementIgnoredForAria(element))
    return true;
  const style = getElementComputedStyle(element);
  const isSlot = element.nodeName === "SLOT";
  if ((style == null ? void 0 : style.display) === "contents" && !isSlot) {
    for (let child = element.firstChild; child; child = child.nextSibling) {
      if (child.nodeType === 1 && !isElementHiddenForAria(child))
        return false;
      if (child.nodeType === 3 && isVisibleTextNode(child))
        return false;
    }
    return true;
  }
  const isOptionInsideSelect = element.nodeName === "OPTION" && !!element.closest("select");
  if (!isOptionInsideSelect && !isSlot && !isElementStyleVisibilityVisible(element, style))
    return true;
  return belongsToDisplayNoneOrAriaHiddenOrNonSlotted(element);
}
function belongsToDisplayNoneOrAriaHiddenOrNonSlotted(element) {
  let hidden = cacheIsHidden == null ? void 0 : cacheIsHidden.get(element);
  if (hidden === void 0) {
    hidden = false;
    if (element.parentElement && element.parentElement.shadowRoot && !element.assignedSlot)
      hidden = true;
    if (!hidden) {
      const style = getElementComputedStyle(element);
      hidden = !style || style.display === "none" || getAriaBoolean(element.getAttribute("aria-hidden")) === true;
    }
    if (!hidden) {
      const parent = parentElementOrShadowHost(element);
      if (parent)
        hidden = belongsToDisplayNoneOrAriaHiddenOrNonSlotted(parent);
    }
    cacheIsHidden == null ? void 0 : cacheIsHidden.set(element, hidden);
  }
  return hidden;
}
function getIdRefs(element, ref) {
  if (!ref)
    return [];
  const root = enclosingShadowRootOrDocument(element);
  if (!root)
    return [];
  try {
    const ids = ref.split(" ").filter((id) => !!id);
    const result = [];
    for (const id of ids) {
      const firstElement = root.querySelector("#" + CSS.escape(id));
      if (firstElement && !result.includes(firstElement))
        result.push(firstElement);
    }
    return result;
  } catch (e) {
    return [];
  }
}
function trimFlatString(s) {
  return s.trim();
}
function asFlatString(s) {
  return s.split(" ").map((chunk) => chunk.replace(/\r\n/g, "\n").replace(/[\u200b\u00ad]/g, "").replace(/\s\s*/g, " ")).join(" ").trim();
}
function queryInAriaOwned(element, selector) {
  const result = [...element.querySelectorAll(selector)];
  for (const owned of getIdRefs(element, element.getAttribute("aria-owns"))) {
    if (owned.matches(selector))
      result.push(owned);
    result.push(...owned.querySelectorAll(selector));
  }
  return result;
}
function getCSSContent(element, pseudo) {
  const cache = pseudo === "::before" ? cachePseudoContentBefore : pseudo === "::after" ? cachePseudoContentAfter : cachePseudoContent;
  if (cache == null ? void 0 : cache.has(element))
    return cache == null ? void 0 : cache.get(element);
  const style = getElementComputedStyle(element, pseudo);
  let content;
  if (style) {
    const contentValue = style.content;
    if (contentValue && contentValue !== "none" && contentValue !== "normal") {
      if (style.display !== "none" && style.visibility !== "hidden") {
        content = parseCSSContentPropertyAsString(element, contentValue, !!pseudo);
      }
    }
  }
  if (pseudo && content !== void 0) {
    const display = (style == null ? void 0 : style.display) || "inline";
    if (display !== "inline")
      content = " " + content + " ";
  }
  if (cache)
    cache.set(element, content);
  return content;
}
function parseCSSContentPropertyAsString(element, content, isPseudo) {
  if (!content || content === "none" || content === "normal") {
    return;
  }
  try {
    let tokens = tokenize(content).filter((token) => !(token instanceof WhitespaceToken));
    const delimIndex = tokens.findIndex((token) => token instanceof DelimToken && token.value === "/");
    if (delimIndex !== -1) {
      tokens = tokens.slice(delimIndex + 1);
    } else if (!isPseudo) {
      return;
    }
    const accumulated = [];
    let index = 0;
    while (index < tokens.length) {
      if (tokens[index] instanceof StringToken) {
        accumulated.push(tokens[index].value);
        index++;
      } else if (index + 2 < tokens.length && tokens[index] instanceof FunctionToken && tokens[index].value === "attr" && tokens[index + 1] instanceof IdentToken && tokens[index + 2] instanceof CloseParenToken) {
        const attrName = tokens[index + 1].value;
        accumulated.push(element.getAttribute(attrName) || "");
        index += 3;
      } else {
        return;
      }
    }
    return accumulated.join("");
  } catch {
  }
}
function getAriaLabelledByElements(element) {
  const ref = element.getAttribute("aria-labelledby");
  if (ref === null)
    return null;
  const refs = getIdRefs(element, ref);
  return refs.length ? refs : null;
}
function allowsNameFromContent(role, targetDescendant) {
  const alwaysAllowsNameFromContent = ["button", "cell", "checkbox", "columnheader", "gridcell", "heading", "link", "menuitem", "menuitemcheckbox", "menuitemradio", "option", "radio", "row", "rowheader", "switch", "tab", "tooltip", "treeitem"].includes(role);
  const descendantAllowsNameFromContent = targetDescendant && ["", "caption", "code", "contentinfo", "definition", "deletion", "emphasis", "insertion", "list", "listitem", "mark", "none", "paragraph", "presentation", "region", "row", "rowgroup", "section", "strong", "subscript", "superscript", "table", "term", "time"].includes(role);
  return alwaysAllowsNameFromContent || descendantAllowsNameFromContent;
}
function computeAccessibleNameComposite(element, includeHidden, collectElements) {
  const elementProhibitsNaming = ["caption", "code", "definition", "deletion", "emphasis", "generic", "insertion", "mark", "paragraph", "presentation", "strong", "subscript", "suggestion", "superscript", "term", "time"].includes(getAriaRole(element) || "");
  if (elementProhibitsNaming)
    return { ...emptyCompositeString(), derivedFromContent: false };
  const outDerivedFromContent = { value: false };
  const result = getTextAlternativeInternal(element, {
    includeHidden,
    collectElements,
    outDerivedFromContent,
    visitedElements: /* @__PURE__ */ new Set(),
    embeddedInTargetElement: "self"
  });
  return { text: asFlatString(result.text), elements: result.elements, derivedFromContent: outDerivedFromContent.value };
}
function getElementAccessibleName(element, includeHidden) {
  const cache = includeHidden ? cacheAccessibleNameHidden : cacheAccessibleName;
  let accessibleName = cache == null ? void 0 : cache.get(element);
  if (accessibleName === void 0) {
    accessibleName = computeAccessibleNameComposite(
      element,
      includeHidden,
      true
      /* collectElements */
    );
    cache == null ? void 0 : cache.set(element, accessibleName);
  }
  return accessibleName;
}
function getElementAccessibleNameText(element, includeHidden) {
  var _a;
  const composite = (_a = includeHidden ? cacheAccessibleNameHidden : cacheAccessibleName) == null ? void 0 : _a.get(element);
  if (composite !== void 0)
    return composite.text;
  const cache = includeHidden ? cacheAccessibleNameTextHidden : cacheAccessibleNameText;
  let text = cache == null ? void 0 : cache.get(element);
  if (text === void 0) {
    text = computeAccessibleNameComposite(
      element,
      includeHidden,
      false
      /* collectElements */
    ).text;
    cache == null ? void 0 : cache.set(element, text);
  }
  return text;
}
function getElementAccessibleDescription(element, includeHidden) {
  const cache = includeHidden ? cacheAccessibleDescriptionHidden : cacheAccessibleDescription;
  let accessibleDescription = cache == null ? void 0 : cache.get(element);
  if (accessibleDescription === void 0) {
    accessibleDescription = { text: "", derivedFromContent: false };
    if (element.hasAttribute("aria-describedby")) {
      const describedBy = getIdRefs(element, element.getAttribute("aria-describedby"));
      accessibleDescription.text = asFlatString(describedBy.map((ref) => getTextAlternativeInternal(ref, {
        includeHidden,
        visitedElements: /* @__PURE__ */ new Set(),
        embeddedInDescribedBy: { element: ref, hidden: isElementHiddenForAria(ref) }
      }).text).join(" "));
      accessibleDescription.derivedFromContent = describedBy.some((ref) => ref === element || element.contains(ref));
    } else if (element.hasAttribute("aria-description")) {
      accessibleDescription.text = asFlatString(element.getAttribute("aria-description") || "");
    } else {
      accessibleDescription.text = asFlatString(element.getAttribute("title") || "");
    }
    cache == null ? void 0 : cache.set(element, accessibleDescription);
  }
  return accessibleDescription;
}
var kAriaInvalidRoles = [
  "application",
  "checkbox",
  "columnheader",
  "combobox",
  "gridcell",
  "listbox",
  "radiogroup",
  "rowheader",
  "searchbox",
  "slider",
  "spinbutton",
  "switch",
  "textbox",
  "tree"
];
function getAriaInvalid(element) {
  const ariaInvalid = element.getAttribute("aria-invalid");
  if (!ariaInvalid || ariaInvalid.trim() === "" || ariaInvalid.toLocaleLowerCase() === "false")
    return "false";
  if (ariaInvalid === "true" || ariaInvalid === "grammar" || ariaInvalid === "spelling")
    return ariaInvalid;
  return "true";
}
function insideTargetElement(options) {
  return options.embeddedInTargetElement === "self" || options.embeddedInTargetElement === "descendant";
}
function getTextAlternativeInternal(element, options) {
  var _a, _b, _c, _d, _e;
  if (options.visitedElements.has(element))
    return emptyCompositeString();
  const childOptions = {
    ...options,
    embeddedInTargetElement: options.embeddedInTargetElement === "self" ? "descendant" : options.embeddedInTargetElement
  };
  if (!options.includeHidden) {
    const isEmbeddedInHiddenReferenceTraversal = !!((_a = options.embeddedInLabelledBy) == null ? void 0 : _a.hidden) || !!((_b = options.embeddedInDescribedBy) == null ? void 0 : _b.hidden) || !!((_c = options.embeddedInNativeTextAlternative) == null ? void 0 : _c.hidden) || !!((_d = options.embeddedInLabel) == null ? void 0 : _d.hidden);
    if (isElementIgnoredForAria(element) || !isEmbeddedInHiddenReferenceTraversal && isElementHiddenForAria(element)) {
      options.visitedElements.add(element);
      return emptyCompositeString();
    }
  }
  const labelledBy = getAriaLabelledByElements(element);
  if (!options.embeddedInLabelledBy) {
    const accessibleName = joinCompositeString((labelledBy || []).map((ref) => getTextAlternativeInternal(ref, {
      ...options,
      embeddedInLabelledBy: { element: ref, hidden: isElementHiddenForAria(ref) },
      embeddedInDescribedBy: void 0,
      embeddedInTargetElement: void 0,
      embeddedInLabel: void 0,
      embeddedInNativeTextAlternative: void 0
    })), " ", options.collectElements);
    if (accessibleName.text) {
      if (options.outDerivedFromContent && insideTargetElement(options) && (labelledBy || []).some((ref) => ref === element || element.contains(ref)))
        options.outDerivedFromContent.value = true;
      return accessibleName;
    }
  }
  const role = getAriaRole(element) || "";
  const tagName = elementSafeTagName(element);
  if (!!options.embeddedInLabel || !!options.embeddedInLabelledBy || options.embeddedInTargetElement === "descendant") {
    const isOwnLabel = [...element.labels || []].includes(element);
    const isOwnLabelledBy = (labelledBy || []).includes(element);
    if (!isOwnLabel && !isOwnLabelledBy) {
      if (role === "textbox") {
        options.visitedElements.add(element);
        if (tagName === "INPUT" || tagName === "TEXTAREA")
          return compositeString(element.value, element, options.collectElements);
        return compositeString(element.textContent, element, options.collectElements);
      }
      if (["combobox", "listbox"].includes(role)) {
        options.visitedElements.add(element);
        let selectedOptions;
        if (tagName === "SELECT") {
          selectedOptions = [...element.selectedOptions];
          if (!selectedOptions.length && element.options.length)
            selectedOptions.push(element.options[0]);
        } else {
          const listbox = role === "combobox" ? queryInAriaOwned(element, "*").find((e) => getAriaRole(e) === "listbox") : element;
          selectedOptions = listbox ? queryInAriaOwned(listbox, '[aria-selected="true"]').filter((e) => getAriaRole(e) === "option") : [];
        }
        if (!selectedOptions.length && tagName === "INPUT") {
          return compositeString(element.value, element, options.collectElements);
        }
        return joinCompositeString(selectedOptions.map((option) => getTextAlternativeInternal(option, childOptions)), " ", options.collectElements);
      }
      if (["progressbar", "scrollbar", "slider", "spinbutton", "meter"].includes(role)) {
        options.visitedElements.add(element);
        if (element.hasAttribute("aria-valuetext"))
          return compositeString(element.getAttribute("aria-valuetext"), element, options.collectElements);
        if (element.hasAttribute("aria-valuenow"))
          return compositeString(element.getAttribute("aria-valuenow"), element, options.collectElements);
        return compositeString(element.getAttribute("value"), element, options.collectElements);
      }
      if (["menu"].includes(role)) {
        options.visitedElements.add(element);
        return emptyCompositeString();
      }
    }
  }
  const ariaLabel = element.getAttribute("aria-label") || "";
  if (trimFlatString(ariaLabel)) {
    options.visitedElements.add(element);
    return compositeString(ariaLabel, element, options.collectElements);
  }
  if (!["presentation", "none"].includes(role)) {
    if (tagName === "INPUT" && ["button", "submit", "reset"].includes(element.type)) {
      options.visitedElements.add(element);
      const value = element.value || "";
      if (trimFlatString(value))
        return compositeString(value, element, options.collectElements);
      if (element.type === "submit")
        return compositeString("Submit", element, options.collectElements);
      if (element.type === "reset")
        return compositeString("Reset", element, options.collectElements);
      const title = element.getAttribute("title") || "";
      return compositeString(title, element, options.collectElements);
    }
    if (tagName === "INPUT" && element.type === "file") {
      options.visitedElements.add(element);
      const labels = element.labels || [];
      if (labels.length && !options.embeddedInLabelledBy)
        return getAccessibleNameFromAssociatedLabels(labels, options);
      return compositeString("Choose File", element, options.collectElements);
    }
    if (tagName === "INPUT" && element.type === "image") {
      options.visitedElements.add(element);
      const labels = element.labels || [];
      if (labels.length && !options.embeddedInLabelledBy)
        return getAccessibleNameFromAssociatedLabels(labels, options);
      const alt = element.getAttribute("alt") || "";
      if (trimFlatString(alt))
        return compositeString(alt, element, options.collectElements);
      const title = element.getAttribute("title") || "";
      if (trimFlatString(title))
        return compositeString(title, element, options.collectElements);
      return compositeString("Submit", element, options.collectElements);
    }
    if (!labelledBy && tagName === "BUTTON") {
      options.visitedElements.add(element);
      const labels = element.labels || [];
      if (labels.length)
        return getAccessibleNameFromAssociatedLabels(labels, options);
    }
    if (!labelledBy && tagName === "OUTPUT") {
      options.visitedElements.add(element);
      const labels = element.labels || [];
      if (labels.length)
        return getAccessibleNameFromAssociatedLabels(labels, options);
      return compositeString(element.getAttribute("title") || "", element, options.collectElements);
    }
    if (!labelledBy && (tagName === "TEXTAREA" || tagName === "SELECT" || tagName === "INPUT" || tagName === "METER" || tagName === "PROGRESS")) {
      options.visitedElements.add(element);
      const labels = element.labels || [];
      if (labels.length)
        return getAccessibleNameFromAssociatedLabels(labels, options);
      const usePlaceholder = tagName === "INPUT" && ["text", "password", "number", "search", "tel", "email", "url"].includes(element.type) || tagName === "TEXTAREA";
      const placeholder = element.getAttribute("placeholder") || "";
      const title = element.getAttribute("title") || "";
      if (!usePlaceholder || title)
        return compositeString(title, element, options.collectElements);
      return compositeString(placeholder, element, options.collectElements);
    }
    if (!labelledBy && tagName === "FIELDSET") {
      options.visitedElements.add(element);
      for (let child = element.firstElementChild; child; child = child.nextElementSibling) {
        if (elementSafeTagName(child) === "LEGEND") {
          return getTextAlternativeInternal(child, {
            ...childOptions,
            embeddedInNativeTextAlternative: { element: child, hidden: isElementHiddenForAria(child) }
          });
        }
      }
      const title = element.getAttribute("title") || "";
      return compositeString(title, element, options.collectElements);
    }
    if (!labelledBy && tagName === "FIGURE") {
      options.visitedElements.add(element);
      for (let child = element.firstElementChild; child; child = child.nextElementSibling) {
        if (elementSafeTagName(child) === "FIGCAPTION") {
          return getTextAlternativeInternal(child, {
            ...childOptions,
            embeddedInNativeTextAlternative: { element: child, hidden: isElementHiddenForAria(child) }
          });
        }
      }
      const title = element.getAttribute("title") || "";
      return compositeString(title, element, options.collectElements);
    }
    if (tagName === "IMG") {
      options.visitedElements.add(element);
      const alt = element.getAttribute("alt") || "";
      if (trimFlatString(alt))
        return compositeString(alt, element, options.collectElements);
      const title = element.getAttribute("title") || "";
      return compositeString(title, element, options.collectElements);
    }
    if (tagName === "TABLE") {
      options.visitedElements.add(element);
      for (let child = element.firstElementChild; child; child = child.nextElementSibling) {
        if (elementSafeTagName(child) === "CAPTION") {
          return getTextAlternativeInternal(child, {
            ...childOptions,
            embeddedInNativeTextAlternative: { element: child, hidden: isElementHiddenForAria(child) }
          });
        }
      }
      const summary = element.getAttribute("summary") || "";
      if (summary)
        return compositeString(summary, element, options.collectElements);
    }
    if (tagName === "AREA") {
      options.visitedElements.add(element);
      const alt = element.getAttribute("alt") || "";
      if (trimFlatString(alt))
        return compositeString(alt, element, options.collectElements);
      const title = element.getAttribute("title") || "";
      return compositeString(title, element, options.collectElements);
    }
    if (tagName === "SVG" || element.ownerSVGElement) {
      options.visitedElements.add(element);
      for (let child = element.firstElementChild; child; child = child.nextElementSibling) {
        if (elementSafeTagName(child) === "TITLE" && child.ownerSVGElement) {
          return getTextAlternativeInternal(child, {
            ...childOptions,
            embeddedInLabelledBy: { element: child, hidden: isElementHiddenForAria(child) }
          });
        }
      }
    }
    if (element.ownerSVGElement && tagName === "A") {
      const title = element.getAttribute("xlink:title") || "";
      if (trimFlatString(title)) {
        options.visitedElements.add(element);
        return compositeString(title, element, options.collectElements);
      }
    }
  }
  const shouldNameFromContentForSummary = tagName === "SUMMARY" && !["presentation", "none"].includes(role);
  if (allowsNameFromContent(role, options.embeddedInTargetElement === "descendant") || shouldNameFromContentForSummary || !!options.embeddedInLabelledBy || !!options.embeddedInDescribedBy || !!options.embeddedInLabel || !!options.embeddedInNativeTextAlternative) {
    options.visitedElements.add(element);
    const accessibleName = innerAccumulatedElementText(element, childOptions);
    const maybeTrimmedAccessibleName = options.embeddedInTargetElement === "self" ? trimFlatString(accessibleName.text) : accessibleName.text;
    if (maybeTrimmedAccessibleName) {
      if (options.outDerivedFromContent && insideTargetElement(options) && trimFlatString(accessibleName.text))
        options.outDerivedFromContent.value = true;
      (_e = accessibleName.elements) == null ? void 0 : _e.add(element);
      return accessibleName;
    }
  }
  if (!["presentation", "none"].includes(role) || tagName === "IFRAME" || tagName === "FRAME") {
    options.visitedElements.add(element);
    const title = element.getAttribute("title") || "";
    if (trimFlatString(title))
      return compositeString(title, element, options.collectElements);
  }
  options.visitedElements.add(element);
  return emptyCompositeString();
}
function innerAccumulatedElementText(element, options) {
  const tokens = [];
  const elements = options.collectElements ? /* @__PURE__ */ new Set() : void 0;
  const visit = (node, skipSlotted) => {
    var _a;
    if (skipSlotted && node.assignedSlot)
      return;
    if (node.nodeType === 1) {
      const display = ((_a = getElementComputedStyle(node)) == null ? void 0 : _a.display) || "inline";
      const childComposite = getTextAlternativeInternal(node, options);
      let token = childComposite.text;
      for (const contributor of childComposite.elements || [])
        elements == null ? void 0 : elements.add(contributor);
      if (display !== "inline" || node.nodeName === "BR")
        token = " " + token + " ";
      tokens.push(token);
    } else if (node.nodeType === 3) {
      tokens.push(node.textContent || "");
    }
  };
  tokens.push(getCSSContent(element, "::before") || "");
  const content = getCSSContent(element);
  if (content !== void 0) {
    tokens.push(content);
  } else {
    const assignedNodes = element.nodeName === "SLOT" ? element.assignedNodes() : [];
    if (assignedNodes.length) {
      for (const child of assignedNodes)
        visit(child, false);
    } else {
      for (let child = element.firstChild; child; child = child.nextSibling)
        visit(child, true);
      if (element.shadowRoot) {
        for (let child = element.shadowRoot.firstChild; child; child = child.nextSibling)
          visit(child, true);
      }
      for (const owned of getIdRefs(element, element.getAttribute("aria-owns")))
        visit(owned, true);
    }
  }
  tokens.push(getCSSContent(element, "::after") || "");
  return { text: tokens.join(""), elements };
}
var kAriaSelectedRoles = ["gridcell", "option", "row", "tab", "rowheader", "columnheader", "treeitem"];
function getAriaSelected(element) {
  if (elementSafeTagName(element) === "OPTION")
    return element.selected;
  if (kAriaSelectedRoles.includes(getAriaRole(element) || ""))
    return getAriaBoolean(element.getAttribute("aria-selected")) === true;
  return false;
}
var kAriaCheckedRoles = ["checkbox", "menuitemcheckbox", "option", "radio", "switch", "menuitemradio", "treeitem"];
function getAriaChecked(element) {
  const result = getChecked(element, true);
  return result === "error" ? false : result;
}
function getChecked(element, allowMixed) {
  const tagName = elementSafeTagName(element);
  if (allowMixed && tagName === "INPUT" && element.indeterminate)
    return "mixed";
  if (tagName === "INPUT" && ["checkbox", "radio"].includes(element.type))
    return element.checked;
  if (kAriaCheckedRoles.includes(getAriaRole(element) || "")) {
    const checked = element.getAttribute("aria-checked");
    if (checked === "true")
      return true;
    if (allowMixed && checked === "mixed")
      return "mixed";
    return false;
  }
  return "error";
}
var kAriaPressedRoles = ["button"];
function getAriaPressed(element) {
  if (kAriaPressedRoles.includes(getAriaRole(element) || "")) {
    const pressed = element.getAttribute("aria-pressed");
    if (pressed === "true")
      return true;
    if (pressed === "mixed")
      return "mixed";
  }
  return false;
}
var kAriaExpandedRoles = ["application", "button", "checkbox", "combobox", "gridcell", "link", "listbox", "menuitem", "row", "rowheader", "tab", "treeitem", "columnheader", "menuitemcheckbox", "menuitemradio", "rowheader", "switch"];
function getAriaExpanded(element) {
  if (elementSafeTagName(element) === "DETAILS")
    return element.open;
  if (kAriaExpandedRoles.includes(getAriaRole(element) || "")) {
    const expanded = element.getAttribute("aria-expanded");
    if (expanded === null)
      return void 0;
    if (expanded === "true")
      return true;
    return false;
  }
  return void 0;
}
var kAriaLevelRoles = ["heading", "listitem", "row", "treeitem"];
function getAriaLevel(element) {
  const native = { "H1": 1, "H2": 2, "H3": 3, "H4": 4, "H5": 5, "H6": 6 }[elementSafeTagName(element)];
  if (native)
    return native;
  if (kAriaLevelRoles.includes(getAriaRole(element) || "")) {
    const attr = element.getAttribute("aria-level");
    const value = attr === null ? Number.NaN : Number(attr);
    if (Number.isInteger(value) && value >= 1)
      return value;
  }
  return 0;
}
var kAriaDisabledRoles = ["application", "button", "composite", "gridcell", "group", "input", "link", "menuitem", "scrollbar", "separator", "tab", "checkbox", "columnheader", "combobox", "grid", "listbox", "menu", "menubar", "menuitemcheckbox", "menuitemradio", "option", "radio", "radiogroup", "row", "rowheader", "searchbox", "select", "slider", "spinbutton", "switch", "tablist", "textbox", "toolbar", "tree", "treegrid", "treeitem"];
function getAriaDisabled2(element) {
  return isNativelyDisabled2(element) || hasExplicitAriaDisabled(element);
}
function isNativelyDisabled2(element) {
  const isNativeFormControl = ["BUTTON", "INPUT", "SELECT", "TEXTAREA", "OPTION", "OPTGROUP"].includes(elementSafeTagName(element));
  return isNativeFormControl && (element.hasAttribute("disabled") || belongsToDisabledOptGroup2(element) || belongsToDisabledFieldSet2(element));
}
function belongsToDisabledOptGroup2(element) {
  return elementSafeTagName(element) === "OPTION" && !!element.closest("OPTGROUP[DISABLED]");
}
function belongsToDisabledFieldSet2(element) {
  const fieldSetElement = element == null ? void 0 : element.closest("FIELDSET[DISABLED]");
  if (!fieldSetElement)
    return false;
  const legendElement = fieldSetElement.querySelector(":scope > LEGEND");
  return !legendElement || !legendElement.contains(element);
}
function hasExplicitAriaDisabled(element) {
  if (!kAriaDisabledRoles.includes(getAriaRole(element) || ""))
    return false;
  return hasAriaDisabledInChain2(element);
}
function hasAriaDisabledInChain2(element) {
  let result = cacheAriaDisabled == null ? void 0 : cacheAriaDisabled.get(element);
  if (result === void 0) {
    const attribute = (element.getAttribute("aria-disabled") || "").toLowerCase();
    if (attribute === "true") {
      result = true;
    } else if (attribute === "false") {
      result = false;
    } else {
      const parent = parentElementOrShadowHost(element);
      result = parent ? hasAriaDisabledInChain2(parent) : false;
    }
    cacheAriaDisabled == null ? void 0 : cacheAriaDisabled.set(element, result);
  }
  return result;
}
function getAccessibleNameFromAssociatedLabels(labels, options) {
  return joinCompositeString([...labels].map((label) => getTextAlternativeInternal(label, {
    ...options,
    embeddedInLabel: { element: label, hidden: isElementHiddenForAria(label) },
    embeddedInNativeTextAlternative: void 0,
    embeddedInLabelledBy: void 0,
    embeddedInDescribedBy: void 0,
    embeddedInTargetElement: void 0
  })).filter((accessibleName) => !!accessibleName.text), " ", options.collectElements);
}
function receivesPointerEvents(element) {
  const cache = cachePointerEvents;
  let e = element;
  let result;
  const parents = [];
  for (; e; e = parentElementOrShadowHost(e)) {
    const cached = cache.get(e);
    if (cached !== void 0) {
      result = cached;
      break;
    }
    parents.push(e);
    const style = getElementComputedStyle(e);
    if (!style) {
      result = true;
      break;
    }
    const value = style.pointerEvents;
    if (value) {
      result = value !== "none";
      break;
    }
  }
  if (result === void 0)
    result = true;
  for (const parent of parents)
    cache.set(parent, result);
  return result;
}
var cacheAccessibleName;
var cacheAccessibleNameHidden;
var cacheAccessibleNameText;
var cacheAccessibleNameTextHidden;
var cacheAccessibleDescription;
var cacheAccessibleDescriptionHidden;
var cacheAccessibleErrorMessage;
var cacheIsHidden;
var cachePseudoContent;
var cachePseudoContentBefore;
var cachePseudoContentAfter;
var cachePointerEvents;
var cacheAriaRole;
var cacheAriaDisabled;
var cachesCounter2 = 0;
function beginAriaCaches() {
  beginDOMCaches();
  ++cachesCounter2;
  cacheAriaRole != null ? cacheAriaRole : cacheAriaRole = /* @__PURE__ */ new Map();
  cacheAriaDisabled != null ? cacheAriaDisabled : cacheAriaDisabled = /* @__PURE__ */ new Map();
  cacheAccessibleName != null ? cacheAccessibleName : cacheAccessibleName = /* @__PURE__ */ new Map();
  cacheAccessibleNameHidden != null ? cacheAccessibleNameHidden : cacheAccessibleNameHidden = /* @__PURE__ */ new Map();
  cacheAccessibleNameText != null ? cacheAccessibleNameText : cacheAccessibleNameText = /* @__PURE__ */ new Map();
  cacheAccessibleNameTextHidden != null ? cacheAccessibleNameTextHidden : cacheAccessibleNameTextHidden = /* @__PURE__ */ new Map();
  cacheAccessibleDescription != null ? cacheAccessibleDescription : cacheAccessibleDescription = /* @__PURE__ */ new Map();
  cacheAccessibleDescriptionHidden != null ? cacheAccessibleDescriptionHidden : cacheAccessibleDescriptionHidden = /* @__PURE__ */ new Map();
  cacheAccessibleErrorMessage != null ? cacheAccessibleErrorMessage : cacheAccessibleErrorMessage = /* @__PURE__ */ new Map();
  cacheIsHidden != null ? cacheIsHidden : cacheIsHidden = /* @__PURE__ */ new Map();
  cachePseudoContent != null ? cachePseudoContent : cachePseudoContent = /* @__PURE__ */ new Map();
  cachePseudoContentBefore != null ? cachePseudoContentBefore : cachePseudoContentBefore = /* @__PURE__ */ new Map();
  cachePseudoContentAfter != null ? cachePseudoContentAfter : cachePseudoContentAfter = /* @__PURE__ */ new Map();
  cachePointerEvents != null ? cachePointerEvents : cachePointerEvents = /* @__PURE__ */ new Map();
}
function endAriaCaches() {
  if (!--cachesCounter2) {
    cacheAccessibleName = void 0;
    cacheAccessibleNameHidden = void 0;
    cacheAccessibleNameText = void 0;
    cacheAccessibleNameTextHidden = void 0;
    cacheAccessibleDescription = void 0;
    cacheAccessibleDescriptionHidden = void 0;
    cacheAccessibleErrorMessage = void 0;
    cacheIsHidden = void 0;
    cachePseudoContent = void 0;
    cachePseudoContentBefore = void 0;
    cachePseudoContentAfter = void 0;
    cachePointerEvents = void 0;
    cacheAriaRole = void 0;
    cacheAriaDisabled = void 0;
  }
  endDOMCaches();
}
var inputTypeToRole = {
  "button": "button",
  "checkbox": "checkbox",
  "image": "button",
  "number": "spinbutton",
  "radio": "radio",
  "range": "slider",
  "reset": "button",
  "submit": "button"
};
function emptyCompositeString() {
  return { text: "" };
}
function compositeString(text, element, collectElements) {
  const elements = text && collectElements ? /* @__PURE__ */ new Set([element]) : void 0;
  return { text: text || "", elements };
}
function joinCompositeString(parts, separator, collectElements) {
  let elements;
  if (collectElements) {
    elements = /* @__PURE__ */ new Set();
    for (const part of parts) {
      for (const element of part.elements || [])
        elements.add(element);
    }
  }
  return { text: parts.map((part) => part.text).join(separator), elements };
}

// src/vendor/isomorphic/ariaSnapshot.ts
function hasPointerCursor(ariaNode) {
  return ariaNode.box.cursor === "pointer";
}

// src/vendor/isomorphic/yaml.ts
function yamlEscapeKeyIfNeeded(str) {
  if (!yamlStringNeedsQuotes(str))
    return str;
  return `'` + str.replace(/'/g, `''`) + `'`;
}
function yamlEscapeValueIfNeeded(str) {
  if (!yamlStringNeedsQuotes(str))
    return str;
  return '"' + str.replace(/[\\"\x00-\x1f\x7f-\x9f]/g, (c) => {
    switch (c) {
      case "\\":
        return "\\\\";
      case '"':
        return '\\"';
      case "\b":
        return "\\b";
      case "\f":
        return "\\f";
      case "\n":
        return "\\n";
      case "\r":
        return "\\r";
      case "	":
        return "\\t";
      default:
        const code = c.charCodeAt(0);
        return "\\x" + code.toString(16).padStart(2, "0");
    }
  }) + '"';
}
function yamlStringNeedsQuotes(str) {
  if (str.length === 0)
    return true;
  if (/^\s|\s$/.test(str))
    return true;
  if (/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f-\x9f]/.test(str))
    return true;
  if (/^-/.test(str))
    return true;
  if (/[\n:](\s|$)/.test(str))
    return true;
  if (/\s#/.test(str))
    return true;
  if (/[\n\r]/.test(str))
    return true;
  if (/^[&*\],?!>|@"'#%]/.test(str))
    return true;
  if (/[{}`]/.test(str))
    return true;
  if (/^\[/.test(str))
    return true;
  if (!isNaN(Number(str)) || ["y", "n", "yes", "no", "true", "false", "on", "off", "null"].includes(str.toLowerCase()))
    return true;
  return false;
}

// src/vendor/isomorphic/ariaSnapshotRenderer.ts
function renderAriaSnapshotAsYaml(snapshot, options = {}) {
  const lines = [];
  const includeText = options.convertStringsToRegex ? textContributesInfo : () => true;
  const renderString = options.convertStringsToRegex ? convertToBestGuessRegex : (str) => str;
  const visitText = (text, depth) => {
    const escaped = yamlEscapeValueIfNeeded(renderString(text));
    if (escaped)
      lines.push(indent(depth) + "- text: " + escaped);
  };
  const createKey = (node) => {
    let key = node.role;
    if (node.name && node.name.length <= 900) {
      const name = renderString(node.name);
      if (name) {
        const stringifiedName = name.startsWith("/") && name.endsWith("/") ? name : JSON.stringify(name);
        key += " " + stringifiedName;
      }
    }
    if (node.checked === "mixed")
      key += ` [checked=mixed]`;
    if (node.checked === true)
      key += ` [checked]`;
    if (node.disabled)
      key += ` [disabled]`;
    if (node.expanded)
      key += ` [expanded]`;
    if (node.active)
      key += ` [active]`;
    if (node.invalid === "grammar" || node.invalid === "spelling")
      key += ` [invalid=${node.invalid}]`;
    if (node.invalid === true)
      key += ` [invalid]`;
    if (node.level)
      key += ` [level=${node.level}]`;
    if (node.pressed === "mixed")
      key += ` [pressed=mixed]`;
    if (node.pressed === true)
      key += ` [pressed]`;
    if (node.selected === true)
      key += ` [selected]`;
    if (node.ref) {
      key += ` [ref=${node.ref}]`;
      if (node.cursor === "pointer")
        key += " [cursor=pointer]";
    }
    if (node.box)
      key += ` [box=${node.box.x},${node.box.y},${node.box.width},${node.box.height}]`;
    return key;
  };
  const visit = (node, depth) => {
    var _a, _b;
    if (node.role === "text") {
      visitText(node.text || "", depth);
      return;
    }
    (_a = options.lineToNode) == null ? void 0 : _a.set(lines.length, node);
    const escapedKey = indent(depth) + "- " + yamlEscapeKeyIfNeeded(createKey(node));
    const props = [];
    if (node.url !== void 0)
      props.push(["url", node.url]);
    if (node.placeholder !== void 0)
      props.push(["placeholder", node.placeholder]);
    if (node.text === void 0 && !props.length && !((_b = node.children) == null ? void 0 : _b.length) && !node.truncatedChildren) {
      lines.push(escapedKey);
    } else if (node.text !== void 0 && !props.length) {
      if (includeText(node, node.text))
        lines.push(escapedKey + ": " + yamlEscapeValueIfNeeded(renderString(node.text)));
      else
        lines.push(escapedKey);
    } else {
      lines.push(escapedKey + ":");
      for (const [name, value] of props)
        lines.push(indent(depth + 1) + "- /" + name + ": " + yamlEscapeValueIfNeeded(value));
      if (node.text !== void 0) {
        visitText(includeText(node, node.text) ? node.text : "", depth + 1);
      } else {
        for (const child of node.children || []) {
          if (typeof child === "string")
            visitText(includeText(node, child) ? child : "", depth + 1);
          else
            visit(child, depth + 1);
        }
      }
      if (node.truncatedChildren)
        lines.push(indent(depth + 1) + "- " + truncationMarker(node.truncatedChildren));
    }
  };
  for (const node of snapshot)
    visit(node, 0);
  return lines.join("\n");
}
function indent(depth) {
  return "  ".repeat(depth);
}
function truncationMarker(count) {
  return `… (${count} ${count === 1 ? "child" : "children"} truncated)`;
}
function convertToBestGuessRegex(text) {
  const dynamicContent = [
    // 550e8400-e29b-41d4-a716-446655440000
    { regex: /\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b/, replacement: "[0-9a-fA-F-]+" },
    // 2mb
    { regex: /\b[\d,.]+[bkmBKM]+\b/, replacement: "[\\d,.]+[bkmBKM]+" },
    // 2ms, 20s
    { regex: /\b\d+[hmsp]+\b/, replacement: "\\d+[hmsp]+" },
    { regex: /\b[\d,.]+[hmsp]+\b/, replacement: "[\\d,.]+[hmsp]+" },
    // Do not replace single digits with regex by default.
    // 2+ digits: [Issue 22, 22.3, 2.33, 2,333]
    { regex: /\b\d+,\d+\b/, replacement: "\\d+,\\d+" },
    { regex: /\b\d+\.\d{2,}\b/, replacement: "\\d+\\.\\d+" },
    { regex: /\b\d{2,}\.\d+\b/, replacement: "\\d+\\.\\d+" },
    { regex: /\b\d{2,}\b/, replacement: "\\d+" }
  ];
  let pattern = "";
  let lastIndex = 0;
  const combinedRegex = new RegExp(dynamicContent.map((r) => "(" + r.regex.source + ")").join("|"), "g");
  text.replace(combinedRegex, (match, ...args) => {
    const offset = args[args.length - 2];
    const groups = args.slice(0, -2);
    pattern += escapeRegExp(text.slice(lastIndex, offset));
    for (let i = 0; i < groups.length; i++) {
      if (groups[i]) {
        const { replacement } = dynamicContent[i];
        pattern += replacement;
        break;
      }
    }
    lastIndex = offset + match.length;
    return match;
  });
  if (!pattern)
    return text;
  pattern += escapeRegExp(text.slice(lastIndex));
  return String(new RegExp(pattern));
}
function textContributesInfo(node, text) {
  if (!text.length)
    return false;
  if (!node.name)
    return true;
  const substr = text.length <= 200 && node.name.length <= 200 ? longestCommonSubstring(text, node.name) : "";
  let filtered = text;
  while (substr && filtered.includes(substr))
    filtered = filtered.replace(substr, "");
  return filtered.trim().length / text.length > 0.1;
}

// src/vendor/injected/ariaSnapshotDistiller.ts
function distillAriaSnapshot(snapshot, options) {
  runPlugins(snapshot, options.mode === "ai" ? aiPlugins : normalizePlugins, options);
}
function runPlugins(snapshot, plugins, options) {
  var _a, _b;
  const ctx = { snapshot, depth: -1, maxDepth: options.depth, ancestors: [], pendingContentRefs: /* @__PURE__ */ new Set() };
  const traverse = (node, depth) => {
    const children = [];
    const visitChild = (child) => {
      var _a2, _b2;
      if (typeof child === "string") {
        children.push(child);
        return;
      }
      ctx.depth = depth + 1;
      for (const plugin of plugins) {
        const result = (_a2 = plugin.enter) == null ? void 0 : _a2.call(plugin, child, ctx);
        if (result === "remove")
          return;
        if (result === "unwrap") {
          child.children.forEach(visitChild);
          return;
        }
      }
      traverse(child, depth + 1);
      ctx.depth = depth + 1;
      for (const plugin of plugins) {
        const result = (_b2 = plugin.exit) == null ? void 0 : _b2.call(plugin, child, ctx);
        if (result === "remove")
          return;
        if (result === "unwrap") {
          children.push(...child.children);
          return;
        }
      }
      children.push(child);
    };
    ctx.ancestors.push(node);
    node.children.forEach(visitChild);
    ctx.ancestors.pop();
    node.children = children;
  };
  for (const plugin of plugins)
    (_a = plugin.enter) == null ? void 0 : _a.call(plugin, snapshot.root, ctx);
  traverse(snapshot.root, -1);
  ctx.depth = -1;
  for (const plugin of plugins)
    (_b = plugin.exit) == null ? void 0 : _b.call(plugin, snapshot.root, ctx);
}
function isLeafGeneric(node) {
  return node.role === "generic" && node.children.every((child) => typeof child === "string");
}
function isClickTargetRoot(node, ctx) {
  return !!node.ref && hasPointerCursor(node) && !ctx.ancestors.some((ancestor) => !!ancestor.ref && hasPointerCursor(ancestor));
}
var mergeStringChildren = {
  name: "mergeStringChildren",
  exit(node) {
    const children = [];
    const buffer = [];
    const flush = () => {
      if (!buffer.length)
        return;
      const text = normalizeWhiteSpace(buffer.join(""));
      if (text)
        children.push(text);
      buffer.length = 0;
    };
    for (const child of node.children) {
      if (typeof child === "string") {
        buffer.push(child);
      } else {
        flush();
        children.push(child);
      }
    }
    flush();
    node.children = children;
    if (node.children.length === 1 && node.children[0] === node.name)
      node.children = [];
  }
};
var unwrapSingleChildGenerics = {
  name: "unwrapSingleChildGenerics",
  exit(node, ctx) {
    if (node.role !== "generic" || node.name || node.children.length > 1 || !node.children.every((child) => typeof child !== "string" && !!child.ref))
      return;
    if (!node.children.length && isClickTargetRoot(node, ctx))
      return;
    return "unwrap";
  }
};
var removeNamelessImages = {
  name: "removeNamelessImages",
  exit(node, ctx) {
    if (node.role === "img" && !node.name && !node.children.length && !isClickTargetRoot(node, ctx))
      return "remove";
  }
};
var removeRedundantNames = {
  name: "removeRedundantNames",
  enter(node, ctx) {
    var _a;
    if (!node.ref)
      return;
    for (const ref of ((_a = ctx.snapshot.info.get(node.ref)) == null ? void 0 : _a.nameFromContentRefs) || [])
      ctx.pendingContentRefs.add(ref);
    const beyondDepth = !!ctx.maxDepth && ctx.depth > ctx.maxDepth;
    if (!beyondDepth && !isLeafGeneric(node))
      ctx.pendingContentRefs.delete(node.ref);
  },
  exit(node, ctx) {
    var _a;
    if (!node.ref)
      return;
    const nameFromContentRefs = (_a = ctx.snapshot.info.get(node.ref)) == null ? void 0 : _a.nameFromContentRefs;
    if (!(nameFromContentRefs == null ? void 0 : nameFromContentRefs.length))
      return;
    if (nameFromContentRefs.every((ref) => !ctx.pendingContentRefs.has(ref))) {
      node.name = "";
    } else {
      for (const ref of nameFromContentRefs)
        ctx.pendingContentRefs.delete(ref);
    }
  }
};
var removeNameRepeatingChild = {
  name: "removeNameRepeatingChild",
  exit(node, ctx) {
    const parent = ctx.ancestors[ctx.ancestors.length - 1];
    if (!(parent == null ? void 0 : parent.name) || node.role !== "generic" || node.active || Object.keys(node.props).length)
      return;
    const singleTextChild = node.children.length === 1 && typeof node.children[0] === "string" ? node.children[0] : void 0;
    const text = node.name ? node.children.length ? void 0 : node.name : singleTextChild;
    if (text && text === parent.name) {
      if (node.ref)
        ctx.pendingContentRefs.add(node.ref);
      return "remove";
    }
  }
};
var inlineTextIntoGeneric = {
  name: "inlineTextIntoGeneric",
  exit(node) {
    if (node.role !== "generic" || Object.keys(node.props).length || node.children.length !== 1)
      return;
    const child = node.children[0];
    if (typeof child === "string")
      return;
    if (child.role !== "generic" || child.name || child.active || Object.keys(child.props).length)
      return;
    if (child.children.length === 1 && typeof child.children[0] === "string")
      node.children = [child.children[0]];
  }
};
var normalizePlugins = [
  mergeStringChildren,
  unwrapSingleChildGenerics
];
var aiPlugins = [
  mergeStringChildren,
  removeNamelessImages,
  removeRedundantNames,
  inlineTextIntoGeneric,
  removeNameRepeatingChild,
  unwrapSingleChildGenerics
];

// src/vendor/injected/ariaSnapshot.ts
var lastRef = 0;
function toInternalOptions(options) {
  const renderBoxes = options.boxes;
  if (options.mode === "ai") {
    return {
      visibility: "ariaOrVisible",
      refs: options.refs ? "interactable" : "none",
      refPrefix: options.refPrefix,
      includeGenericRole: true,
      renderActive: !options.doNotRenderActive,
      renderCursorPointer: true,
      renderBoxes
    };
  }
  if (options.mode === "autoexpect") {
    return { visibility: "ariaAndVisible", refs: "none", renderBoxes };
  }
  return { visibility: "aria", refs: "none", renderBoxes };
}
function generateAriaTree(rootElement, publicOptions) {
  const options = toInternalOptions(publicOptions);
  const visited = /* @__PURE__ */ new Set();
  const nameSourceElements = /* @__PURE__ */ new Map();
  const snapshot = {
    root: { role: "fragment", name: "", children: [], props: {}, box: computeBox(rootElement), receivesPointerEvents: true },
    info: /* @__PURE__ */ new Map(),
    refs: /* @__PURE__ */ new Map(),
    iframeRefs: []
  };
  setAriaNodeElement(snapshot.root, rootElement);
  const visit = (ariaNode, node, parentElementVisible) => {
    if (visited.has(node))
      return;
    visited.add(node);
    if (node.nodeType === Node.TEXT_NODE && node.nodeValue) {
      if (!parentElementVisible)
        return;
      const text = node.nodeValue;
      if (ariaNode.role !== "textbox" && text)
        ariaNode.children.push(node.nodeValue || "");
      return;
    }
    if (node.nodeType !== Node.ELEMENT_NODE)
      return;
    const element = node;
    const isElementVisibleForAria = !isElementHiddenForAria(element);
    let visible = isElementVisibleForAria;
    if (options.visibility === "ariaOrVisible")
      visible = isElementVisibleForAria || isElementVisible(element);
    if (options.visibility === "ariaAndVisible")
      visible = isElementVisibleForAria && isElementVisible(element);
    if (options.visibility === "aria" && !visible)
      return;
    const ariaChildren = [];
    if (element.hasAttribute("aria-owns")) {
      const ids = element.getAttribute("aria-owns").split(/\s+/);
      for (const id of ids) {
        const ownedElement = rootElement.ownerDocument.getElementById(id);
        if (ownedElement)
          ariaChildren.push(ownedElement);
      }
    }
    const childAriaNode = visible ? toAriaNode(element, options, nameSourceElements) : null;
    let elementInfo;
    if (childAriaNode) {
      if (childAriaNode.ref) {
        elementInfo = { element, nameFromContentRefs: [] };
        snapshot.info.set(childAriaNode.ref, elementInfo);
        snapshot.refs.set(element, childAriaNode.ref);
        if (childAriaNode.role === "iframe")
          snapshot.iframeRefs.push(childAriaNode.ref);
      }
      ariaNode.children.push(childAriaNode);
    }
    processElement(childAriaNode || ariaNode, element, ariaChildren, visible);
    if (elementInfo) {
      for (const contributor of nameSourceElements.get(childAriaNode) || []) {
        const ref = snapshot.refs.get(contributor);
        if (ref && ref !== childAriaNode.ref)
          elementInfo.nameFromContentRefs.push(ref);
      }
    }
  };
  function processElement(ariaNode, element, ariaChildren, parentElementVisible) {
    var _a;
    const display = ((_a = getElementComputedStyle(element)) == null ? void 0 : _a.display) || "inline";
    const treatAsBlock = display !== "inline" || element.nodeName === "BR" ? " " : "";
    if (treatAsBlock)
      ariaNode.children.push(treatAsBlock);
    ariaNode.children.push(getCSSContent(element, "::before") || "");
    const assignedNodes = element.nodeName === "SLOT" ? element.assignedNodes() : [];
    if (assignedNodes.length) {
      for (const child of assignedNodes)
        visit(ariaNode, child, parentElementVisible);
    } else {
      for (let child = element.firstChild; child; child = child.nextSibling) {
        if (!child.assignedSlot)
          visit(ariaNode, child, parentElementVisible);
      }
      if (element.shadowRoot) {
        for (let child = element.shadowRoot.firstChild; child; child = child.nextSibling)
          visit(ariaNode, child, parentElementVisible);
      }
    }
    for (const child of ariaChildren)
      visit(ariaNode, child, parentElementVisible);
    ariaNode.children.push(getCSSContent(element, "::after") || "");
    if (treatAsBlock)
      ariaNode.children.push(treatAsBlock);
    if (ariaNode.children.length === 1 && ariaNode.name === ariaNode.children[0])
      ariaNode.children = [];
    if (ariaNode.role === "link" && element.hasAttribute("href")) {
      const href = element.getAttribute("href");
      ariaNode.props["url"] = truncateDataUrl(href);
    }
    if (ariaNode.role === "textbox" && element.hasAttribute("placeholder") && element.getAttribute("placeholder") !== ariaNode.name) {
      const placeholder = element.getAttribute("placeholder");
      ariaNode.props["placeholder"] = placeholder;
    }
  }
  beginAriaCaches();
  try {
    visit(snapshot.root, rootElement, true);
  } finally {
    endAriaCaches();
  }
  distillAriaSnapshot(snapshot, publicOptions);
  return snapshot;
}
function computeAriaRef(ariaNode, options) {
  var _a;
  if (options.refs === "none")
    return;
  if (options.refs === "interactable" && (!ariaNode.box.visible || !ariaNode.receivesPointerEvents))
    return;
  const element = ariaNodeElement(ariaNode);
  let ariaRef = element._ariaRef;
  if (!ariaRef || ariaRef.role !== ariaNode.role || ariaRef.name !== ariaNode.name) {
    ariaRef = { role: ariaNode.role, name: ariaNode.name, ref: ((_a = options.refPrefix) != null ? _a : "") + "e" + ++lastRef };
    element._ariaRef = ariaRef;
  }
  ariaNode.ref = ariaRef.ref;
}
function toAriaNode(element, options, nameSourceElements) {
  var _a;
  const active = element.ownerDocument.activeElement === element && element.ownerDocument.hasFocus();
  if (element.nodeName === "IFRAME" || element.nodeName === "FRAME") {
    const ariaNode = {
      role: "iframe",
      name: "",
      children: [],
      props: {},
      box: computeBox(element),
      receivesPointerEvents: true,
      active
    };
    setAriaNodeElement(ariaNode, element);
    computeAriaRef(ariaNode, options);
    return ariaNode;
  }
  const defaultRole = options.includeGenericRole ? "generic" : null;
  const role = (_a = getAriaRole(element)) != null ? _a : defaultRole;
  if (!role || role === "presentation" || role === "none")
    return null;
  const name = getElementAccessibleName(element, false);
  const receivesPointerEvents2 = receivesPointerEvents(element);
  const box = computeBox(element);
  if (role === "generic" && box.inline && element.childNodes.length === 1 && element.childNodes[0].nodeType === Node.TEXT_NODE)
    return null;
  const result = {
    role,
    name: normalizeWhiteSpace(name.text),
    children: [],
    props: {},
    box,
    receivesPointerEvents: receivesPointerEvents2,
    active
  };
  setAriaNodeElement(result, element);
  nameSourceElements.set(result, name.elements);
  computeAriaRef(result, options);
  if (kAriaCheckedRoles.includes(role))
    result.checked = getAriaChecked(element);
  if (kAriaDisabledRoles.includes(role))
    result.disabled = getAriaDisabled2(element);
  if (kAriaExpandedRoles.includes(role))
    result.expanded = getAriaExpanded(element);
  if (kAriaInvalidRoles.includes(role)) {
    const invalid = getAriaInvalid(element);
    result.invalid = invalid === "false" ? false : invalid === "true" ? true : invalid;
  }
  if (kAriaLevelRoles.includes(role))
    result.level = getAriaLevel(element);
  if (kAriaPressedRoles.includes(role))
    result.pressed = getAriaPressed(element);
  if (kAriaSelectedRoles.includes(role))
    result.selected = getAriaSelected(element);
  if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
    if (element.type !== "checkbox" && element.type !== "radio" && element.type !== "file")
      result.children = [element.value];
  }
  return result;
}
function renderAriaTreeAsJSON(ariaSnapshot, publicOptions) {
  const options = toInternalOptions(publicOptions);
  const iframeDepths = {};
  const visit = (ariaNode, depth, renderCursorPointer) => {
    if (ariaNode.role === "iframe" && ariaNode.ref)
      iframeDepths[ariaNode.ref] = depth;
    const node = { role: ariaNode.role };
    if (ariaNode.name)
      node.name = ariaNode.name;
    if (ariaNode.checked === "mixed" || ariaNode.checked === true)
      node.checked = ariaNode.checked;
    if (ariaNode.disabled)
      node.disabled = true;
    if (ariaNode.expanded)
      node.expanded = true;
    if (ariaNode.active && options.renderActive)
      node.active = true;
    if (ariaNode.invalid)
      node.invalid = ariaNode.invalid;
    if (ariaNode.level)
      node.level = ariaNode.level;
    if (ariaNode.pressed === "mixed" || ariaNode.pressed === true)
      node.pressed = ariaNode.pressed;
    if (ariaNode.selected === true)
      node.selected = true;
    if (ariaNode.ref) {
      node.ref = ariaNode.ref;
      if (renderCursorPointer && hasPointerCursor(ariaNode))
        node.cursor = "pointer";
    }
    if (options.renderBoxes) {
      const element = ariaNodeElement(ariaNode);
      if (element) {
        const r = element.getBoundingClientRect();
        node.box = { x: Math.round(r.x), y: Math.round(r.y), width: Math.round(r.width), height: Math.round(r.height) };
      }
    }
    if (ariaNode.props.url !== void 0)
      node.url = ariaNode.props.url;
    if (ariaNode.props.placeholder !== void 0)
      node.placeholder = ariaNode.props.placeholder;
    const singleTextChild = ariaNode.children.length === 1 && typeof ariaNode.children[0] === "string" ? ariaNode.children[0] : void 0;
    const isAtDepthLimit = !!publicOptions.depth && depth === publicOptions.depth;
    if (singleTextChild !== void 0) {
      node.text = singleTextChild;
    } else if (isAtDepthLimit && ariaNode.children.length) {
      node.truncatedChildren = ariaNode.children.length;
    } else if (ariaNode.children.length) {
      const inCursorPointer = !!ariaNode.ref && renderCursorPointer && hasPointerCursor(ariaNode);
      node.children = ariaNode.children.map((child) => {
        if (typeof child === "string")
          return child;
        return visit(child, depth + 1, renderCursorPointer && !inCursorPointer);
      });
    }
    return node;
  };
  const json = [];
  const nodesToRender = ariaSnapshot.root.role === "fragment" ? ariaSnapshot.root.children : [ariaSnapshot.root];
  for (const nodeToRender of nodesToRender) {
    if (typeof nodeToRender === "string")
      json.push({ role: "text", text: nodeToRender });
    else
      json.push(visit(nodeToRender, 0, !!options.renderCursorPointer));
  }
  return { json, iframeDepths };
}
var elementSymbol = /* @__PURE__ */ Symbol("element");
function ariaNodeElement(ariaNode) {
  return ariaNode[elementSymbol];
}
function setAriaNodeElement(ariaNode, element) {
  ariaNode[elementSymbol] = element;
}

// src/vendor/isomorphic/cssParser.ts
var InvalidSelectorError = class extends Error {
};
function parseCSS(selector, customNames) {
  let tokens;
  try {
    tokens = tokenize(selector);
    if (!(tokens[tokens.length - 1] instanceof EOFToken))
      tokens.push(new EOFToken());
  } catch (e) {
    const newMessage = e.message + ` while parsing css selector "${selector}". Did you mean to CSS.escape it?`;
    const index = (e.stack || "").indexOf(e.message);
    if (index !== -1)
      e.stack = e.stack.substring(0, index) + newMessage + e.stack.substring(index + e.message.length);
    e.message = newMessage;
    throw e;
  }
  const unsupportedToken = tokens.find((token) => {
    return token instanceof AtKeywordToken || token instanceof BadStringToken || token instanceof BadURLToken || token instanceof ColumnToken || token instanceof CDOToken || token instanceof CDCToken || token instanceof SemicolonToken || // TODO: Consider using these for something, e.g. to escape complex strings.
    // For example :xpath{ (//div/bar[@attr="foo"])[2]/baz }
    // Or this way :xpath( {complex-xpath-goes-here("hello")} )
    token instanceof OpenCurlyToken || token instanceof CloseCurlyToken || // TODO: Consider treating these as strings?
    token instanceof URLToken || token instanceof PercentageToken;
  });
  if (unsupportedToken)
    throw new InvalidSelectorError(`Unsupported token "${unsupportedToken.toSource()}" while parsing css selector "${selector}". Did you mean to CSS.escape it?`);
  let pos = 0;
  const names = /* @__PURE__ */ new Set();
  function unexpected() {
    return new InvalidSelectorError(`Unexpected token "${tokens[pos].toSource()}" while parsing css selector "${selector}". Did you mean to CSS.escape it?`);
  }
  function skipWhitespace() {
    while (tokens[pos] instanceof WhitespaceToken)
      pos++;
  }
  function isIdent(p = pos) {
    return tokens[p] instanceof IdentToken;
  }
  function isString(p = pos) {
    return tokens[p] instanceof StringToken;
  }
  function isNumber(p = pos) {
    return tokens[p] instanceof NumberToken;
  }
  function isComma(p = pos) {
    return tokens[p] instanceof CommaToken;
  }
  function isOpenParen(p = pos) {
    return tokens[p] instanceof OpenParenToken;
  }
  function isCloseParen(p = pos) {
    return tokens[p] instanceof CloseParenToken;
  }
  function isFunction(p = pos) {
    return tokens[p] instanceof FunctionToken;
  }
  function isStar(p = pos) {
    return tokens[p] instanceof DelimToken && tokens[p].value === "*";
  }
  function isEOF(p = pos) {
    return tokens[p] instanceof EOFToken;
  }
  function isClauseCombinator(p = pos) {
    return tokens[p] instanceof DelimToken && [">", "+", "~"].includes(tokens[p].value);
  }
  function isSelectorClauseEnd(p = pos) {
    return isComma(p) || isCloseParen(p) || isEOF(p) || isClauseCombinator(p) || tokens[p] instanceof WhitespaceToken;
  }
  function consumeFunctionArguments() {
    const result2 = [consumeArgument()];
    while (true) {
      skipWhitespace();
      if (!isComma())
        break;
      pos++;
      result2.push(consumeArgument());
    }
    return result2;
  }
  function consumeArgument() {
    skipWhitespace();
    if (isNumber())
      return tokens[pos++].value;
    if (isString())
      return tokens[pos++].value;
    return consumeComplexSelector();
  }
  function consumeComplexSelector() {
    const result2 = { simples: [] };
    skipWhitespace();
    if (isClauseCombinator()) {
      result2.simples.push({ selector: { functions: [{ name: "scope", args: [] }] }, combinator: "" });
    } else {
      result2.simples.push({ selector: consumeSimpleSelector(), combinator: "" });
    }
    while (true) {
      skipWhitespace();
      if (isClauseCombinator()) {
        result2.simples[result2.simples.length - 1].combinator = tokens[pos++].value;
        skipWhitespace();
      } else if (isSelectorClauseEnd()) {
        break;
      }
      result2.simples.push({ combinator: "", selector: consumeSimpleSelector() });
    }
    return result2;
  }
  function consumeSimpleSelector() {
    let rawCSSString = "";
    const functions = [];
    while (!isSelectorClauseEnd()) {
      if (isIdent() || isStar()) {
        rawCSSString += tokens[pos++].toSource();
      } else if (tokens[pos] instanceof HashToken) {
        rawCSSString += tokens[pos++].toSource();
      } else if (tokens[pos] instanceof DelimToken && tokens[pos].value === ".") {
        pos++;
        if (isIdent())
          rawCSSString += "." + tokens[pos++].toSource();
        else
          throw unexpected();
      } else if (tokens[pos] instanceof ColonToken) {
        pos++;
        if (isIdent()) {
          if (!customNames.has(tokens[pos].value.toLowerCase())) {
            rawCSSString += ":" + tokens[pos++].toSource();
          } else {
            const name = tokens[pos++].value.toLowerCase();
            functions.push({ name, args: [] });
            names.add(name);
          }
        } else if (isFunction()) {
          const name = tokens[pos++].value.toLowerCase();
          if (!customNames.has(name)) {
            rawCSSString += `:${name}(${consumeBuiltinFunctionArguments()})`;
          } else {
            functions.push({ name, args: consumeFunctionArguments() });
            names.add(name);
          }
          skipWhitespace();
          if (!isCloseParen())
            throw unexpected();
          pos++;
        } else {
          throw unexpected();
        }
      } else if (tokens[pos] instanceof OpenSquareToken) {
        rawCSSString += "[";
        pos++;
        while (!(tokens[pos] instanceof CloseSquareToken) && !isEOF())
          rawCSSString += tokens[pos++].toSource();
        if (!(tokens[pos] instanceof CloseSquareToken))
          throw unexpected();
        rawCSSString += "]";
        pos++;
      } else {
        throw unexpected();
      }
    }
    if (!rawCSSString && !functions.length)
      throw unexpected();
    return { css: rawCSSString || void 0, functions };
  }
  function consumeBuiltinFunctionArguments() {
    let s = "";
    let balance = 1;
    while (!isEOF()) {
      if (isOpenParen() || isFunction())
        balance++;
      if (isCloseParen())
        balance--;
      if (!balance)
        break;
      s += tokens[pos++].toSource();
    }
    return s;
  }
  const result = consumeFunctionArguments();
  if (!isEOF())
    throw unexpected();
  if (result.some((arg) => typeof arg !== "object" || !("simples" in arg)))
    throw new InvalidSelectorError(`Error while parsing css selector "${selector}". Did you mean to CSS.escape it?`);
  return { selector: result, names: Array.from(names) };
}

// src/vendor/isomorphic/selectorParser.ts
var kNestedSelectorNames = /* @__PURE__ */ new Set(["internal:has", "internal:has-not", "internal:and", "internal:or", "internal:chain", "left-of", "right-of", "above", "below", "near"]);
var kNestedSelectorNamesWithDistance = /* @__PURE__ */ new Set(["left-of", "right-of", "above", "below", "near"]);
var customCSSNames = /* @__PURE__ */ new Set(["not", "is", "where", "has", "scope", "light", "visible", "text", "text-matches", "text-is", "has-text", "above", "below", "right-of", "left-of", "near", "nth-match"]);
function parseSelector(selector) {
  const parsedStrings = parseSelectorString(selector);
  const parts = [];
  for (const part of parsedStrings.parts) {
    if (part.name === "css" || part.name === "css:light") {
      if (part.name === "css:light")
        part.body = ":light(" + part.body + ")";
      const parsedCSS = parseCSS(part.body, customCSSNames);
      parts.push({
        name: "css",
        body: parsedCSS.selector,
        source: part.body
      });
      continue;
    }
    if (kNestedSelectorNames.has(part.name)) {
      let innerSelector;
      let distance;
      try {
        const unescaped = JSON.parse("[" + part.body + "]");
        if (!Array.isArray(unescaped) || unescaped.length < 1 || unescaped.length > 2 || typeof unescaped[0] !== "string")
          throw new InvalidSelectorError(`Malformed selector: ${part.name}=` + part.body);
        innerSelector = unescaped[0];
        if (unescaped.length === 2) {
          if (typeof unescaped[1] !== "number" || !kNestedSelectorNamesWithDistance.has(part.name))
            throw new InvalidSelectorError(`Malformed selector: ${part.name}=` + part.body);
          distance = unescaped[1];
        }
      } catch (e) {
        throw new InvalidSelectorError(`Malformed selector: ${part.name}=` + part.body);
      }
      const nested = { name: part.name, source: part.body, body: { parsed: parseSelector(innerSelector), distance } };
      const lastFrame = [...nested.body.parsed.parts].reverse().find((part2) => part2.name === "internal:control" && part2.body === "enter-frame");
      const lastFrameIndex = lastFrame ? nested.body.parsed.parts.indexOf(lastFrame) : -1;
      if (lastFrameIndex !== -1 && selectorPartsEqual(nested.body.parsed.parts.slice(0, lastFrameIndex + 1), parts.slice(0, lastFrameIndex + 1)))
        nested.body.parsed.parts.splice(0, lastFrameIndex + 1);
      parts.push(nested);
      continue;
    }
    parts.push({ ...part, source: part.body });
  }
  if (kNestedSelectorNames.has(parts[0].name))
    throw new InvalidSelectorError(`"${parts[0].name}" selector cannot be first`);
  return {
    capture: parsedStrings.capture,
    parts
  };
}
function selectorPartsEqual(list1, list2) {
  return stringifySelector({ parts: list1 }) === stringifySelector({ parts: list2 });
}
function stringifySelector(selector, forceEngineName) {
  if (typeof selector === "string")
    return selector;
  return selector.parts.map((p, i) => {
    let includeEngine = true;
    if (!forceEngineName && i !== selector.capture) {
      if (p.name === "css")
        includeEngine = false;
      else if (p.name === "xpath" && (p.source.startsWith("//") || p.source.startsWith("..")))
        includeEngine = false;
    }
    const prefix = includeEngine ? p.name + "=" : "";
    return `${i === selector.capture ? "*" : ""}${prefix}${p.source}`;
  }).join(" >> ");
}
function parseSelectorString(selector) {
  let index = 0;
  let quote;
  let start = 0;
  const result = { parts: [] };
  const append = () => {
    const part = selector.substring(start, index).trim();
    const eqIndex = part.indexOf("=");
    let name;
    let body;
    if (eqIndex !== -1 && part.substring(0, eqIndex).trim().match(/^[a-zA-Z_0-9-+:*]+$/)) {
      name = part.substring(0, eqIndex).trim();
      body = part.substring(eqIndex + 1);
    } else if (part.length > 1 && part[0] === '"' && part[part.length - 1] === '"') {
      name = "text";
      body = part;
    } else if (part.length > 1 && part[0] === "'" && part[part.length - 1] === "'") {
      name = "text";
      body = part;
    } else if (/^\(*\/\//.test(part) || part.startsWith("..")) {
      name = "xpath";
      body = part;
    } else {
      name = "css";
      body = part;
    }
    let capture = false;
    if (name[0] === "*") {
      capture = true;
      name = name.substring(1);
    }
    result.parts.push({ name, body });
    if (capture) {
      if (result.capture !== void 0)
        throw new InvalidSelectorError(`Only one of the selectors can capture using * modifier`);
      result.capture = result.parts.length - 1;
    }
  };
  if (!selector.includes(">>")) {
    index = selector.length;
    append();
    return result;
  }
  const shouldIgnoreTextSelectorQuote = () => {
    const prefix = selector.substring(start, index);
    const match = prefix.match(/^\s*text\s*=(.*)$/);
    return !!match && !!match[1];
  };
  while (index < selector.length) {
    const c = selector[index];
    if (c === "\\" && index + 1 < selector.length) {
      index += 2;
    } else if (c === quote) {
      quote = void 0;
      index++;
    } else if (!quote && (c === '"' || c === "'" || c === "`") && !shouldIgnoreTextSelectorQuote()) {
      quote = c;
      index++;
    } else if (!quote && c === ">" && selector[index + 1] === ">") {
      append();
      index += 2;
      start = index;
    } else {
      index++;
    }
  }
  append();
  return result;
}
function parseAttributeSelector(selector, allowUnquotedStrings) {
  let wp = 0;
  let EOL = selector.length === 0;
  const next = () => selector[wp] || "";
  const eat1 = () => {
    const result2 = next();
    ++wp;
    EOL = wp >= selector.length;
    return result2;
  };
  const syntaxError = (stage) => {
    if (EOL)
      throw new InvalidSelectorError(`Unexpected end of selector while parsing selector \`${selector}\``);
    throw new InvalidSelectorError(`Error while parsing selector \`${selector}\` - unexpected symbol "${next()}" at position ${wp}` + (stage ? " during " + stage : ""));
  };
  function skipSpaces() {
    while (!EOL && /\s/.test(next()))
      eat1();
  }
  function isCSSNameChar(char) {
    return char >= "" || char >= "0" && char <= "9" || char >= "A" && char <= "Z" || char >= "a" && char <= "z" || char >= "0" && char <= "9" || char === "_" || char === "-";
  }
  function readIdentifier() {
    let result2 = "";
    skipSpaces();
    while (!EOL && isCSSNameChar(next()))
      result2 += eat1();
    return result2;
  }
  function readQuotedString(quote) {
    let result2 = eat1();
    if (result2 !== quote)
      syntaxError("parsing quoted string");
    while (!EOL && next() !== quote) {
      if (next() === "\\")
        eat1();
      result2 += eat1();
    }
    if (next() !== quote)
      syntaxError("parsing quoted string");
    result2 += eat1();
    return result2;
  }
  function readRegularExpression() {
    if (eat1() !== "/")
      syntaxError("parsing regular expression");
    let source = "";
    let inClass = false;
    while (!EOL) {
      if (next() === "\\") {
        source += eat1();
        if (EOL)
          syntaxError("parsing regular expression");
      } else if (inClass && next() === "]") {
        inClass = false;
      } else if (!inClass && next() === "[") {
        inClass = true;
      } else if (!inClass && next() === "/") {
        break;
      }
      source += eat1();
    }
    if (eat1() !== "/")
      syntaxError("parsing regular expression");
    let flags = "";
    while (!EOL && next().match(/[dgimsuvy]/))
      flags += eat1();
    try {
      return new RegExp(source, flags);
    } catch (e) {
      throw new InvalidSelectorError(`Error while parsing selector \`${selector}\`: ${e.message}`);
    }
  }
  function readAttributeToken() {
    let token = "";
    skipSpaces();
    if (next() === `'` || next() === `"`)
      token = readQuotedString(next()).slice(1, -1);
    else
      token = readIdentifier();
    if (!token)
      syntaxError("parsing property path");
    return token;
  }
  function readOperator() {
    skipSpaces();
    let op = "";
    if (!EOL)
      op += eat1();
    if (!EOL && op !== "=")
      op += eat1();
    if (!["=", "*=", "^=", "$=", "|=", "~="].includes(op))
      syntaxError("parsing operator");
    return op;
  }
  function readAttribute() {
    eat1();
    const jsonPath = [];
    jsonPath.push(readAttributeToken());
    skipSpaces();
    while (next() === ".") {
      eat1();
      jsonPath.push(readAttributeToken());
      skipSpaces();
    }
    if (next() === "]") {
      eat1();
      return { name: jsonPath.join("."), jsonPath, op: "<truthy>", value: null, caseSensitive: false };
    }
    const operator = readOperator();
    let value = void 0;
    let caseSensitive = true;
    skipSpaces();
    if (next() === "/") {
      if (operator !== "=")
        throw new InvalidSelectorError(`Error while parsing selector \`${selector}\` - cannot use ${operator} in attribute with regular expression`);
      value = readRegularExpression();
    } else if (next() === `'` || next() === `"`) {
      value = readQuotedString(next()).slice(1, -1);
      skipSpaces();
      if (next() === "i" || next() === "I") {
        caseSensitive = false;
        eat1();
      } else if (next() === "s" || next() === "S") {
        caseSensitive = true;
        eat1();
      }
    } else {
      value = "";
      while (!EOL && (isCSSNameChar(next()) || next() === "+" || next() === "."))
        value += eat1();
      if (value === "true") {
        value = true;
      } else if (value === "false") {
        value = false;
      } else {
        if (!allowUnquotedStrings) {
          value = +value;
          if (Number.isNaN(value))
            syntaxError("parsing attribute value");
        }
      }
    }
    skipSpaces();
    if (next() !== "]")
      syntaxError("parsing attribute value");
    eat1();
    if (operator !== "=" && typeof value !== "string")
      throw new InvalidSelectorError(`Error while parsing selector \`${selector}\` - cannot use ${operator} in attribute with non-string matching value - ${value}`);
    return { name: jsonPath.join("."), jsonPath, op: operator, value, caseSensitive };
  }
  const result = {
    name: "",
    attributes: []
  };
  result.name = readIdentifier();
  skipSpaces();
  while (next() === "[") {
    result.attributes.push(readAttribute());
    skipSpaces();
  }
  if (!EOL)
    syntaxError(void 0);
  if (!result.name && !result.attributes.length)
    throw new InvalidSelectorError(`Error while parsing selector \`${selector}\` - selector cannot be empty`);
  return result;
}

// src/vendor/injected/selectorUtils.ts
function matchesAttributePart(value, attr) {
  const objValue = typeof value === "string" && !attr.caseSensitive ? value.toUpperCase() : value;
  const attrValue = typeof attr.value === "string" && !attr.caseSensitive ? attr.value.toUpperCase() : attr.value;
  if (attr.op === "<truthy>")
    return !!objValue;
  if (attr.op === "=") {
    if (attrValue instanceof RegExp)
      return typeof objValue === "string" && !!objValue.match(attrValue);
    return objValue === attrValue;
  }
  if (typeof objValue !== "string" || typeof attrValue !== "string")
    return false;
  if (attr.op === "*=")
    return objValue.includes(attrValue);
  if (attr.op === "^=")
    return objValue.startsWith(attrValue);
  if (attr.op === "$=")
    return objValue.endsWith(attrValue);
  if (attr.op === "|=")
    return objValue === attrValue || objValue.startsWith(attrValue + "-");
  if (attr.op === "~=")
    return objValue.split(" ").includes(attrValue);
  return false;
}
function shouldSkipForTextMatching(element) {
  const document = element.ownerDocument;
  return element.nodeName === "SCRIPT" || element.nodeName === "NOSCRIPT" || element.nodeName === "STYLE" || document.head && document.head.contains(element);
}
function elementText(cache, root) {
  let value = cache.get(root);
  if (value === void 0) {
    value = { full: "", normalized: "", immediate: [] };
    if (!shouldSkipForTextMatching(root)) {
      let currentImmediate = "";
      if (root instanceof HTMLInputElement && (root.type === "submit" || root.type === "button" || root.type === "reset")) {
        value = { full: root.value, normalized: normalizeWhiteSpace(root.value), immediate: [root.value] };
      } else {
        for (let child = root.firstChild; child; child = child.nextSibling) {
          if (child.nodeType === Node.TEXT_NODE) {
            value.full += child.nodeValue || "";
            currentImmediate += child.nodeValue || "";
          } else if (child.nodeType === Node.COMMENT_NODE) {
            continue;
          } else {
            if (currentImmediate)
              value.immediate.push(currentImmediate);
            currentImmediate = "";
            if (child.nodeType === Node.ELEMENT_NODE)
              value.full += elementText(cache, child).full;
          }
        }
        if (currentImmediate)
          value.immediate.push(currentImmediate);
        if (root.shadowRoot)
          value.full += elementText(cache, root.shadowRoot).full;
        if (value.full)
          value.normalized = normalizeWhiteSpace(value.full);
      }
    }
    cache.set(root, value);
  }
  return value;
}
function elementMatchesText(cache, element, matcher) {
  if (shouldSkipForTextMatching(element))
    return "none";
  if (!matcher(elementText(cache, element)))
    return "none";
  for (let child = element.firstChild; child; child = child.nextSibling) {
    if (child.nodeType === Node.ELEMENT_NODE && matcher(elementText(cache, child)))
      return "selfAndChildren";
  }
  if (element.shadowRoot && matcher(elementText(cache, element.shadowRoot)))
    return "selfAndChildren";
  return "self";
}
function getElementLabels(textCache, element, options) {
  let labels = getAriaLabelledByElements(element);
  if (labels) {
    if (options == null ? void 0 : options.skipRefsInsideElement)
      labels = labels.filter((label) => label !== element && !element.contains(label));
    return labels.map((label) => elementText(textCache, label));
  }
  const ariaLabel = element.getAttribute("aria-label");
  if (ariaLabel !== null && !!ariaLabel.trim())
    return [{ full: ariaLabel, normalized: normalizeWhiteSpace(ariaLabel), immediate: [ariaLabel] }];
  const isNonHiddenInput = element.nodeName === "INPUT" && element.type !== "hidden";
  if (["BUTTON", "METER", "OUTPUT", "PROGRESS", "SELECT", "TEXTAREA"].includes(element.nodeName) || isNonHiddenInput) {
    const labels2 = element.labels;
    if (labels2)
      return [...labels2].map((label) => elementText(textCache, label));
  }
  return [];
}

// src/vendor/injected/roleSelectorEngine.ts
var kSupportedAttributes = ["selected", "checked", "pressed", "expanded", "level", "disabled", "name", "description", "include-hidden"];
kSupportedAttributes.sort();
function validateSupportedRole(attr, roles, role) {
  if (!roles.includes(role))
    throw new Error(`"${attr}" attribute is only supported for roles: ${roles.slice().sort().map((role2) => `"${role2}"`).join(", ")}`);
}
function validateSupportedValues(attr, values) {
  if (attr.op !== "<truthy>" && !values.includes(attr.value))
    throw new Error(`"${attr.name}" must be one of ${values.map((v) => JSON.stringify(v)).join(", ")}`);
}
function validateSupportedOp(attr, ops) {
  if (!ops.includes(attr.op))
    throw new Error(`"${attr.name}" does not support "${attr.op}" matcher`);
}
function validateAttributes(attrs, role) {
  const options = { role };
  for (const attr of attrs) {
    switch (attr.name) {
      case "checked": {
        validateSupportedRole(attr.name, kAriaCheckedRoles, role);
        validateSupportedValues(attr, [true, false, "mixed"]);
        validateSupportedOp(attr, ["<truthy>", "="]);
        options.checked = attr.op === "<truthy>" ? true : attr.value;
        break;
      }
      case "pressed": {
        validateSupportedRole(attr.name, kAriaPressedRoles, role);
        validateSupportedValues(attr, [true, false, "mixed"]);
        validateSupportedOp(attr, ["<truthy>", "="]);
        options.pressed = attr.op === "<truthy>" ? true : attr.value;
        break;
      }
      case "selected": {
        validateSupportedRole(attr.name, kAriaSelectedRoles, role);
        validateSupportedValues(attr, [true, false]);
        validateSupportedOp(attr, ["<truthy>", "="]);
        options.selected = attr.op === "<truthy>" ? true : attr.value;
        break;
      }
      case "expanded": {
        validateSupportedRole(attr.name, kAriaExpandedRoles, role);
        validateSupportedValues(attr, [true, false]);
        validateSupportedOp(attr, ["<truthy>", "="]);
        options.expanded = attr.op === "<truthy>" ? true : attr.value;
        break;
      }
      case "level": {
        validateSupportedRole(attr.name, kAriaLevelRoles, role);
        if (typeof attr.value === "string")
          attr.value = +attr.value;
        if (attr.op !== "=" || typeof attr.value !== "number" || Number.isNaN(attr.value))
          throw new Error(`"level" attribute must be compared to a number`);
        options.level = attr.value;
        break;
      }
      case "disabled": {
        validateSupportedValues(attr, [true, false]);
        validateSupportedOp(attr, ["<truthy>", "="]);
        options.disabled = attr.op === "<truthy>" ? true : attr.value;
        break;
      }
      case "name": {
        if (attr.op === "<truthy>")
          throw new Error(`"name" attribute must have a value`);
        if (typeof attr.value !== "string" && !(attr.value instanceof RegExp))
          throw new Error(`"name" attribute must be a string or a regular expression`);
        options.name = attr.value;
        options.nameOp = attr.op;
        options.nameExact = attr.caseSensitive;
        break;
      }
      case "description": {
        if (attr.op === "<truthy>")
          throw new Error(`"description" attribute must have a value`);
        if (typeof attr.value !== "string" && !(attr.value instanceof RegExp))
          throw new Error(`"description" attribute must be a string or a regular expression`);
        options.description = attr.value;
        options.descriptionOp = attr.op;
        options.descriptionExact = attr.caseSensitive;
        break;
      }
      case "include-hidden": {
        validateSupportedValues(attr, [true, false]);
        validateSupportedOp(attr, ["<truthy>", "="]);
        options.includeHidden = attr.op === "<truthy>" ? true : attr.value;
        break;
      }
      default: {
        throw new Error(`Unknown attribute "${attr.name}", must be one of ${kSupportedAttributes.map((a) => `"${a}"`).join(", ")}.`);
      }
    }
  }
  return options;
}
function queryRole(scope, options, internal) {
  const result = [];
  const match = (element) => {
    if (getAriaRole(element) !== options.role)
      return;
    if (options.selected !== void 0 && getAriaSelected(element) !== options.selected)
      return;
    if (options.checked !== void 0 && getAriaChecked(element) !== options.checked)
      return;
    if (options.pressed !== void 0 && getAriaPressed(element) !== options.pressed)
      return;
    if (options.expanded !== void 0 && getAriaExpanded(element) !== options.expanded)
      return;
    if (options.level !== void 0 && getAriaLevel(element) !== options.level)
      return;
    if (options.disabled !== void 0 && getAriaDisabled2(element) !== options.disabled)
      return;
    if (!options.includeHidden && isElementHiddenForAria(element))
      return;
    if (options.name !== void 0) {
      const accessibleName = normalizeWhiteSpace(getElementAccessibleNameText(element, !!options.includeHidden));
      if (typeof options.name === "string")
        options.name = normalizeWhiteSpace(options.name);
      if (internal && !options.nameExact && options.nameOp === "=")
        options.nameOp = "*=";
      if (!matchesAttributePart(accessibleName, { name: "", jsonPath: [], op: options.nameOp || "=", value: options.name, caseSensitive: !!options.nameExact }))
        return;
    }
    if (options.description !== void 0) {
      const accessibleDescription = normalizeWhiteSpace(getElementAccessibleDescription(element, !!options.includeHidden).text);
      if (typeof options.description === "string")
        options.description = normalizeWhiteSpace(options.description);
      if (internal && !options.descriptionExact && options.descriptionOp === "=")
        options.descriptionOp = "*=";
      if (!matchesAttributePart(accessibleDescription, { name: "", jsonPath: [], op: options.descriptionOp || "=", value: options.description, caseSensitive: !!options.descriptionExact }))
        return;
    }
    result.push(element);
  };
  const query = (root) => {
    const shadows = [];
    if (root.shadowRoot)
      shadows.push(root.shadowRoot);
    for (const element of root.querySelectorAll("*")) {
      match(element);
      if (element.shadowRoot)
        shadows.push(element.shadowRoot);
    }
    shadows.forEach(query);
  };
  query(scope);
  return result;
}
function createRoleEngine(internal) {
  return {
    queryAll: (scope, selector) => {
      const parsed = parseAttributeSelector(selector, true);
      const role = parsed.name.toLowerCase();
      if (!role)
        throw new Error(`Role must not be empty`);
      const options = validateAttributes(parsed.attributes, role);
      beginAriaCaches();
      try {
        return queryRole(scope, options, internal);
      } finally {
        endAriaCaches();
      }
    }
  };
}

// src/vendor/injected/layoutSelectorUtils.ts
function boxRightOf(box1, box2, maxDistance) {
  const distance = box1.left - box2.right;
  if (distance < 0 || maxDistance !== void 0 && distance > maxDistance)
    return;
  return distance + Math.max(box2.bottom - box1.bottom, 0) + Math.max(box1.top - box2.top, 0);
}
function boxLeftOf(box1, box2, maxDistance) {
  const distance = box2.left - box1.right;
  if (distance < 0 || maxDistance !== void 0 && distance > maxDistance)
    return;
  return distance + Math.max(box2.bottom - box1.bottom, 0) + Math.max(box1.top - box2.top, 0);
}
function boxAbove(box1, box2, maxDistance) {
  const distance = box2.top - box1.bottom;
  if (distance < 0 || maxDistance !== void 0 && distance > maxDistance)
    return;
  return distance + Math.max(box1.left - box2.left, 0) + Math.max(box2.right - box1.right, 0);
}
function boxBelow(box1, box2, maxDistance) {
  const distance = box1.top - box2.bottom;
  if (distance < 0 || maxDistance !== void 0 && distance > maxDistance)
    return;
  return distance + Math.max(box1.left - box2.left, 0) + Math.max(box2.right - box1.right, 0);
}
function boxNear(box1, box2, maxDistance) {
  const kThreshold = maxDistance === void 0 ? 50 : maxDistance;
  let score = 0;
  if (box1.left - box2.right >= 0)
    score += box1.left - box2.right;
  if (box2.left - box1.right >= 0)
    score += box2.left - box1.right;
  if (box2.top - box1.bottom >= 0)
    score += box2.top - box1.bottom;
  if (box1.top - box2.bottom >= 0)
    score += box1.top - box2.bottom;
  return score > kThreshold ? void 0 : score;
}
var kLayoutSelectorNames = ["left-of", "right-of", "above", "below", "near"];
function layoutSelectorScore(name, element, inner, maxDistance) {
  const box = element.getBoundingClientRect();
  const scorer = { "left-of": boxLeftOf, "right-of": boxRightOf, "above": boxAbove, "below": boxBelow, "near": boxNear }[name];
  let bestScore;
  for (const e of inner) {
    if (e === element)
      continue;
    const score = scorer(box, e.getBoundingClientRect(), maxDistance);
    if (score === void 0)
      continue;
    if (bestScore === void 0 || score < bestScore)
      bestScore = score;
  }
  return bestScore;
}

// src/vendor/injected/selectorEvaluator.ts
var SelectorEvaluatorImpl = class {
  constructor() {
    this._retainCacheCounter = 0;
    this._cacheText = /* @__PURE__ */ new Map();
    this._cacheQueryCSS = /* @__PURE__ */ new Map();
    this._cacheMatches = /* @__PURE__ */ new Map();
    this._cacheQuery = /* @__PURE__ */ new Map();
    this._cacheMatchesSimple = /* @__PURE__ */ new Map();
    this._cacheMatchesParents = /* @__PURE__ */ new Map();
    this._cacheCallMatches = /* @__PURE__ */ new Map();
    this._cacheCallQuery = /* @__PURE__ */ new Map();
    this._cacheQuerySimple = /* @__PURE__ */ new Map();
    this._engines = /* @__PURE__ */ new Map();
    this._engines.set("not", notEngine);
    this._engines.set("is", isEngine);
    this._engines.set("where", isEngine);
    this._engines.set("has", hasEngine);
    this._engines.set("scope", scopeEngine);
    this._engines.set("light", lightEngine);
    this._engines.set("visible", visibleEngine);
    this._engines.set("text", textEngine);
    this._engines.set("text-is", textIsEngine);
    this._engines.set("text-matches", textMatchesEngine);
    this._engines.set("has-text", hasTextEngine);
    this._engines.set("right-of", createLayoutEngine("right-of"));
    this._engines.set("left-of", createLayoutEngine("left-of"));
    this._engines.set("above", createLayoutEngine("above"));
    this._engines.set("below", createLayoutEngine("below"));
    this._engines.set("near", createLayoutEngine("near"));
    this._engines.set("nth-match", nthMatchEngine);
    const allNames = [...this._engines.keys()];
    allNames.sort();
    const parserNames = [...customCSSNames];
    parserNames.sort();
    if (allNames.join("|") !== parserNames.join("|"))
      throw new Error(`Please keep customCSSNames in sync with evaluator engines: ${allNames.join("|")} vs ${parserNames.join("|")}`);
  }
  begin() {
    ++this._retainCacheCounter;
  }
  end() {
    --this._retainCacheCounter;
    if (!this._retainCacheCounter) {
      this._cacheQueryCSS.clear();
      this._cacheMatches.clear();
      this._cacheQuery.clear();
      this._cacheMatchesSimple.clear();
      this._cacheMatchesParents.clear();
      this._cacheCallMatches.clear();
      this._cacheCallQuery.clear();
      this._cacheQuerySimple.clear();
      this._cacheText.clear();
    }
  }
  _cached(cache, main, rest, cb) {
    if (!cache.has(main))
      cache.set(main, []);
    const entries = cache.get(main);
    const entry = entries.find((e) => rest.every((value, index) => e.rest[index] === value));
    if (entry)
      return entry.result;
    const result = cb();
    entries.push({ rest, result });
    return result;
  }
  _checkSelector(s) {
    const wellFormed = typeof s === "object" && s && (Array.isArray(s) || "simples" in s && s.simples.length);
    if (!wellFormed)
      throw new Error(`Malformed selector "${s}"`);
    return s;
  }
  matches(element, s, context) {
    const selector = this._checkSelector(s);
    this.begin();
    try {
      return this._cached(this._cacheMatches, element, [selector, context.scope, context.pierceShadow, context.originalScope], () => {
        if (Array.isArray(selector))
          return this._matchesEngine(isEngine, element, selector, context);
        if (this._hasScopeClause(selector))
          context = this._expandContextForScopeMatching(context);
        if (!this._matchesSimple(element, selector.simples[selector.simples.length - 1].selector, context))
          return false;
        return this._matchesParents(element, selector, selector.simples.length - 2, context);
      });
    } finally {
      this.end();
    }
  }
  query(context, s) {
    const selector = this._checkSelector(s);
    this.begin();
    try {
      return this._cached(this._cacheQuery, selector, [context.scope, context.pierceShadow, context.originalScope], () => {
        if (Array.isArray(selector))
          return this._queryEngine(isEngine, context, selector);
        if (this._hasScopeClause(selector))
          context = this._expandContextForScopeMatching(context);
        const previousScoreMap = this._scoreMap;
        this._scoreMap = /* @__PURE__ */ new Map();
        let elements = this._querySimple(context, selector.simples[selector.simples.length - 1].selector);
        elements = elements.filter((element) => this._matchesParents(element, selector, selector.simples.length - 2, context));
        if (this._scoreMap.size) {
          elements.sort((a, b) => {
            const aScore = this._scoreMap.get(a);
            const bScore = this._scoreMap.get(b);
            if (aScore === bScore)
              return 0;
            if (aScore === void 0)
              return 1;
            if (bScore === void 0)
              return -1;
            return aScore - bScore;
          });
        }
        this._scoreMap = previousScoreMap;
        return elements;
      });
    } finally {
      this.end();
    }
  }
  _markScore(element, score) {
    if (this._scoreMap)
      this._scoreMap.set(element, score);
  }
  _hasScopeClause(selector) {
    return selector.simples.some((simple) => simple.selector.functions.some((f) => f.name === "scope"));
  }
  _expandContextForScopeMatching(context) {
    if (context.scope.nodeType !== 1)
      return context;
    const scope = parentElementOrShadowHost(context.scope);
    if (!scope)
      return context;
    return { ...context, scope, originalScope: context.originalScope || context.scope };
  }
  _matchesSimple(element, simple, context) {
    return this._cached(this._cacheMatchesSimple, element, [simple, context.scope, context.pierceShadow, context.originalScope], () => {
      if (element === context.scope)
        return false;
      if (simple.css && !this._matchesCSS(element, simple.css))
        return false;
      for (const func of simple.functions) {
        if (!this._matchesEngine(this._getEngine(func.name), element, func.args, context))
          return false;
      }
      return true;
    });
  }
  _querySimple(context, simple) {
    if (!simple.functions.length)
      return this._queryCSS(context, simple.css || "*");
    return this._cached(this._cacheQuerySimple, simple, [context.scope, context.pierceShadow, context.originalScope], () => {
      let css = simple.css;
      const funcs = simple.functions;
      if (css === "*" && funcs.length)
        css = void 0;
      let elements;
      let firstIndex = -1;
      if (css !== void 0) {
        elements = this._queryCSS(context, css);
      } else {
        firstIndex = funcs.findIndex((func) => this._getEngine(func.name).query !== void 0);
        if (firstIndex === -1)
          firstIndex = 0;
        elements = this._queryEngine(this._getEngine(funcs[firstIndex].name), context, funcs[firstIndex].args);
      }
      for (let i = 0; i < funcs.length; i++) {
        if (i === firstIndex)
          continue;
        const engine = this._getEngine(funcs[i].name);
        if (engine.matches !== void 0)
          elements = elements.filter((e) => this._matchesEngine(engine, e, funcs[i].args, context));
      }
      for (let i = 0; i < funcs.length; i++) {
        if (i === firstIndex)
          continue;
        const engine = this._getEngine(funcs[i].name);
        if (engine.matches === void 0)
          elements = elements.filter((e) => this._matchesEngine(engine, e, funcs[i].args, context));
      }
      return elements;
    });
  }
  _matchesParents(element, complex, index, context) {
    if (index < 0)
      return true;
    return this._cached(this._cacheMatchesParents, element, [complex, index, context.scope, context.pierceShadow, context.originalScope], () => {
      const { selector: simple, combinator } = complex.simples[index];
      if (combinator === ">") {
        const parent = parentElementOrShadowHostInContext(element, context);
        if (!parent || !this._matchesSimple(parent, simple, context))
          return false;
        return this._matchesParents(parent, complex, index - 1, context);
      }
      if (combinator === "+") {
        const previousSibling = previousSiblingInContext(element, context);
        if (!previousSibling || !this._matchesSimple(previousSibling, simple, context))
          return false;
        return this._matchesParents(previousSibling, complex, index - 1, context);
      }
      if (combinator === "") {
        let parent = parentElementOrShadowHostInContext(element, context);
        while (parent) {
          if (this._matchesSimple(parent, simple, context)) {
            if (this._matchesParents(parent, complex, index - 1, context))
              return true;
            if (complex.simples[index - 1].combinator === "")
              break;
          }
          parent = parentElementOrShadowHostInContext(parent, context);
        }
        return false;
      }
      if (combinator === "~") {
        let previousSibling = previousSiblingInContext(element, context);
        while (previousSibling) {
          if (this._matchesSimple(previousSibling, simple, context)) {
            if (this._matchesParents(previousSibling, complex, index - 1, context))
              return true;
            if (complex.simples[index - 1].combinator === "~")
              break;
          }
          previousSibling = previousSiblingInContext(previousSibling, context);
        }
        return false;
      }
      if (combinator === ">=") {
        let parent = element;
        while (parent) {
          if (this._matchesSimple(parent, simple, context)) {
            if (this._matchesParents(parent, complex, index - 1, context))
              return true;
            if (complex.simples[index - 1].combinator === "")
              break;
          }
          parent = parentElementOrShadowHostInContext(parent, context);
        }
        return false;
      }
      throw new Error(`Unsupported combinator "${combinator}"`);
    });
  }
  _matchesEngine(engine, element, args, context) {
    if (engine.matches)
      return this._callMatches(engine, element, args, context);
    if (engine.query)
      return this._callQuery(engine, args, context).includes(element);
    throw new Error(`Selector engine should implement "matches" or "query"`);
  }
  _queryEngine(engine, context, args) {
    if (engine.query)
      return this._callQuery(engine, args, context);
    if (engine.matches)
      return this._queryCSS(context, "*").filter((element) => this._callMatches(engine, element, args, context));
    throw new Error(`Selector engine should implement "matches" or "query"`);
  }
  _callMatches(engine, element, args, context) {
    return this._cached(this._cacheCallMatches, element, [engine, context.scope, context.pierceShadow, context.originalScope, ...args], () => {
      return engine.matches(element, args, context, this);
    });
  }
  _callQuery(engine, args, context) {
    return this._cached(this._cacheCallQuery, engine, [context.scope, context.pierceShadow, context.originalScope, ...args], () => {
      return engine.query(context, args, this);
    });
  }
  _matchesCSS(element, css) {
    return element.matches(css);
  }
  _queryCSS(context, css) {
    return this._cached(this._cacheQueryCSS, css, [context.scope, context.pierceShadow, context.originalScope], () => {
      let result = [];
      function query(root) {
        result = result.concat([...root.querySelectorAll(css)]);
        if (!context.pierceShadow)
          return;
        if (root.shadowRoot)
          query(root.shadowRoot);
        for (const element of root.querySelectorAll("*")) {
          if (element.shadowRoot)
            query(element.shadowRoot);
        }
      }
      query(context.scope);
      return result;
    });
  }
  _getEngine(name) {
    const engine = this._engines.get(name);
    if (!engine)
      throw new Error(`Unknown selector engine "${name}"`);
    return engine;
  }
};
var isEngine = {
  matches(element, args, context, evaluator) {
    if (args.length === 0)
      throw new Error(`"is" engine expects non-empty selector list`);
    return args.some((selector) => evaluator.matches(element, selector, context));
  },
  query(context, args, evaluator) {
    if (args.length === 0)
      throw new Error(`"is" engine expects non-empty selector list`);
    let elements = [];
    for (const arg of args)
      elements = elements.concat(evaluator.query(context, arg));
    return args.length === 1 ? elements : sortInDOMOrder(elements);
  }
};
var hasEngine = {
  matches(element, args, context, evaluator) {
    if (args.length === 0)
      throw new Error(`"has" engine expects non-empty selector list`);
    return evaluator.query({ ...context, scope: element }, args).length > 0;
  }
  // TODO: we can implement efficient "query" by matching "args" and returning
  // all parents/descendants, just have to be careful with the ":scope" matching.
};
var scopeEngine = {
  matches(element, args, context, evaluator) {
    if (args.length !== 0)
      throw new Error(`"scope" engine expects no arguments`);
    const actualScope = context.originalScope || context.scope;
    if (actualScope.nodeType === 9)
      return element === actualScope.documentElement;
    return element === actualScope;
  },
  query(context, args, evaluator) {
    if (args.length !== 0)
      throw new Error(`"scope" engine expects no arguments`);
    const actualScope = context.originalScope || context.scope;
    if (actualScope.nodeType === 9) {
      const root = actualScope.documentElement;
      return root ? [root] : [];
    }
    if (actualScope.nodeType === 1)
      return [actualScope];
    return [];
  }
};
var notEngine = {
  matches(element, args, context, evaluator) {
    if (args.length === 0)
      throw new Error(`"not" engine expects non-empty selector list`);
    return !evaluator.matches(element, args, context);
  }
};
var lightEngine = {
  query(context, args, evaluator) {
    return evaluator.query({ ...context, pierceShadow: false }, args);
  },
  matches(element, args, context, evaluator) {
    return evaluator.matches(element, args, { ...context, pierceShadow: false });
  }
};
var visibleEngine = {
  matches(element, args, context, evaluator) {
    if (args.length)
      throw new Error(`"visible" engine expects no arguments`);
    return isElementVisible(element);
  }
};
var textEngine = {
  matches(element, args, context, evaluator) {
    if (args.length !== 1 || typeof args[0] !== "string")
      throw new Error(`"text" engine expects a single string`);
    const text = normalizeWhiteSpace(args[0]).toLowerCase();
    const matcher = (elementText2) => elementText2.normalized.toLowerCase().includes(text);
    return elementMatchesText(evaluator._cacheText, element, matcher) === "self";
  }
};
var textIsEngine = {
  matches(element, args, context, evaluator) {
    if (args.length !== 1 || typeof args[0] !== "string")
      throw new Error(`"text-is" engine expects a single string`);
    const text = normalizeWhiteSpace(args[0]);
    const matcher = (elementText2) => {
      if (!text && !elementText2.immediate.length)
        return true;
      return elementText2.immediate.some((s) => normalizeWhiteSpace(s) === text);
    };
    return elementMatchesText(evaluator._cacheText, element, matcher) !== "none";
  }
};
var textMatchesEngine = {
  matches(element, args, context, evaluator) {
    if (args.length === 0 || typeof args[0] !== "string" || args.length > 2 || args.length === 2 && typeof args[1] !== "string")
      throw new Error(`"text-matches" engine expects a regexp body and optional regexp flags`);
    const re = new RegExp(args[0], args.length === 2 ? args[1] : void 0);
    const matcher = (elementText2) => re.test(elementText2.full);
    return elementMatchesText(evaluator._cacheText, element, matcher) === "self";
  }
};
var hasTextEngine = {
  matches(element, args, context, evaluator) {
    if (args.length !== 1 || typeof args[0] !== "string")
      throw new Error(`"has-text" engine expects a single string`);
    if (shouldSkipForTextMatching(element))
      return false;
    const text = normalizeWhiteSpace(args[0]).toLowerCase();
    const matcher = (elementText2) => elementText2.normalized.toLowerCase().includes(text);
    return matcher(elementText(evaluator._cacheText, element));
  }
};
function createLayoutEngine(name) {
  return {
    matches(element, args, context, evaluator) {
      const maxDistance = args.length && typeof args[args.length - 1] === "number" ? args[args.length - 1] : void 0;
      const queryArgs = maxDistance === void 0 ? args : args.slice(0, args.length - 1);
      if (args.length < 1 + (maxDistance === void 0 ? 0 : 1))
        throw new Error(`"${name}" engine expects a selector list and optional maximum distance in pixels`);
      const inner = evaluator.query(context, queryArgs);
      const score = layoutSelectorScore(name, element, inner, maxDistance);
      if (score === void 0)
        return false;
      evaluator._markScore(element, score);
      return true;
    }
  };
}
var nthMatchEngine = {
  query(context, args, evaluator) {
    let index = args[args.length - 1];
    if (args.length < 2)
      throw new Error(`"nth-match" engine expects non-empty selector list and an index argument`);
    if (typeof index !== "number" || index < 1)
      throw new Error(`"nth-match" engine expects a one-based index as the last argument`);
    const elements = isEngine.query(context, args.slice(0, args.length - 1), evaluator);
    index--;
    return index < elements.length ? [elements[index]] : [];
  }
};
function parentElementOrShadowHostInContext(element, context) {
  if (element === context.scope)
    return;
  if (!context.pierceShadow)
    return element.parentElement || void 0;
  return parentElementOrShadowHost(element);
}
function previousSiblingInContext(element, context) {
  if (element === context.scope)
    return;
  return element.previousElementSibling || void 0;
}
function sortInDOMOrder(elements) {
  const elementToEntry = /* @__PURE__ */ new Map();
  const roots = [];
  const result = [];
  function append(element) {
    let entry = elementToEntry.get(element);
    if (entry)
      return entry;
    const parent = parentElementOrShadowHost(element);
    if (parent) {
      const parentEntry = append(parent);
      parentEntry.children.push(element);
    } else {
      roots.push(element);
    }
    entry = { children: [], taken: false };
    elementToEntry.set(element, entry);
    return entry;
  }
  for (const e of elements)
    append(e).taken = true;
  function visit(element) {
    const entry = elementToEntry.get(element);
    if (entry.taken)
      result.push(element);
    if (entry.children.length > 1) {
      const set = new Set(entry.children);
      entry.children = [];
      let child = element.firstElementChild;
      while (child && entry.children.length < set.size) {
        if (set.has(child))
          entry.children.push(child);
        child = child.nextElementSibling;
      }
      child = element.shadowRoot ? element.shadowRoot.firstElementChild : null;
      while (child && entry.children.length < set.size) {
        if (set.has(child))
          entry.children.push(child);
        child = child.nextElementSibling;
      }
    }
    entry.children.forEach(visit);
  }
  roots.forEach(visit);
  return result;
}

// src/vendor/injected/xpathSelectorEngine.ts
var XPathEngine = {
  queryAll(root, selector) {
    if (selector.startsWith("/") && root.nodeType !== Node.DOCUMENT_NODE)
      selector = "." + selector;
    const result = [];
    const document = root.ownerDocument || root;
    if (!document)
      return result;
    const it = document.evaluate(selector, root, null, XPathResult.ORDERED_NODE_ITERATOR_TYPE);
    for (let node = it.iterateNext(); node; node = it.iterateNext()) {
      if (node.nodeType === Node.ELEMENT_NODE)
        result.push(node);
    }
    return result;
  }
};

// src/vendor/injected/selectorGenerator.ts
function splitTestIdAttributeNames(testIdAttributeName) {
  return testIdAttributeName.split(",");
}
var kTextScoreRange = 10;
var kExactPenalty = kTextScoreRange / 2;
var kTestIdScore = 1;
var kOtherTestIdScore = 2;
var kIframeByAttributeScore = 10;
var kBeginPenalizedScore = 50;
var kRoleWithNameScore = 100;
var kTextScore = 120;
var kLabelScore = 140;
var kPlaceholderScore = 160;
var kAltTextScore = 180;
var kTitleScore = 200;
var kTextScoreRegex = 250;
var kPlaceholderScoreExact = kPlaceholderScore + kExactPenalty;
var kLabelScoreExact = kLabelScore + kExactPenalty;
var kRoleWithNameScoreExact = kRoleWithNameScore + kExactPenalty;
var kAltTextScoreExact = kAltTextScore + kExactPenalty;
var kTextScoreExact = kTextScore + kExactPenalty;
var kTitleScoreExact = kTitleScore + kExactPenalty;
var kEndPenalizedScore = 300;
var kCSSIdScore = 500;
var kRoleWithoutNameScore = 510;
var kCSSInputTypeNameScore = 520;
var kCSSTagNameScore = 530;
var kNthScore = 1e4;
var kCSSFallbackScore = 1e7;
var kScoreThresholdForTextExpect = 1e3;
function generateSelector(injectedScript, targetElement, options) {
  var _a;
  injectedScript._evaluator.begin();
  const cache = { allowText: /* @__PURE__ */ new Map(), disallowText: /* @__PURE__ */ new Map() };
  beginAriaCaches();
  beginDOMCaches();
  try {
    let targetTokens;
    if (options.forTextExpect) {
      targetTokens = cssFallback(injectedScript, targetElement.ownerDocument.documentElement, options);
      for (let element = targetElement; element; element = parentElementOrShadowHost(element)) {
        const tokens = generateSelectorFor(cache, injectedScript, element, { ...options, noText: true });
        if (!tokens)
          continue;
        const score = combineScores(tokens);
        if (score <= kScoreThresholdForTextExpect) {
          targetTokens = tokens;
          break;
        }
      }
    } else {
      if (!targetElement.matches("input,textarea,select") && !targetElement.isContentEditable) {
        const interactiveParent = closestCrossShadow(targetElement, "button,select,input,[role=button],[role=checkbox],[role=radio],a,[role=link]", options.root);
        if (interactiveParent && isElementVisible(interactiveParent))
          targetElement = interactiveParent;
      }
      targetTokens = generateSelectorFor(cache, injectedScript, targetElement, options) || cssFallback(injectedScript, targetElement, options);
    }
    const selector = joinTokens(targetTokens);
    const parsedSelector = injectedScript.parseSelector(selector);
    return {
      selector,
      elements: injectedScript.querySelectorAll(parsedSelector, (_a = options.root) != null ? _a : targetElement.ownerDocument)
    };
  } finally {
    endDOMCaches();
    endAriaCaches();
    injectedScript._evaluator.end();
  }
}
function generateSelectorFor(cache, injectedScript, targetElement, options) {
  var _a;
  if (options.root && !isInsideScope(options.root, targetElement))
    throw new Error(`Target element must belong to the root's subtree`);
  if (targetElement === options.root)
    return [{ engine: "css", selector: ":scope", score: 1 }];
  if (targetElement.ownerDocument.documentElement === targetElement)
    return [{ engine: "css", selector: "html", score: 1 }];
  let result = null;
  const updateResult = (candidate) => {
    if (!result || combineScores(candidate) < combineScores(result))
      result = candidate;
  };
  const candidates = [];
  for (const candidate of buildTextCandidates(injectedScript, targetElement, !options.isRecursive, options))
    candidates.push({ candidate, isTextCandidate: true });
  for (const token of buildNoTextCandidates(injectedScript, targetElement, options)) {
    if (options.omitInternalEngines && token.engine.startsWith("internal:"))
      continue;
    candidates.push({ candidate: [token], isTextCandidate: false });
  }
  candidates.sort((a, b) => combineScores(a.candidate) - combineScores(b.candidate));
  for (const { candidate, isTextCandidate } of candidates) {
    const elements = injectedScript.querySelectorAll(injectedScript.parseSelector(joinTokens(candidate)), (_a = options.root) != null ? _a : targetElement.ownerDocument);
    if (!elements.includes(targetElement)) {
      continue;
    }
    if (elements.length === 1) {
      updateResult(candidate);
      break;
    }
    const index = elements.indexOf(targetElement);
    if (index > 5) {
      continue;
    }
    updateResult([...candidate, { engine: "nth", selector: String(index), score: kNthScore }]);
    if (options.isRecursive) {
      continue;
    }
    for (let parent = parentElementOrShadowHost(targetElement); parent && parent !== options.root; parent = parentElementOrShadowHost(parent)) {
      const filtered = elements.filter((e) => isInsideScope(parent, e) && e !== parent);
      const newIndex = filtered.indexOf(targetElement);
      if (filtered.length > 5 || newIndex === -1 || newIndex === index && filtered.length > 1) {
        continue;
      }
      const inParent = filtered.length === 1 ? candidate : [...candidate, { engine: "nth", selector: String(newIndex), score: kNthScore }];
      const idealSelectorForParent = { engine: "", selector: "", score: 1 };
      if (result && combineScores([idealSelectorForParent, ...inParent]) >= combineScores(result)) {
        continue;
      }
      const noText = !!options.noText || isTextCandidate;
      const cacheMap = noText ? cache.disallowText : cache.allowText;
      let parentTokens = cacheMap.get(parent);
      if (parentTokens === void 0) {
        parentTokens = generateSelectorFor(cache, injectedScript, parent, { ...options, isRecursive: true, noText }) || cssFallback(injectedScript, parent, options);
        cacheMap.set(parent, parentTokens);
      }
      if (!parentTokens)
        continue;
      updateResult([...parentTokens, ...inParent]);
    }
  }
  return result;
}
function buildNoTextCandidates(injectedScript, element, options) {
  const candidates = [];
  const testIdAttributeNames = splitTestIdAttributeNames(options.testIdAttributeName);
  {
    for (const attr of ["data-testid", "data-test-id", "data-test"]) {
      if (!testIdAttributeNames.includes(attr) && element.getAttribute(attr))
        candidates.push({ engine: "css", selector: `[${attr}=${quoteCSSAttributeValue(element.getAttribute(attr))}]`, score: kOtherTestIdScore });
    }
    const idAttr = element.getAttribute("id");
    if (idAttr && !isGuidLike(idAttr))
      candidates.push({ engine: "css", selector: makeSelectorForId(idAttr), score: kCSSIdScore });
    candidates.push({ engine: "css", selector: escapeNodeName(element), score: kCSSTagNameScore });
  }
  if (element.nodeName === "IFRAME" || element.nodeName === "FRAME") {
    for (const attribute of ["name", "title"]) {
      if (element.getAttribute(attribute))
        candidates.push({ engine: "css", selector: `${escapeNodeName(element)}[${attribute}=${quoteCSSAttributeValue(element.getAttribute(attribute))}]`, score: kIframeByAttributeScore });
    }
    for (const testIdAttr of testIdAttributeNames) {
      if (element.getAttribute(testIdAttr))
        candidates.push({ engine: "css", selector: `[${testIdAttr}=${quoteCSSAttributeValue(element.getAttribute(testIdAttr))}]`, score: kTestIdScore });
    }
    penalizeScoreForLength([candidates]);
    return candidates;
  }
  for (const testIdAttr of testIdAttributeNames) {
    if (element.getAttribute(testIdAttr))
      candidates.push({ engine: "internal:testid", selector: `[${testIdAttr}=${escapeForAttributeSelector(element.getAttribute(testIdAttr), true)}]`, score: kTestIdScore });
  }
  if (element.nodeName === "INPUT" || element.nodeName === "TEXTAREA") {
    const input = element;
    if (input.placeholder) {
      candidates.push({ engine: "internal:attr", selector: `[placeholder=${escapeForAttributeSelector(input.placeholder, true)}]`, score: kPlaceholderScoreExact });
      for (const alternative of suitableTextAlternatives(input.placeholder))
        candidates.push({ engine: "internal:attr", selector: `[placeholder=${escapeForAttributeSelector(alternative.text, false)}]`, score: kPlaceholderScore - alternative.scoreBonus });
    }
  }
  const labels = getElementLabels(injectedScript._evaluator._cacheText, element, { skipRefsInsideElement: options.noText });
  for (const label of labels) {
    const labelText = label.normalized;
    candidates.push({ engine: "internal:label", selector: escapeForTextSelector(labelText, true), score: kLabelScoreExact });
    for (const alternative of suitableTextAlternatives(labelText))
      candidates.push({ engine: "internal:label", selector: escapeForTextSelector(alternative.text, false), score: kLabelScore - alternative.scoreBonus });
  }
  const ariaRole = getAriaRole(element);
  if (ariaRole && !["none", "presentation"].includes(ariaRole))
    candidates.push({ engine: "internal:role", selector: ariaRole, score: kRoleWithoutNameScore });
  if (element.getAttribute("name") && ["BUTTON", "FORM", "FIELDSET", "FRAME", "IFRAME", "INPUT", "KEYGEN", "OBJECT", "OUTPUT", "SELECT", "TEXTAREA", "MAP", "META", "PARAM"].includes(element.nodeName))
    candidates.push({ engine: "css", selector: `${escapeNodeName(element)}[name=${quoteCSSAttributeValue(element.getAttribute("name"))}]`, score: kCSSInputTypeNameScore });
  if (["INPUT", "TEXTAREA"].includes(element.nodeName) && element.getAttribute("type") !== "hidden") {
    if (element.getAttribute("type"))
      candidates.push({ engine: "css", selector: `${escapeNodeName(element)}[type=${quoteCSSAttributeValue(element.getAttribute("type"))}]`, score: kCSSInputTypeNameScore });
  }
  if (["INPUT", "TEXTAREA", "SELECT"].includes(element.nodeName) && element.getAttribute("type") !== "hidden")
    candidates.push({ engine: "css", selector: escapeNodeName(element), score: kCSSInputTypeNameScore + 1 });
  penalizeScoreForLength([candidates]);
  return candidates;
}
function buildTextCandidates(injectedScript, element, isTargetNode, options) {
  if (element.nodeName === "SELECT")
    return [];
  const candidates = [];
  if (!options.noText) {
    const title = element.getAttribute("title");
    if (title) {
      candidates.push([{ engine: "internal:attr", selector: `[title=${escapeForAttributeSelector(title, true)}]`, score: kTitleScoreExact }]);
      for (const alternative of suitableTextAlternatives(title))
        candidates.push([{ engine: "internal:attr", selector: `[title=${escapeForAttributeSelector(alternative.text, false)}]`, score: kTitleScore - alternative.scoreBonus }]);
    }
    const alt = element.getAttribute("alt");
    if (alt && ["APPLET", "AREA", "IMG", "INPUT"].includes(element.nodeName)) {
      candidates.push([{ engine: "internal:attr", selector: `[alt=${escapeForAttributeSelector(alt, true)}]`, score: kAltTextScoreExact }]);
      for (const alternative of suitableTextAlternatives(alt))
        candidates.push([{ engine: "internal:attr", selector: `[alt=${escapeForAttributeSelector(alternative.text, false)}]`, score: kAltTextScore - alternative.scoreBonus }]);
    }
  }
  const text = options.noText ? "" : elementText(injectedScript._evaluator._cacheText, element).normalized;
  const textAlternatives = text ? suitableTextAlternatives(text) : [];
  if (text) {
    if (isTargetNode) {
      if (text.length <= 80)
        candidates.push([{ engine: "internal:text", selector: escapeForTextSelector(text, true), score: kTextScoreExact }]);
      for (const alternative of textAlternatives)
        candidates.push([{ engine: "internal:text", selector: escapeForTextSelector(alternative.text, false), score: kTextScore - alternative.scoreBonus }]);
    }
    const cssToken = { engine: "css", selector: escapeNodeName(element), score: kCSSTagNameScore };
    for (const alternative of textAlternatives)
      candidates.push([cssToken, { engine: "internal:has-text", selector: escapeForTextSelector(alternative.text, false), score: kTextScore - alternative.scoreBonus }]);
    if (isTargetNode && text.length <= 80) {
      const re = new RegExp("^" + escapeRegExp(text) + "$");
      candidates.push([cssToken, { engine: "internal:has-text", selector: escapeForTextSelector(re, false), score: kTextScoreRegex }]);
    }
  }
  const ariaRole = getAriaRole(element);
  if (ariaRole && !["none", "presentation"].includes(ariaRole)) {
    const accessibleName = getElementAccessibleName(element, false);
    const ariaName = options.noText && accessibleName.derivedFromContent ? "" : accessibleName.text;
    const accessibleDescription = getElementAccessibleDescription(element, false);
    const ariaDescription = options.noText && accessibleDescription.derivedFromContent ? "" : accessibleDescription.text;
    if (ariaName && !ariaName.match(/^\p{Co}+$/u)) {
      const roleToken = { engine: "internal:role", selector: `${ariaRole}[name=${escapeForAttributeSelector(ariaName, true)}]`, score: kRoleWithNameScoreExact };
      candidates.push([roleToken]);
      for (const alternative of suitableTextAlternatives(ariaName))
        candidates.push([{ engine: "internal:role", selector: `${ariaRole}[name=${escapeForAttributeSelector(alternative.text, false)}]`, score: kRoleWithNameScore - alternative.scoreBonus }]);
      if (ariaDescription) {
        candidates.push([{ engine: "internal:role", selector: `${ariaRole}[name=${escapeForAttributeSelector(ariaName, true)}][description=${escapeForAttributeSelector(ariaDescription, true)}]`, score: kRoleWithNameScoreExact + 1 }]);
        for (const alternative of suitableTextAlternatives(ariaName))
          candidates.push([{ engine: "internal:role", selector: `${ariaRole}[name=${escapeForAttributeSelector(alternative.text, false)}][description=${escapeForAttributeSelector(ariaDescription, false)}]`, score: kRoleWithNameScore - alternative.scoreBonus + 1 }]);
      }
    } else {
      const roleToken = { engine: "internal:role", selector: `${ariaRole}`, score: kRoleWithoutNameScore };
      if (ariaDescription)
        candidates.push([{ engine: "internal:role", selector: `${ariaRole}[description=${escapeForAttributeSelector(ariaDescription, true)}]`, score: kRoleWithoutNameScore + 1 }]);
      for (const alternative of textAlternatives)
        candidates.push([roleToken, { engine: "internal:has-text", selector: escapeForTextSelector(alternative.text, false), score: kTextScore - alternative.scoreBonus }]);
      if (!options.noText && isTargetNode && text.length <= 80) {
        const re = new RegExp("^" + escapeRegExp(text) + "$");
        candidates.push([roleToken, { engine: "internal:has-text", selector: escapeForTextSelector(re, false), score: kTextScoreRegex }]);
      }
    }
  }
  penalizeScoreForLength(candidates);
  return candidates;
}
function makeSelectorForId(id) {
  return /^[a-zA-Z][a-zA-Z0-9\-\_]+$/.test(id) ? "#" + id : `[id=${quoteCSSAttributeValue(id)}]`;
}
function cssFallback(injectedScript, targetElement, options) {
  var _a;
  const root = (_a = options.root) != null ? _a : targetElement.ownerDocument;
  const tokens = [];
  function uniqueCSSSelector(prefix) {
    const path = tokens.slice();
    if (prefix)
      path.unshift(prefix);
    const selector = path.join(" > ");
    const parsedSelector = injectedScript.parseSelector(selector);
    const node = injectedScript.querySelector(parsedSelector, root, false);
    return node === targetElement ? selector : void 0;
  }
  function makeStrict(selector) {
    const token = { engine: "css", selector, score: kCSSFallbackScore };
    const parsedSelector = injectedScript.parseSelector(selector);
    const elements = injectedScript.querySelectorAll(parsedSelector, root);
    if (elements.length === 1)
      return [token];
    const nth = { engine: "nth", selector: String(elements.indexOf(targetElement)), score: kNthScore };
    return [token, nth];
  }
  for (let element = targetElement; element && element !== root; element = parentElementOrShadowHost(element)) {
    let bestTokenForLevel = "";
    if (element.id) {
      const token = makeSelectorForId(element.id);
      const selector = uniqueCSSSelector(token);
      if (selector)
        return makeStrict(selector);
      bestTokenForLevel = token;
    }
    const parent = element.parentNode;
    const classes = [...element.classList].map(escapeClassName);
    for (let i = 0; i < classes.length; ++i) {
      const token = "." + classes.slice(0, i + 1).join(".");
      const selector = uniqueCSSSelector(token);
      if (selector)
        return makeStrict(selector);
      if (!bestTokenForLevel && parent) {
        const sameClassSiblings = parent.querySelectorAll(token);
        if (sameClassSiblings.length === 1)
          bestTokenForLevel = token;
      }
    }
    if (parent) {
      const siblings = [...parent.children];
      const nodeName = element.nodeName;
      const sameTagSiblings = siblings.filter((sibling) => sibling.nodeName === nodeName);
      const token = sameTagSiblings.indexOf(element) === 0 ? escapeNodeName(element) : `${escapeNodeName(element)}:nth-child(${1 + siblings.indexOf(element)})`;
      const selector = uniqueCSSSelector(token);
      if (selector)
        return makeStrict(selector);
      if (!bestTokenForLevel)
        bestTokenForLevel = token;
    } else if (!bestTokenForLevel) {
      bestTokenForLevel = escapeNodeName(element);
    }
    tokens.unshift(bestTokenForLevel);
  }
  return makeStrict(uniqueCSSSelector());
}
function penalizeScoreForLength(groups) {
  for (const group of groups) {
    for (const token of group) {
      if (token.score > kBeginPenalizedScore && token.score < kEndPenalizedScore)
        token.score += Math.min(kTextScoreRange, token.selector.length / 10 | 0);
    }
  }
}
function joinTokens(tokens) {
  const parts = [];
  let lastEngine = "";
  for (const { engine, selector } of tokens) {
    if (parts.length && (lastEngine !== "css" || engine !== "css" || selector.startsWith(":nth-match(")))
      parts.push(">>");
    lastEngine = engine;
    if (engine === "css")
      parts.push(selector);
    else
      parts.push(`${engine}=${selector}`);
  }
  return parts.join(" ");
}
function combineScores(tokens) {
  let score = 0;
  for (let i = 0; i < tokens.length; i++)
    score += tokens[i].score * (tokens.length - i);
  return score;
}
function isGuidLike(id) {
  let lastCharacterType;
  let transitionCount = 0;
  for (let i = 0; i < id.length; ++i) {
    const c = id[i];
    let characterType;
    if (c === "-" || c === "_")
      continue;
    if (c >= "a" && c <= "z")
      characterType = "lower";
    else if (c >= "A" && c <= "Z")
      characterType = "upper";
    else if (c >= "0" && c <= "9")
      characterType = "digit";
    else
      characterType = "other";
    if (characterType === "lower" && lastCharacterType === "upper") {
      lastCharacterType = characterType;
      continue;
    }
    if (lastCharacterType && lastCharacterType !== characterType)
      ++transitionCount;
    lastCharacterType = characterType;
  }
  return transitionCount >= id.length / 4;
}
function trimWordBoundary(text, maxLength) {
  if (text.length <= maxLength)
    return text;
  text = text.substring(0, maxLength);
  const match = text.match(/^(.*)\b(.+?)$/);
  if (!match)
    return "";
  return match[1].trimEnd();
}
function suitableTextAlternatives(text) {
  let result = [];
  {
    const match = text.match(/^([\d.,]+)[^.,\w]/);
    const leadingNumberLength = match ? match[1].length : 0;
    if (leadingNumberLength) {
      const alt = trimWordBoundary(text.substring(leadingNumberLength).trimStart(), 80);
      result.push({ text: alt, scoreBonus: alt.length <= 30 ? 2 : 1 });
    }
  }
  {
    const match = text.match(/[^.,\w]([\d.,]+)$/);
    const trailingNumberLength = match ? match[1].length : 0;
    if (trailingNumberLength) {
      const alt = trimWordBoundary(text.substring(0, text.length - trailingNumberLength).trimEnd(), 80);
      result.push({ text: alt, scoreBonus: alt.length <= 30 ? 2 : 1 });
    }
  }
  if (text.length <= 30) {
    result.push({ text, scoreBonus: 0 });
  } else {
    result.push({ text: trimWordBoundary(text, 80), scoreBonus: 0 });
    result.push({ text: trimWordBoundary(text, 30), scoreBonus: 1 });
  }
  result = result.filter((r) => r.text);
  if (!result.length)
    result.push({ text: text.substring(0, 80), scoreBonus: 0 });
  return result;
}
function escapeNodeName(node) {
  return node.nodeName.toLocaleLowerCase().replace(/[:\.]/g, (char) => "\\" + char);
}
function escapeClassName(className) {
  let result = "";
  for (let i = 0; i < className.length; i++)
    result += cssEscapeCharacter(className, i);
  return result;
}
function cssEscapeCharacter(s, i) {
  const c = s.charCodeAt(i);
  if (c === 0)
    return "�";
  if (c >= 1 && c <= 31 || c >= 48 && c <= 57 && (i === 0 || i === 1 && s.charCodeAt(0) === 45))
    return "\\" + c.toString(16) + " ";
  if (i === 0 && c === 45 && s.length === 1)
    return "\\" + s.charAt(i);
  if (c >= 128 || c === 45 || c === 95 || c >= 48 && c <= 57 || c >= 65 && c <= 90 || c >= 97 && c <= 122)
    return s.charAt(i);
  return "\\" + s.charAt(i);
}

// src/refactInjected.ts
var injectedInstanceName = "__refact_injected__";
var bindingName = "__refact_binding";
function createLocatorRegExp(value) {
  var _a;
  return new RegExp(value.source, (_a = value.flags) != null ? _a : "");
}
function matchesRegExp(expression, value) {
  expression.lastIndex = 0;
  return expression.test(value);
}
function createLocatorTextMatcher(value, exact = false, regex) {
  if (regex) {
    const expression = createLocatorRegExp(regex);
    return { matcher: (text) => matchesRegExp(expression, text.normalized), kind: "regex" };
  }
  const normalized = normalizeWhiteSpace(value);
  if (exact)
    return { matcher: (text) => text.normalized === normalized, kind: "strict" };
  const folded = normalized.toLowerCase();
  return { matcher: (text) => text.normalized.toLowerCase().includes(folded), kind: "lax" };
}
function queryAllPiercingShadow(scope) {
  const result = [];
  const visit = (root) => {
    if (root instanceof Element)
      result.push(root);
    for (const element of root.querySelectorAll("*")) {
      result.push(element);
      if (element.shadowRoot)
        visit(element.shadowRoot);
    }
  };
  visit(scope);
  return result;
}
function queryDescendantsPiercingShadow(scope) {
  const elements = queryAllPiercingShadow(scope);
  if (scope instanceof Element)
    elements.shift();
  return elements;
}
function queryCssPiercingShadow(scope, selector) {
  return queryParsedSelector(parseSelector(`css=${selector}`), scope);
}
function queryByAttribute(scope, attribute, value, exact = false, regex) {
  if (!attribute)
    throw new Error("Test id attribute must not be empty");
  const expression = regex ? createLocatorRegExp(regex) : void 0;
  const expected = normalizeWhiteSpace(value);
  const expectedFolded = expected.toLowerCase();
  return queryDescendantsPiercingShadow(scope).filter((element) => {
    const raw = element.getAttribute(attribute);
    if (raw === null)
      return false;
    const actual = normalizeWhiteSpace(raw);
    if (expression)
      return matchesRegExp(expression, actual);
    return exact ? actual === expected : actual.toLowerCase().includes(expectedFolded);
  });
}
function serializeRoleValue(value, regex, exact) {
  if (regex) {
    const expression = createLocatorRegExp(regex);
    return `/${expression.source}/${expression.flags}`;
  }
  if (value === void 0)
    return void 0;
  return `${JSON.stringify(value)}${exact ? "s" : "i"}`;
}
function serializeRoleSelector(locator) {
  var _a;
  const role = (_a = locator.role) != null ? _a : "";
  const attributes = [];
  const add = (name, value) => {
    if (value !== void 0)
      attributes.push(`[${name}=${String(value)}]`);
  };
  add("checked", locator.checked);
  add("disabled", locator.disabled);
  add("selected", locator.selected);
  add("expanded", locator.expanded);
  add("include-hidden", locator.include_hidden);
  add("level", locator.level);
  add("name", serializeRoleValue(locator.name, locator.name_regex, locator.exact));
  add("description", serializeRoleValue(locator.description, locator.description_regex, locator.exact));
  add("pressed", locator.pressed);
  return `${role}${attributes.join("")}`;
}
function matchesLocatorText(element, matcher) {
  const text = elementText(/* @__PURE__ */ new Map(), element).normalized;
  if (typeof matcher === "string")
    return text.toLowerCase().includes(normalizeWhiteSpace(matcher).toLowerCase());
  return matchesRegExp(createLocatorRegExp(matcher), text);
}
function applyLocatorIndex(elements, locator) {
  const selectors = Number(locator.nth !== void 0) + Number(locator.first === true) + Number(locator.last === true);
  if (selectors > 1)
    throw new Error("Locator can use only one of nth, first, or last");
  if (!elements.length)
    return elements;
  if (locator.first)
    return [elements[0]];
  if (locator.last)
    return [elements[elements.length - 1]];
  if (locator.nth !== void 0) {
    const index = locator.nth < 0 ? elements.length + locator.nth : locator.nth;
    return index >= 0 && index < elements.length ? [elements[index]] : [];
  }
  return elements;
}
var RefactInjected = class {
  constructor(global, builtins) {
    this.global = global;
    this.builtinSnapshot = builtins;
    this.hitTargetController = new HitTargetController(global, builtins);
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
  resolveAll(locator, scopeOverride) {
    var _a, _b, _c, _d, _e, _f, _g, _h, _i, _j, _k, _l;
    const document = this.global.document;
    const scope = scopeOverride != null ? scopeOverride : locator.within ? queryCssPiercingShadow(document, locator.within)[0] : document;
    if (!scope)
      throw new Error("Scope selector not found");
    if (scopeOverride && locator.within)
      throw new Error("Nested relative locators cannot use within");
    let elements;
    switch (locator.by) {
      case "css":
        elements = queryCssPiercingShadow(scope, (_a = locator.value) != null ? _a : "");
        break;
      case "id": {
        const element = queryCssPiercingShadow(scope, `#${CSS.escape((_b = locator.value) != null ? _b : "")}`)[0];
        elements = element ? [element] : [];
        break;
      }
      case "name":
        elements = queryCssPiercingShadow(scope, `[name=${JSON.stringify((_c = locator.value) != null ? _c : "")}]`);
        break;
      case "test_id":
        elements = queryByAttribute(scope, (_d = locator.attribute) != null ? _d : "data-testid", (_e = locator.value) != null ? _e : "", locator.exact, locator.regex);
        break;
      case "placeholder":
        elements = queryByAttribute(scope, "placeholder", (_f = locator.value) != null ? _f : "", locator.exact, locator.regex);
        break;
      case "alt_text":
        elements = queryByAttribute(scope, "alt", (_g = locator.value) != null ? _g : "", locator.exact, locator.regex);
        break;
      case "title":
        elements = queryByAttribute(scope, "title", (_h = locator.value) != null ? _h : "", locator.exact, locator.regex);
        break;
      case "autocomplete":
        elements = queryCssPiercingShadow(scope, `[autocomplete=${JSON.stringify((_i = locator.value) != null ? _i : "")}]`);
        break;
      case "text": {
        const cache = /* @__PURE__ */ new Map();
        const { matcher, kind } = createLocatorTextMatcher((_j = locator.value) != null ? _j : "", locator.exact, locator.regex);
        elements = [];
        let lastDidNotMatchSelf = null;
        for (const element of queryAllPiercingShadow(scope)) {
          if (kind === "lax" && lastDidNotMatchSelf && lastDidNotMatchSelf.contains(element))
            continue;
          const matches = elementMatchesText(cache, element, matcher);
          if (matches === "none")
            lastDidNotMatchSelf = element;
          if (matches === "self")
            elements.push(element);
        }
        break;
      }
      case "label": {
        const cache = /* @__PURE__ */ new Map();
        const { matcher } = createLocatorTextMatcher((_k = locator.value) != null ? _k : "", locator.exact, locator.regex);
        elements = queryDescendantsPiercingShadow(scope).filter(
          (element) => getElementLabels(cache, element).some((label) => matcher(label))
        );
        break;
      }
      case "role": {
        const selector = serializeRoleSelector(locator);
        elements = createRoleEngine(true).queryAll(scope, selector);
        break;
      }
      case "xpath": {
        const result = document.evaluate(
          (_l = locator.value) != null ? _l : "",
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
    if (locator.filter) {
      const filter = locator.filter;
      elements = elements.filter((element) => {
        if (filter.visible !== void 0 && isElementVisible(element) !== filter.visible)
          return false;
        if (filter.has_text !== void 0 && !matchesLocatorText(element, filter.has_text))
          return false;
        if (filter.has_not_text !== void 0 && matchesLocatorText(element, filter.has_not_text))
          return false;
        if (filter.has && !this.resolveAll(filter.has, element).length)
          return false;
        if (filter.has_not && this.resolveAll(filter.has_not, element).length)
          return false;
        return true;
      });
    }
    if (locator.locator) {
      const chained = /* @__PURE__ */ new Set();
      for (const element of elements) {
        for (const inner of this.resolveAll(locator.locator, element))
          chained.add(inner);
      }
      elements = sortInDOMOrder(chained);
    }
    if (locator.and) {
      const other = new Set(this.resolveAll(locator.and, scope));
      elements = elements.filter((element) => other.has(element));
    }
    if (locator.or)
      elements = sortInDOMOrder(/* @__PURE__ */ new Set([...elements, ...this.resolveAll(locator.or, scope)]));
    return applyLocatorIndex(elements, locator);
  }
  extractLinks(locator, limit) {
    const roots = this.resolveAll(locator);
    const links = [];
    let total = 0;
    for (const root of roots) {
      const anchors = root.matches("a[href]") ? [root] : Array.from(root.querySelectorAll("a[href]"));
      total += anchors.length;
      for (const anchor of anchors) {
        if (links.length < limit)
          links.push({ url: anchor.href, text: (anchor.innerText || "").trim().substring(0, 200) });
      }
    }
    return { ok: true, links, total };
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
  expectHitTarget(element, point) {
    return this.hitTargetController.expectHitTarget(point, element);
  }
  installHitTargetInterceptor(element, action, point, blockAllEvents = false) {
    return this.hitTargetController.install(element, action, point, blockAllEvents);
  }
  takeHitTargetInterceptor(id) {
    return this.hitTargetController.take(id);
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
  getImplicitRole(element) {
    var _a;
    this.ensureConnected(element);
    return (_a = getImplicitAriaRole(element)) != null ? _a : "generic";
  }
  computeRole(element) {
    var _a;
    this.ensureConnected(element);
    return (_a = getAriaRole(element)) != null ? _a : "generic";
  }
  getAccessibleName(element, includeHidden = false) {
    this.ensureConnected(element);
    return getElementAccessibleName(element, includeHidden).text;
  }
  getAccessibleDescription(element) {
    this.ensureConnected(element);
    return getElementAccessibleDescription(element, false).text;
  }
  expectationValues(element) {
    var _a, _b, _c, _d;
    this.ensureConnected(element);
    const htmlElement = element;
    const input = element;
    const select = element;
    const rect = element.getBoundingClientRect();
    const view = this.global;
    return {
      attached: element.isConnected,
      visible: isElementVisible(element),
      enabled: !getAriaDisabled(element),
      editable: !getReadonly(element) && (element.matches("input, textarea, select") || htmlElement.isContentEditable),
      checked: input.checked === true,
      focused: element === this.global.document.activeElement,
      empty: element.children.length === 0 && !normalizeWhiteSpace((_a = element.textContent) != null ? _a : "").length && !("value" in input && input.value),
      inViewport: rect.width > 0 && rect.height > 0 && rect.bottom > 0 && rect.right > 0 && rect.top < view.innerHeight && rect.left < view.innerWidth,
      text: (_c = (_b = htmlElement.innerText) != null ? _b : element.textContent) != null ? _c : "",
      value: "value" in input ? input.value : null,
      values: select.multiple ? Array.from(select.selectedOptions).map((option) => option.value) : null,
      class: element.classList.toString(),
      id: element.id,
      role: (_d = getAriaRole(element)) != null ? _d : "",
      accessibleName: getElementAccessibleName(element, false).text,
      accessibleDescription: getElementAccessibleDescription(element, false).text
    };
  }
  ariaSnapshot(element, options) {
    var _a;
    const root = (_a = element != null ? element : this.global.document.body) != null ? _a : this.global.document.documentElement;
    this.ensureConnected(root);
    const tree = generateAriaTree(root, options);
    const { json } = renderAriaTreeAsJSON(tree, options);
    const nodes = [];
    if (options.boxes) {
      const visit = (node) => {
        var _a2;
        if (typeof node === "string")
          return;
        if (node.box)
          nodes.push({ role: node.role, name: node.name, ref: node.ref, box: node.box });
        for (const child of (_a2 = node.children) != null ? _a2 : [])
          visit(child);
      };
      for (const node of json)
        visit(node);
    }
    return { yaml: renderAriaSnapshotAsYaml(json), nodes };
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
var selectorEvaluator = new SelectorEvaluatorImpl();
var selectorEngines = /* @__PURE__ */ new Map([
  ["css", {
    queryAll(root, selector) {
      return selectorEvaluator.query({ scope: root, pierceShadow: true }, selector);
    }
  }],
  ["xpath", XPathEngine]
]);
function createInternalTextMatcher(selector) {
  if (selector.startsWith("/") && selector.lastIndexOf("/") > 0) {
    const lastSlash = selector.lastIndexOf("/");
    const expression = new RegExp(selector.slice(1, lastSlash), selector.slice(lastSlash + 1));
    return (text) => matchesRegExp(expression, text.full);
  }
  const exact = selector.endsWith("s");
  const value = normalizeWhiteSpace(JSON.parse(selector.slice(0, -1)));
  if (exact)
    return (text) => text.normalized === value;
  const folded = value.toLowerCase();
  return (text) => text.normalized.toLowerCase().includes(folded);
}
function createInternalAttributeEngine(testId) {
  return {
    queryAll(root, selector) {
      const parsed = parseAttributeSelector(selector, true);
      if (parsed.name || parsed.attributes.length !== 1)
        throw new Error(`Malformed ${testId ? "test id" : "attribute"} selector: ${selector}`);
      const attribute = parsed.attributes[0];
      const names = testId ? attribute.name.split(",") : [attribute.name];
      return queryDescendantsPiercingShadow(root).filter(
        (element) => names.some((name) => {
          const value = element.getAttribute(name);
          return value !== null && matchesAttributePart(value, attribute);
        })
      );
    }
  };
}
selectorEngines.set("internal:role", createRoleEngine(true));
selectorEngines.set("internal:text", {
  queryAll(root, selector) {
    const matcher = createInternalTextMatcher(selector);
    return queryAllPiercingShadow(root).filter(
      (element) => elementMatchesText(selectorEvaluator._cacheText, element, matcher) === "self"
    );
  }
});
selectorEngines.set("internal:has-text", {
  queryAll(root, selector) {
    if (!(root instanceof Element))
      return [];
    return createInternalTextMatcher(selector)(elementText(selectorEvaluator._cacheText, root)) ? [root] : [];
  }
});
selectorEngines.set("internal:label", {
  queryAll(root, selector) {
    const matcher = createInternalTextMatcher(selector);
    return queryDescendantsPiercingShadow(root).filter(
      (element) => getElementLabels(selectorEvaluator._cacheText, element).some(matcher)
    );
  }
});
selectorEngines.set("internal:attr", createInternalAttributeEngine(false));
selectorEngines.set("internal:testid", createInternalAttributeEngine(true));
function querySelectorPart(part, root) {
  const engine = selectorEngines.get(part.name);
  if (!engine)
    throw new Error(`Unknown selector engine "${part.name}"`);
  return engine.queryAll(root, part.body);
}
function queryLayoutSelector(elements, part, originalRoot) {
  const body = part.body;
  const inner = queryParsedSelector(body.parsed, originalRoot);
  const matches = [];
  for (const element of elements) {
    const score = layoutSelectorScore(part.name, element, inner, body.distance);
    if (score !== void 0)
      matches.push({ element, score });
  }
  matches.sort((left, right) => left.score - right.score);
  return new Set(matches.map((match) => match.element));
}
function queryParsedSelector(selector, root) {
  if (selector.capture !== void 0) {
    const captured = { parts: selector.parts.slice(0, selector.capture + 1) };
    if (selector.capture < selector.parts.length - 1) {
      const parsed = { parts: selector.parts.slice(selector.capture + 1) };
      captured.parts.push({
        name: "internal:has",
        body: { parsed },
        source: stringifySelector(parsed)
      });
    }
    return queryParsedSelector(captured, root);
  }
  selectorEvaluator.begin();
  try {
    let roots = /* @__PURE__ */ new Set([root]);
    for (const part of selector.parts) {
      if (part.name === "nth") {
        const index = Number(part.body);
        roots = Number.isInteger(index) && index >= 0 && index < roots.size ? /* @__PURE__ */ new Set([[...roots][index]]) : /* @__PURE__ */ new Set();
      } else if (part.name === "internal:has") {
        roots = new Set([...roots].filter(
          (element) => queryParsedSelector(part.body.parsed, element).length > 0
        ));
      } else if (part.name === "internal:and") {
        const andElements = queryParsedSelector(part.body.parsed, root);
        roots = new Set(andElements.filter((element) => roots.has(element)));
      } else if (part.name === "internal:or") {
        const orElements = queryParsedSelector(part.body.parsed, root);
        roots = new Set(sortInDOMOrder(/* @__PURE__ */ new Set([...roots, ...orElements])));
      } else if (kLayoutSelectorNames.includes(part.name)) {
        roots = queryLayoutSelector(roots, part, root);
      } else {
        const next = /* @__PURE__ */ new Set();
        for (const queryRoot of roots) {
          for (const element of querySelectorPart(part, queryRoot))
            next.add(element);
        }
        roots = next;
      }
    }
    return [...roots];
  } finally {
    selectorEvaluator.end();
  }
}
Object.defineProperty(RefactInjected.prototype, "querySelectorAll", {
  value(selectorChain, scope) {
    return queryParsedSelector(parseSelector(selectorChain), scope != null ? scope : globalThis.document);
  }
});
var _lastAriaSnapshotForQuery = /* @__PURE__ */ new WeakMap();
Object.defineProperty(RefactInjected.prototype, "ariaSnapshot", {
  value(element, options) {
    var _a;
    const root = (_a = element != null ? element : globalThis.document.body) != null ? _a : globalThis.document.documentElement;
    if (!root.isConnected)
      throw new Error("Element is not connected to a document");
    const tree = generateAriaTree(root, options);
    const { json } = renderAriaTreeAsJSON(tree, options);
    _lastAriaSnapshotForQuery.set(this, tree.info);
    const nodes = [];
    if (options.boxes || options.refs) {
      const visit = (node) => {
        var _a2;
        if (typeof node === "string")
          return;
        if (node.ref || node.box)
          nodes.push({ role: node.role, name: node.name, ref: node.ref, box: node.box });
        for (const child of (_a2 = node.children) != null ? _a2 : [])
          visit(child);
      };
      for (const node of json)
        visit(node);
    }
    return { yaml: renderAriaSnapshotAsYaml(json), nodes };
  }
});
Object.defineProperty(RefactInjected.prototype, "resolveAriaRef", {
  value(reference) {
    var _a;
    const result = (_a = _lastAriaSnapshotForQuery.get(this)) == null ? void 0 : _a.get(reference);
    if (!result)
      throw new Error(`REF_UNKNOWN: ref ${reference} is unknown; take a fresh snapshot`);
    if (!result.element.isConnected)
      throw new Error(`REF_DETACHED: ref ${reference} is detached; take a fresh snapshot`);
    return result.element;
  }
});
Object.defineProperty(RefactInjected.prototype, "generateLocator", {
  value(element, options) {
    if (!element.isConnected)
      throw new Error("Element is not attached to the DOM");
    const runtime = {
      _evaluator: selectorEvaluator,
      parseSelector,
      querySelector: (selector, root) => queryParsedSelector(selector, root)[0],
      querySelectorAll: (selector, root) => queryParsedSelector(selector, root)
    };
    return generateSelector(runtime, element, options).selector;
  }
});
