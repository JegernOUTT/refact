// Pure helpers shared by the Markdown special-fence renderers (mermaid, svg,
// html). Kept outside the component files so they can be unit-tested and so
// component modules only export components (react-refresh requirement).

export const DIAGRAM_LANGUAGES: ReadonlySet<string> = new Set([
  "mermaid",
  "svg",
]);
export const ARTIFACT_LANGUAGES: ReadonlySet<string> = new Set(["html"]);

export const STREAMING_SAFE_FENCE_LANGUAGE = "text";
export const STREAMING_SPECIAL_FENCE_LANGUAGES: ReadonlySet<string> = new Set([
  ...DIAGRAM_LANGUAGES,
  ...ARTIFACT_LANGUAGES,
]);

export const MAX_IFRAME_HEIGHT = 800;
export const DEFAULT_IFRAME_HEIGHT = 200;
export const RESIZE_DEBOUNCE_MS = 50;
export const MIN_HEIGHT_DELTA = 5;

const PAN_VISIBLE_MARGIN = 48;

// Keeps at least PAN_VISIBLE_MARGIN px of the diagram inside the canvas so it
// can never be dragged fully out of view.
export function clampPan(
  pan: number,
  canvasSize: number,
  contentSize: number,
): number {
  const margin = Math.min(PAN_VISIBLE_MARGIN, canvasSize);
  return Math.min(canvasSize - margin, Math.max(margin - contentSize, pan));
}

export type SvgMeta = { viewBox: string; width: number; height: number };

export function parseSvgMeta(svgStr: string): SvgMeta | null {
  const parser = new DOMParser();
  const doc = parser.parseFromString(svgStr, "image/svg+xml");
  const svg = doc.querySelector("svg");
  if (!svg) return null;

  const vbAttr = svg.getAttribute("viewBox");
  const widthAttr = svg.getAttribute("width") ?? "";
  const heightAttr = svg.getAttribute("height") ?? "";

  // Parse the viewBox attribute textually instead of via svg.viewBox.baseVal:
  // the SVGAnimatedRect API is not implemented in all DOM environments.
  let vbW = 0;
  let vbH = 0;
  if (vbAttr) {
    const parts = vbAttr
      .trim()
      .split(/[\s,]+/)
      .map(parseFloat);
    if (parts.length === 4 && parts.every((n) => Number.isFinite(n))) {
      vbW = parts[2];
      vbH = parts[3];
    }
  }

  const isAbsW = widthAttr !== "" && !widthAttr.includes("%");
  const isAbsH = heightAttr !== "" && !heightAttr.includes("%");

  const w = isAbsW ? parseFloat(widthAttr) || vbW : vbW;
  const h = isAbsH ? parseFloat(heightAttr) || vbH : vbH;

  const viewBox = vbAttr ?? (w && h ? `0 0 ${w} ${h}` : null);
  if (!viewBox || !w || !h) return null;

  return { viewBox, width: w, height: h };
}

// Rewrites sizing attributes on the root <svg> element only. A regex-based
// approach could strip width/height/style from the first matching child node
// anywhere in the document, mangling the diagram.
export function makeCrispSvg(svgStr: string, vb: string): string {
  if (
    typeof DOMParser === "undefined" ||
    typeof XMLSerializer === "undefined"
  ) {
    return svgStr;
  }
  const doc = new DOMParser().parseFromString(svgStr, "image/svg+xml");
  if (doc.querySelector("parsererror")) return svgStr;
  const svg = doc.documentElement;
  if (svg.tagName.toLowerCase() !== "svg") return svgStr;

  svg.removeAttribute("style");
  svg.setAttribute("viewBox", vb);
  svg.setAttribute("width", "100%");
  svg.setAttribute("height", "100%");
  return new XMLSerializer().serializeToString(svg);
}

// True when the value contains a url(...) reference that is not an internal
// fragment (url(#id)). Covers fill/stroke/filter/mask/clip-path/marker-* and
// inline style values.
function hasExternalUrlRef(value: string): boolean {
  const re = /url\(\s*['"]?\s*([^)'"\s]*)/gi;
  let match: RegExpExecArray | null;
  while ((match = re.exec(value)) !== null) {
    if (!match[1].startsWith("#")) return true;
  }
  return false;
}

// Removes references to external resources from a sanitized SVG. Internal
// fragment references (#id, url(#id)) and inline data images are kept;
// anything else (http(s), protocol-relative, blob, file) is dropped so
// chat-generated SVG cannot trigger network fetches.
export function stripExternalRefs(svgHtml: string): string {
  if (
    typeof DOMParser === "undefined" ||
    typeof XMLSerializer === "undefined"
  ) {
    return svgHtml;
  }
  const doc = new DOMParser().parseFromString(svgHtml, "image/svg+xml");
  if (doc.querySelector("parsererror")) return svgHtml;

  doc.querySelectorAll("*").forEach((el) => {
    for (const attr of ["href", "xlink:href"]) {
      const value = el.getAttribute(attr);
      if (
        value !== null &&
        !value.startsWith("#") &&
        !value.startsWith("data:image/")
      ) {
        el.removeAttribute(attr);
      }
    }

    const attrNames = el.getAttributeNames();
    for (const name of attrNames) {
      if (name === "href" || name === "xlink:href") continue;
      const value = el.getAttribute(name);
      if (
        value !== null &&
        value.includes("url(") &&
        hasExternalUrlRef(value)
      ) {
        el.removeAttribute(name);
      }
    }
  });
  return new XMLSerializer().serializeToString(doc.documentElement);
}

function injectIntoCompleteDocument(
  doc: string,
  stylesHtml: string,
  scriptsHtml: string,
): string {
  let out = doc;

  const headOpen = /<head(\s[^>]*)?>/i.exec(out);
  if (headOpen) {
    const idx = headOpen.index + headOpen[0].length;
    out = out.slice(0, idx) + stylesHtml + out.slice(idx);
  } else {
    const htmlOpen = /<html(\s[^>]*)?>/i.exec(out);
    if (htmlOpen) {
      const idx = htmlOpen.index + htmlOpen[0].length;
      out = out.slice(0, idx) + stylesHtml + out.slice(idx);
    } else {
      out = stylesHtml + out;
    }
  }

  const bodyClose = /<\/body>/i.exec(out);
  if (bodyClose) {
    out =
      out.slice(0, bodyClose.index) + scriptsHtml + out.slice(bodyClose.index);
  } else {
    const htmlClose = /<\/html>/i.exec(out);
    if (htmlClose) {
      out =
        out.slice(0, htmlClose.index) +
        scriptsHtml +
        out.slice(htmlClose.index);
    } else {
      out = out + scriptsHtml;
    }
  }

  return out;
}

// Theme-independent wrapper. The app theme is delivered to the iframe via
// postMessage ('refact-artifact-theme') instead of being baked into srcDoc, so
// appearance flips restyle the preview without reloading it. Injected default
// styles use :where() so any author rule wins, and documents can opt out
// entirely with <html data-artifact-styled>.
//
// The bootstrap script (error/rejection/theme listeners) is injected into
// <head> so it runs before any author code; inline script errors during the
// initial render are therefore always captured. Only the resize reporting
// runs from the end of <body>.
export function wrapArtifactHtml(userCode: string): string {
  const injectedStyles = `<style data-refact-artifact>
:where(html:not([data-artifact-styled])) { color-scheme: light dark; }
html[data-refact-theme="dark"]:not([data-artifact-styled]) { color-scheme: dark; }
html[data-refact-theme="light"]:not([data-artifact-styled]) { color-scheme: light; }
:where(html:not([data-artifact-styled]) body) { margin: 8px; font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; background: Canvas; color: CanvasText; }
</style>`;
  const bootstrapScript = `<script data-refact-artifact-bootstrap>
(function() {
  window.onerror = function(msg, src, line, col) {
    window.parent.postMessage({
      type: 'refact-artifact-error',
      message: String(msg),
      line: line,
      col: col
    }, '*');
  };
  window.addEventListener('unhandledrejection', function(e) {
    window.parent.postMessage({
      type: 'refact-artifact-error',
      message: 'Unhandled promise rejection: ' + String(e.reason)
    }, '*');
  });
  window.addEventListener('message', function(e) {
    var d = e && e.data;
    if (d && d.type === 'refact-artifact-theme' && (d.theme === 'dark' || d.theme === 'light')) {
      document.documentElement.setAttribute('data-refact-theme', d.theme);
    }
  });
})();
</script>`;
  const resizeScript = `<script data-refact-artifact>
(function() {
  var lastH = 0;
  var timer = null;
  function sendHeight() {
    var h = Math.max(
      document.body.scrollHeight,
      document.body.offsetHeight,
      document.documentElement.scrollHeight,
      document.documentElement.offsetHeight
    );
    if (Math.abs(h - lastH) > ${MIN_HEIGHT_DELTA}) {
      lastH = h;
      window.parent.postMessage({ type: 'refact-artifact-resize', height: h }, '*');
    }
  }
  if (typeof ResizeObserver !== 'undefined') {
    new ResizeObserver(function() {
      clearTimeout(timer);
      timer = setTimeout(sendHeight, ${RESIZE_DEBOUNCE_MS});
    }).observe(document.body);
  }
  window.addEventListener('load', sendHeight);
  setTimeout(sendHeight, 100);
  setTimeout(sendHeight, 500);
})();
</script>`;

  const trimmed = userCode.trim();
  const isCompleteDocument =
    trimmed.toLowerCase().startsWith("<!doctype") ||
    trimmed.toLowerCase().startsWith("<html");

  if (isCompleteDocument) {
    return injectIntoCompleteDocument(
      trimmed,
      injectedStyles + bootstrapScript,
      resizeScript,
    );
  }

  return `<!DOCTYPE html>
<html>
<head><meta charset="utf-8">${injectedStyles}${bootstrapScript}</head>
<body>
${userCode}
${resizeScript}
</body>
</html>`;
}

// Masks incomplete special code fences (mermaid/svg/html) while a message is
// still streaming so heavyweight renderers never see partial fence bodies.
export function maskIncompleteSpecialCodeFences(text: string): string {
  const lines = text.split(/(?<=\n)/);
  let inFence = false;
  let fenceChar = "`";
  let fenceLength = 0;
  let specialFenceLineIndex = -1;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].replace(/\r?\n$/, "");

    if (!inFence) {
      const opening = /^( {0,3})(`{3,}|~{3,})([^`~]*)$/.exec(line);
      if (!opening) continue;

      const info = opening[3].trim();
      const language = info.split(/\s+/)[0]?.toLowerCase() ?? "";
      inFence = true;
      fenceChar = opening[2][0];
      fenceLength = opening[2].length;
      specialFenceLineIndex = STREAMING_SPECIAL_FENCE_LANGUAGES.has(language)
        ? i
        : -1;
      continue;
    }

    const closingPattern = new RegExp(
      `^ {0,3}${fenceChar}{${fenceLength},}\\s*$`,
    );
    if (closingPattern.test(line)) {
      inFence = false;
      specialFenceLineIndex = -1;
    }
  }

  if (!inFence || specialFenceLineIndex < 0) return text;

  lines[specialFenceLineIndex] = lines[specialFenceLineIndex].replace(
    /^( {0,3})(`{3,}|~{3,})([^\r\n]*)(\r?\n?)$/,
    `$1$2${STREAMING_SAFE_FENCE_LANGUAGE}$4`,
  );

  return lines.join("");
}
