import { AppBridgeError, isRecord, type AppCaptureParams } from "./appSdk";

export const APP_CAPTURE_METHOD = "dom-raster";
export const APP_CAPTURE_TRUST = "app-reported";
export const APP_CAPTURE_DEFAULTS = {
  maxWidth: 1600,
  maxHeight: 1600,
  maxBytes: 6_000_000,
  maxPixels: 2_560_000,
  maxElements: 4000,
  maxSvgChars: 8_000_000,
} as const;

export type AppCaptureStatus = "ok" | "unsupported" | "error";
export type AppCaptureFormat = "image/png";

export type AppCaptureResult = {
  status: AppCaptureStatus;
  method: typeof APP_CAPTURE_METHOD;
  trust: typeof APP_CAPTURE_TRUST;
  format: AppCaptureFormat | null;
  dataUrl?: string;
  width: number;
  height: number;
  viewportWidth: number;
  viewportHeight: number;
  scale: number;
  bytes: number;
  partial: boolean;
  unsupported: string[];
  masks: number;
  elements: number;
  reason?: string;
};

const clampInt = (v: unknown, lo: number, hi: number, dflt: number) =>
  typeof v === "number" && Number.isFinite(v)
    ? Math.min(hi, Math.max(lo, Math.floor(v)))
    : dflt;

export function normalizeCaptureParams(
  params: AppCaptureParams = {},
): Required<AppCaptureParams> {
  const d = APP_CAPTURE_DEFAULTS;
  return {
    maxWidth: clampInt(params.maxWidth, 16, 4096, d.maxWidth),
    maxHeight: clampInt(params.maxHeight, 16, 4096, d.maxHeight),
    maxBytes: clampInt(params.maxBytes, 1024, 10_000_000, d.maxBytes),
    redactSelectors: Array.isArray(params.redactSelectors)
      ? params.redactSelectors
          .filter((s): s is string => typeof s === "string" && s.length <= 256)
          .slice(0, 32)
      : [],
  };
}

function base64Bytes(dataUrl: string): number {
  const comma = dataUrl.indexOf(",");
  if (comma < 0) return -1;
  const header = dataUrl.slice(0, comma);
  const body = dataUrl.slice(comma + 1);
  if (!header.endsWith(";base64")) return -1;
  if (!/^[A-Za-z0-9+/]*={0,2}$/.test(body) || body.length % 4 !== 0) return -1;
  const pad = body.endsWith("==") ? 2 : body.endsWith("=") ? 1 : 0;
  return (body.length / 4) * 3 - pad;
}

const PNG_SIG = "iVBORw0KGgo";

/* Host-side validation: never trust the app-reported fields blindly. */
export function parseAppCaptureResult(
  value: unknown,
  params: AppCaptureParams = {},
): AppCaptureResult | null {
  if (!isRecord(value)) return null;
  const p = normalizeCaptureParams(params);
  const status = value.status;
  if (status !== "ok" && status !== "unsupported" && status !== "error") {
    return null;
  }
  const num = (k: string, hi: number) => {
    const v = value[k];
    return typeof v === "number" && Number.isFinite(v) && v >= 0 && v <= hi
      ? v
      : null;
  };
  const width = num("width", p.maxWidth);
  const height = num("height", p.maxHeight);
  const vw = num("viewportWidth", 100_000);
  const vh = num("viewportHeight", 100_000);
  const scale = num("scale", 1);
  const masks = num("masks", 100_000);
  const elements = num("elements", 10_000_000);
  if (
    width === null ||
    height === null ||
    vw === null ||
    vh === null ||
    scale === null ||
    masks === null ||
    elements === null
  ) {
    return null;
  }
  const unsupported = Array.isArray(value.unsupported)
    ? value.unsupported
        .filter((s): s is string => typeof s === "string")
        .map((s) => s.slice(0, 64))
        .slice(0, 64)
    : [];
  const reason =
    typeof value.reason === "string" ? value.reason.slice(0, 500) : undefined;
  const base: AppCaptureResult = {
    status,
    method: APP_CAPTURE_METHOD,
    trust: APP_CAPTURE_TRUST,
    format: null,
    width,
    height,
    viewportWidth: vw,
    viewportHeight: vh,
    scale,
    bytes: 0,
    partial: value.partial === true || unsupported.length > 0,
    unsupported,
    masks,
    elements,
  };
  if (reason !== undefined) base.reason = reason;
  if (status !== "ok") return base;
  const dataUrl = value.dataUrl;
  if (typeof dataUrl !== "string") return null;
  if (!dataUrl.startsWith("data:image/png;base64," + PNG_SIG)) return null;
  const format: AppCaptureFormat = "image/png";
  if (value.format !== format) return null;
  const bytes = base64Bytes(dataUrl);
  if (bytes <= 0 || bytes > p.maxBytes) return null;
  if (width < 1 || height < 1) return null;
  return { ...base, format, dataUrl, bytes };
}

export type AppCaptureRequester = {
  request: (
    method: "capture",
    params: AppCaptureParams,
    timeoutMs?: number,
  ) => Promise<unknown>;
};

export async function requestAppCapture(
  bridge: AppCaptureRequester,
  params: AppCaptureParams = {},
  timeoutMs = 20_000,
): Promise<AppCaptureResult> {
  const normalized = normalizeCaptureParams(params);
  const raw = await bridge.request("capture", normalized, timeoutMs);
  const parsed = parseAppCaptureResult(raw, normalized);
  if (!parsed) {
    throw new AppBridgeError(
      "invalid_capture",
      "app returned an invalid capture result",
    );
  }
  return parsed;
}

type RuntimeEnv = { contextTypes: WeakMap<HTMLCanvasElement, string> };

/* Serialized into the sandboxed document; must stay self-contained. */
export async function appCaptureRuntime(
  rawParams: unknown,
  env: RuntimeEnv,
): Promise<unknown> {
  const LIMITS = {
    maxPixels: 2_560_000,
    maxElements: 4000,
    maxSvgChars: 8_000_000,
    maxFontChars: 4_000_000,
    maxImagePixels: 4_000_000,
    maxInlinePixels: 16_000_000,
    decodeTimeoutMs: 5000,
    fontsTimeoutMs: 2000,
  };
  let inlinePixels = 0;
  const reservePixels = (w: number, h: number): boolean => {
    const px = w * h;
    if (!Number.isFinite(px) || px <= 0 || px > LIMITS.maxImagePixels)
      return false;
    if (inlinePixels + px > LIMITS.maxInlinePixels) return false;
    inlinePixels += px;
    return true;
  };
  const P: Record<string, unknown> =
    typeof rawParams === "object" && rawParams !== null
      ? (rawParams as Record<string, unknown>)
      : {};
  const int = (v: unknown, lo: number, hi: number, d: number) =>
    typeof v === "number" && Number.isFinite(v)
      ? Math.min(hi, Math.max(lo, Math.floor(v)))
      : d;
  const maxW = int(P.maxWidth, 16, 4096, 1600);
  const maxH = int(P.maxHeight, 16, 4096, 1600);
  const maxBytes = int(P.maxBytes, 1024, 10_000_000, 6_000_000);
  const redactSelectors = Array.isArray(P.redactSelectors)
    ? (P.redactSelectors as unknown[])
        .filter((s): s is string => typeof s === "string" && s.length <= 256)
        .slice(0, 32)
    : [];

  const vw = Math.max(0, Math.floor(window.innerWidth));
  const vh = Math.max(0, Math.floor(window.innerHeight));
  const unsupported = new Set<string>();
  let masks = 0;
  let elements = 0;
  let truncated = false;
  const result = (
    status: "ok" | "unsupported" | "error",
    extra: Record<string, unknown>,
  ) => ({
    status,
    method: "dom-raster",
    trust: "app-reported",
    format: null,
    width: 0,
    height: 0,
    viewportWidth: vw,
    viewportHeight: vh,
    scale: 0,
    bytes: 0,
    partial: truncated || unsupported.size > 0,
    unsupported: Array.from(unsupported),
    masks,
    elements,
    ...extra,
  });

  if (vw === 0 || vh === 0 || !document.querySelector("body")) {
    return result("unsupported", {
      reason: "document has no painted viewport",
    });
  }
  if (document.visibilityState === "hidden") unsupported.add("document-hidden");

  const SVG_NS = "http://www.w3.org/2000/svg";
  const XHTML_NS = "http://www.w3.org/1999/xhtml";

  const defaultsHost = document.createElement("div");
  defaultsHost.setAttribute("aria-hidden", "true");
  defaultsHost.style.cssText =
    "position:fixed;left:-99999px;top:0;width:0;height:0;overflow:hidden;pointer-events:none;";
  const shadow = defaultsHost.attachShadow({ mode: "closed" });
  const defaultsRoot = document.createElement("div");
  defaultsRoot.style.cssText = "all:initial";
  shadow.appendChild(defaultsRoot);
  document.documentElement.appendChild(defaultsHost);
  const defaultsCache = new Map<string, Map<string, string>>();
  const defaultsFor = (el: Element): Map<string, string> => {
    const key = el.namespaceURI + "|" + el.localName;
    let m = defaultsCache.get(key);
    if (m) return m;
    m = new Map();
    const probe = document.createElementNS(
      el.namespaceURI ?? XHTML_NS,
      el.localName,
    );
    defaultsRoot.appendChild(probe);
    const cs = getComputedStyle(probe);
    for (let i = 0; i < cs.length; i++) {
      const name = cs.item(i);
      m.set(name, cs.getPropertyValue(name));
    }
    probe.remove();
    defaultsCache.set(key, m);
    return m;
  };

  const SKIP_PROPS =
    /^(transition|animation|will-change|cursor|caret-color|pointer-events|user-select|-webkit-user-select|outline-offset)$/;

  const isRedacted = (el: Element): boolean => {
    if (el.closest("[data-refact-redact],[data-sensitive]")) return true;
    if (el instanceof HTMLInputElement && el.type === "password") return true;
    const ac = el.getAttribute("autocomplete") ?? "";
    if (/^(cc-|current-password|new-password|one-time-code)/.test(ac))
      return true;
    for (const sel of redactSelectors) {
      try {
        if (el.matches(sel)) return true;
      } catch {
        /* invalid selector ignored */
      }
    }
    return false;
  };

  const placeholder = (el: Element, label: string, bg: string): HTMLElement => {
    const r = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    const ph = document.createElementNS(XHTML_NS, "div");
    ph.setAttribute(
      "style",
      [
        "box-sizing:border-box",
        "display:" + (cs.display === "inline" ? "inline-block" : cs.display),
        "width:" + String(r.width) + "px",
        "height:" + String(r.height) + "px",
        "margin:" + cs.margin,
        "position:" + cs.position,
        "top:" + cs.top,
        "left:" + cs.left,
        "right:" + cs.right,
        "bottom:" + cs.bottom,
        "background:" + bg,
        "color:#fff",
        "font:11px sans-serif",
        "overflow:hidden",
        "border-radius:" + cs.borderRadius,
      ].join(";"),
    );
    ph.textContent = label;
    return ph;
  };

  const imageToDataUrl = (img: HTMLImageElement): string | null => {
    const src = img.currentSrc || img.src;
    if (src.startsWith("data:") && src.length <= LIMITS.maxSvgChars / 4)
      return src;
    if (!img.complete || img.naturalWidth === 0) return null;
    if (!reservePixels(img.naturalWidth, img.naturalHeight)) {
      unsupported.add("image-limit");
      return null;
    }
    try {
      const c = document.createElement("canvas");
      c.width = img.naturalWidth;
      c.height = img.naturalHeight;
      const ctx = c.getContext("2d");
      if (!ctx) return null;
      ctx.drawImage(img, 0, 0);
      return c.toDataURL("image/png");
    } catch {
      return null;
    }
  };

  const copyStyle = (
    src: Element,
    dst: Element,
    parentCs: CSSStyleDeclaration | null,
  ) => {
    const cs = getComputedStyle(src);
    const defs = defaultsFor(src);
    const parts: string[] = [];
    for (let i = 0; i < cs.length; i++) {
      const name = cs.item(i);
      if (SKIP_PROPS.test(name)) continue;
      let value = cs.getPropertyValue(name);
      if (
        value === defs.get(name) &&
        (!parentCs || value === parentCs.getPropertyValue(name))
      ) {
        continue;
      }
      if (value.includes("url(")) {
        if (/url\(\s*["']?(?!data:)/.test(value)) {
          unsupported.add("external-css-url");
          value = "none";
        }
      }
      parts.push(name + ":" + value);
    }
    for (const pseudo of ["::before", "::after"]) {
      const content = getComputedStyle(src, pseudo).content;
      if (content && content !== "none" && content !== "normal") {
        unsupported.add("pseudo-elements");
      }
    }
    dst.setAttribute("style", parts.join(";"));
    return cs;
  };

  const shiftForScroll = (
    container: Element,
    clone: Element,
    x: number,
    y: number,
  ) => {
    if (x === 0 && y === 0) return;
    for (const child of Array.from(clone.childNodes)) {
      if (child.nodeType === Node.TEXT_NODE) {
        const span = document.createElementNS(XHTML_NS, "span");
        span.setAttribute(
          "style",
          "position:relative;left:" +
            String(-x) +
            "px;top:" +
            String(-y) +
            "px",
        );
        clone.replaceChild(span, child);
        span.appendChild(child);
      } else if (child instanceof Element) {
        const style = child.getAttribute("style") ?? "";
        const pos = /(^|;)position:(\w+)/.exec(style)?.[2] ?? "static";
        if (pos === "fixed") continue;
        const tr = /(^|;)translate:([^;]+)/.exec(style)?.[2];
        const [tx, ty] =
          tr && tr !== "none" ? tr.trim().split(/\s+/) : ["0px", "0px"];
        child.setAttribute(
          "style",
          style.replace(/(^|;)translate:[^;]+/, "") +
            ";translate:calc(" +
            (tx || "0px") +
            " - " +
            String(x) +
            "px) calc(" +
            (ty || "0px") +
            " - " +
            String(y) +
            "px)",
        );
      }
    }
    void container;
  };

  const cloneNode = (
    node: Node,
    parentCs: CSSStyleDeclaration | null,
  ): Node | null => {
    if (node.nodeType === Node.TEXT_NODE)
      return document.createTextNode(node.nodeValue ?? "");
    if (!(node instanceof Element)) return null;
    if (node === defaultsHost) return null;
    const tag = node.localName;
    if (
      tag === "script" ||
      tag === "noscript" ||
      tag === "template" ||
      tag === "link" ||
      tag === "meta"
    ) {
      return null;
    }
    if (tag === "style") return null;
    elements += 1;
    if (elements > LIMITS.maxElements) {
      truncated = true;
      unsupported.add("element-limit");
      return null;
    }
    const cs = getComputedStyle(node);
    if (cs.display === "none") return null;

    if (node !== document.body && isRedacted(node)) {
      masks += 1;
      return placeholder(node, "redacted", "#555");
    }
    if (
      node instanceof HTMLIFrameElement ||
      tag === "object" ||
      tag === "embed"
    ) {
      unsupported.add("embedded-frame");
      return placeholder(node, "embedded frame not captured", "#844");
    }
    if (node instanceof HTMLVideoElement || tag === "audio") {
      unsupported.add("media");
      return placeholder(node, "media not captured", "#844");
    }
    if (node instanceof HTMLCanvasElement) {
      const kind = env.contextTypes.get(node) ?? "none";
      if (kind !== "2d" && kind !== "bitmaprenderer" && kind !== "none") {
        unsupported.add("canvas-" + kind);
        return placeholder(node, kind + " canvas not captured", "#844");
      }
      if (!reservePixels(node.width, node.height)) {
        unsupported.add("canvas-limit");
        return placeholder(node, "canvas too large", "#844");
      }
      let url: string | null = null;
      try {
        url = node.toDataURL("image/png");
      } catch {
        unsupported.add("tainted-canvas");
        return placeholder(node, "tainted canvas", "#844");
      }
      const img = document.createElementNS(XHTML_NS, "img");
      copyStyle(node, img, parentCs);
      img.setAttribute("src", url);
      return img;
    }

    const isSvg = node.namespaceURI === SVG_NS;
    const clone = isSvg
      ? document.createElementNS(SVG_NS, tag)
      : document.createElementNS(XHTML_NS, tag);
    for (const attr of Array.from(node.attributes)) {
      const n = attr.name.toLowerCase();
      if (
        n.startsWith("on") ||
        n === "style" ||
        n === "srcset" ||
        n === "src" ||
        n === "href" ||
        n === "xlink:href"
      )
        continue;
      if (!isSvg && (n === "class" || n === "id")) continue;
      try {
        clone.setAttribute(attr.name, attr.value);
      } catch {
        /* ignore invalid attribute */
      }
    }
    const nodeCs = copyStyle(node, clone, parentCs);

    if (node instanceof HTMLImageElement) {
      const url = imageToDataUrl(node);
      if (url) clone.setAttribute("src", url);
      else unsupported.add("image-unavailable");
      return clone;
    }
    if (isSvg && (tag === "image" || tag === "use")) {
      const href =
        node.getAttribute("href") ?? node.getAttribute("xlink:href") ?? "";
      if (href.startsWith("data:") || href.startsWith("#"))
        clone.setAttribute("href", href);
      else unsupported.add("svg-external-ref");
    }
    if (node instanceof HTMLInputElement) {
      const t = node.type;
      if (t === "checkbox" || t === "radio") {
        if (node.checked) clone.setAttribute("checked", "");
        else clone.removeAttribute("checked");
        if (node.indeterminate) unsupported.add("indeterminate-checkbox");
      } else if (t === "file") {
        clone.removeAttribute("value");
      } else {
        clone.setAttribute("value", node.value);
      }
      return clone;
    }
    if (node instanceof HTMLTextAreaElement) {
      clone.textContent = node.value;
      if (node.scrollTop || node.scrollLeft) unsupported.add("textarea-scroll");
      return clone;
    }
    if (node instanceof HTMLSelectElement) {
      for (const opt of Array.from(node.options)) {
        const o = document.createElementNS(XHTML_NS, "option");
        o.textContent = opt.text;
        o.setAttribute("value", opt.value);
        if (opt.selected) o.setAttribute("selected", "");
        clone.appendChild(o);
      }
      return clone;
    }
    if (node instanceof HTMLDetailsElement) {
      if (node.open) clone.setAttribute("open", "");
      else clone.removeAttribute("open");
    }
    if (node.shadowRoot) unsupported.add("shadow-dom");

    for (const child of Array.from(node.childNodes)) {
      const c = cloneNode(child, nodeCs);
      if (c) clone.appendChild(c);
    }
    const sx = node === document.body ? 0 : node.scrollLeft;
    const sy = node === document.body ? 0 : node.scrollTop;
    shiftForScroll(node, clone, sx, sy);
    return clone;
  };

  const collectFontCss = async (): Promise<string> => {
    const out: string[] = [];
    let total = 0;
    for (const sheet of Array.from(document.styleSheets)) {
      let rules: CSSRuleList;
      try {
        rules = sheet.cssRules;
      } catch {
        unsupported.add("cross-origin-stylesheet");
        continue;
      }
      for (const rule of Array.from(rules)) {
        if (rule instanceof CSSFontFaceRule) {
          const text = rule.cssText;
          if (/url\(\s*["']?(?!data:)/.test(text)) {
            unsupported.add("external-font");
            continue;
          }
          total += text.length;
          if (total > LIMITS.maxFontChars) {
            unsupported.add("font-limit");
            return out.join("\n");
          }
          out.push(text);
        }
      }
    }
    await Promise.resolve();
    return out.join("\n");
  };

  const toBase64 = (s: string): string => {
    const bytes = new TextEncoder().encode(s);
    let bin = "";
    const CH = 0x8000;
    for (let i = 0; i < bytes.length; i += CH) {
      bin += String.fromCharCode.apply(
        null,
        Array.from(bytes.subarray(i, i + CH)),
      );
    }
    return btoa(bin);
  };

  let svgText: string;
  try {
    if (document.fonts.status !== "loaded") {
      const fontsReady = await Promise.race([
        document.fonts.ready.then(() => true),
        new Promise<boolean>((r) =>
          setTimeout(() => r(false), LIMITS.fontsTimeoutMs),
        ),
      ]);
      if (!fontsReady) unsupported.add("fonts-pending");
    }
    if (document.body.closest("[data-refact-redact],[data-sensitive]")) {
      masks += 1;
      return result("unsupported", {
        reason: "entire document is marked sensitive",
      });
    }
    for (const sel of redactSelectors) {
      try {
        if (
          document.body.matches(sel) ||
          document.documentElement.matches(sel)
        ) {
          masks += 1;
          return result("unsupported", {
            reason: "entire document is redacted",
          });
        }
      } catch {
        /* invalid selector ignored */
      }
    }
    const docCs = getComputedStyle(document.documentElement);
    const bodyClone = cloneNode(document.body, docCs) as Element | null;
    if (!bodyClone) {
      return result("unsupported", { reason: "body is not rendered" });
    }
    const htmlEl = document.createElementNS(XHTML_NS, "html");
    copyStyle(document.documentElement, htmlEl, null);
    const sx = window.scrollX;
    const sy = window.scrollY;
    htmlEl.setAttribute(
      "style",
      (htmlEl.getAttribute("style") ?? "") +
        ";width:" +
        String(vw) +
        "px;height:" +
        String(vh) +
        "px;overflow:hidden",
    );
    const inner = document.createElementNS(XHTML_NS, "div");
    inner.setAttribute(
      "style",
      "transform:translate(" + String(-sx) + "px," + String(-sy) + "px)",
    );
    inner.appendChild(bodyClone);
    htmlEl.appendChild(inner);
    const fontCss = await collectFontCss();
    if (fontCss) {
      const st = document.createElementNS(XHTML_NS, "style");
      st.textContent = fontCss;
      htmlEl.insertBefore(st, htmlEl.firstChild);
    }
    const svg = document.createElementNS(SVG_NS, "svg");
    svg.setAttribute("xmlns", SVG_NS);
    svg.setAttribute("width", String(vw));
    svg.setAttribute("height", String(vh));
    svg.setAttribute("viewBox", "0 0 " + String(vw) + " " + String(vh));
    const fo = document.createElementNS(SVG_NS, "foreignObject");
    fo.setAttribute("x", "0");
    fo.setAttribute("y", "0");
    fo.setAttribute("width", "100%");
    fo.setAttribute("height", "100%");
    fo.appendChild(htmlEl);
    svg.appendChild(fo);
    svgText = new XMLSerializer().serializeToString(svg);
  } catch (err) {
    return result("error", {
      reason:
        "clone failed: " + String(err instanceof Error ? err.message : err),
    });
  } finally {
    defaultsHost.remove();
  }
  if (svgText.length > LIMITS.maxSvgChars) {
    return result("unsupported", {
      reason: "document snapshot exceeds size limit",
    });
  }

  const svgUrl = "data:image/svg+xml;base64," + toBase64(svgText);
  let scale = Math.min(1, maxW / vw, maxH / vh);
  if (vw * vh * scale * scale > LIMITS.maxPixels) {
    scale = Math.sqrt(LIMITS.maxPixels / (vw * vh));
  }
  const outW = Math.max(1, Math.floor(vw * scale));
  const outH = Math.max(1, Math.floor(vh * scale));

  const svgFallback = (reason: string) => {
    unsupported.add("png-raster-blocked");
    return result("unsupported", { reason });
  };

  const img = new Image();
  try {
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(
        () => reject(new Error("svg snapshot decode timed out")),
        LIMITS.decodeTimeoutMs,
      );
      img.onload = () => {
        clearTimeout(timer);
        resolve();
      };
      img.onerror = () => {
        clearTimeout(timer);
        reject(new Error("svg snapshot failed to decode"));
      };
      img.src = svgUrl;
    });
  } catch (err) {
    return svgFallback(String(err instanceof Error ? err.message : err));
  }
  const canvas = document.createElement("canvas");
  canvas.width = outW;
  canvas.height = outH;
  const ctx = canvas.getContext("2d");
  if (!ctx) return svgFallback("2d context unavailable");
  let png: string;
  try {
    ctx.drawImage(img, 0, 0, outW, outH);
    png = canvas.toDataURL("image/png");
  } catch (err) {
    return svgFallback(
      "canvas export blocked: " +
        String(err instanceof Error ? err.name + " " + err.message : err),
    );
  }
  if (!png.startsWith("data:image/png;base64,"))
    return svgFallback("png encoder unavailable");
  const bytes = Math.floor(
    ((png.length - "data:image/png;base64,".length) * 3) / 4,
  );
  if (bytes > maxBytes) {
    return result("unsupported", { reason: "png exceeds byte limit", bytes });
  }
  return result("ok", {
    format: "image/png",
    dataUrl: png,
    width: outW,
    height: outH,
    scale,
    bytes,
  });
}
