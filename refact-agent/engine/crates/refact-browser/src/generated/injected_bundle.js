// @refact-injected-hash dc1d3e4f02ac6061b0e903bf0a8760f8a44ae343ed961bfa6b442ca50fbb5eb8

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

// src/vendor/isomorphic/stringUtils.ts
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
var cacheAccessibleName;
var cacheAccessibleNameHidden;
var cacheAccessibleDescription;
var cacheAccessibleDescriptionHidden;
var cacheIsHidden;
var cachePseudoContent;
var cachePseudoContentBefore;
var cachePseudoContentAfter;
var cacheAriaRole;
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

// src/refactInjected.ts
var injectedInstanceName = "__refact_injected__";
var bindingName = "__refact_binding";
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
