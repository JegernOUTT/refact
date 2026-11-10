import { wrapArtifactHtml } from "./renderUtils";

export const REVIEW_ARTIFACT_CSP =
  "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; font-src 'none'; media-src data:; connect-src 'none'; frame-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'; navigate-to 'none'";

export const REVIEW_THEME_TOKENS = [
  "--rf-color-fg",
  "--rf-color-muted",
  "--rf-color-accent",
  "--rf-color-accent-soft",
  "--rf-color-danger",
  "--rf-surface-1",
  "--rf-surface-2",
  "--rf-border",
  "--rf-font-ui",
  "--rf-font-mono",
  "--rf-text-2",
  "--rf-line-2",
  "--rf-weight-medium",
  "--rf-control-h",
  "--rf-radius-ctl",
  "--rf-space-2",
  "--rf-space-3",
] as const;

export type ReviewThemeToken = (typeof REVIEW_THEME_TOKENS)[number];
export type ReviewArtifactTheme = {
  appearance: "dark" | "light";
  tokens: Partial<Record<ReviewThemeToken, string>>;
  reducedMotion: boolean;
};

export function reviewThemePayload(theme: ReviewArtifactTheme) {
  return {
    appearance: theme.appearance,
    reducedMotion: theme.reducedMotion,
    tokens: Object.fromEntries(
      REVIEW_THEME_TOKENS.flatMap((name) => {
        const value = theme.tokens[name];
        return typeof value === "string" && !/url\s*\(/i.test(value)
          ? [[name, value]]
          : [];
      }),
    ),
  };
}

export function reviewArtifactHtml(
  code: string,
  channel = "",
  allowRevealAnchor = false,
  page = false,
): string {
  const doc = new DOMParser().parseFromString(
    wrapArtifactHtml(code),
    "text/html",
  );
  doc.querySelectorAll("base, meta").forEach((node) => {
    if (
      node.tagName.toLowerCase() === "base" ||
      node.getAttribute("http-equiv")?.trim().toLowerCase() === "refresh"
    )
      node.remove();
  });
  doc.querySelectorAll("a, area").forEach((node) => {
    const href = node.getAttribute("href");
    if (href && !href.startsWith("#")) node.removeAttribute("href");
    node.removeAttribute("target");
    node.removeAttribute("download");
  });
  doc
    .querySelectorAll(
      "script[data-refact-artifact], script[data-refact-artifact-bootstrap]",
    )
    .forEach((node) => {
      node.textContent = (node.textContent ?? "").replace(
        /type: 'refact-artifact-/g,
        `channel: ${JSON.stringify(channel)}, type: 'refact-artifact-`,
      );
    });
  const style = doc.createElement("style");
  style.textContent = `
:where(html) { color-scheme: light dark; }
html { font-family: var(--rf-font-ui); font-size: var(--rf-text-2); line-height: var(--rf-line-2); color: var(--rf-color-fg); }
html body { margin: 0; background: transparent; color: var(--rf-color-fg); font-family: var(--rf-font-ui); font-size: var(--rf-text-2); line-height: var(--rf-line-2); }
:where(button, input, select, textarea) { box-sizing: border-box; font: inherit; color: inherit; background: var(--rf-surface-1); border: 1px solid var(--rf-border); border-radius: var(--rf-radius-ctl); }
:where(button, input:not([type=checkbox]):not([type=radio]), select) { height: var(--rf-control-h); padding: 0 var(--rf-space-2); }
:where(button) { font-weight: var(--rf-weight-medium); cursor: pointer; }
:where(a) { color: var(--rf-color-accent); }
:where(code, pre) { font-family: var(--rf-font-mono); }
html[data-refact-reduced-motion=true] *, html[data-refact-reduced-motion=true] *::before, html[data-refact-reduced-motion=true] *::after { animation: none !important; transition: none !important; scroll-behavior: auto !important; }
`;
  doc.head.append(style);
  const bridge = doc.createElement("script");
  bridge.textContent = `(function() {
    var channel = ${JSON.stringify(channel)};
    var names = ${JSON.stringify(REVIEW_THEME_TOKENS)};
    window.addEventListener('message', function(event) {
      var d = event.data;
      if (event.source !== parent || !d || d.channel !== channel || d.type !== 'refact-review-theme') return;
      var t = d.theme;
      if (!t || (t.appearance !== 'light' && t.appearance !== 'dark')) return;
      document.documentElement.dataset.refactTheme = t.appearance;
      document.documentElement.dataset.refactReducedMotion = String(t.reducedMotion === true);
      names.forEach(function(name) {
        var value = t.tokens && t.tokens[name];
        if (typeof value === 'string' && !/url\\s*\\(/i.test(value)) document.documentElement.style.setProperty(name, value);
      });
    });
    document.addEventListener('click', function(event) {
      var link = event.target.closest && event.target.closest('a, area');
      if (link && !(link.getAttribute('href') || '').startsWith('#')) event.preventDefault();
    }, true);
    ${
      page
        ? `
    var previews = [];
    var rendered = new WeakMap();
    function action(kind, id) {
      if (typeof id === 'string' && id.length <= 256) parent.postMessage({type: 'refact-review-block-action', channel: channel, action: kind, id: id}, '*');
    }
    window.refactReview = Object.freeze({
      revealBlock: function(id) { action('reveal', id); },
      commentBlock: function(id) { action('comment', id); }
    });
    function slots() {
      var seen = new Set();
      document.querySelectorAll('refact-slot').forEach(function(slot) {
        var id = slot.getAttribute('data-block');
        var matches = previews.filter(function(p) { return p.id === id; });
        var p = matches.length === 1 && !seen.has(id) ? matches[0] : null;
        seen.add(id);
        var signature = JSON.stringify(p);
        if (rendered.get(slot) === signature) return;
        rendered.set(slot, signature);
        slot.replaceChildren();
        slot.style.display = 'block';
        var title = document.createElement('strong');
        title.textContent = p ? p.title : 'Block unavailable (unknown or duplicate slot)';
        slot.append(title);
        var pre = document.createElement('pre');
        pre.style.cssText = 'white-space:pre;overflow:auto;max-height:320px';
        pre.textContent = p ? p.text : 'No registered preview is available.';
        slot.append(pre);
        if (!p || !p.available) return;
        ['reveal', 'comment'].forEach(function(kind) {
          var button = document.createElement('button');
          button.type = 'button';
          button.textContent = kind === 'reveal' ? 'Reveal' : 'Comment';
          button.onclick = function() { action(kind, id); };
          slot.append(button);
        });
      });
    }
    window.addEventListener('message', function(event) {
      var d = event.data;
      if (event.source !== parent || !d || d.channel !== channel || d.type !== 'refact-review-page-data' || !Array.isArray(d.previews)) return;
      previews = d.previews;
      slots();
    });
    document.addEventListener('DOMContentLoaded', function() {
      new MutationObserver(slots).observe(document.body, {childList:true, subtree:true, attributes:true, attributeFilter:['data-block']});
      slots();
    });
    `
        : ""
    }
    ${
      allowRevealAnchor && !page
        ? `window.refactReview = Object.freeze({revealAnchor: function(index) {
      if (Number.isSafeInteger(index) && index >= 0) parent.postMessage({type: 'refact-review-reveal-anchor', channel: channel, index: index}, '*');
    }});`
        : ""
    }
  })();`;
  doc.head.prepend(bridge);
  const policy = doc.createElement("meta");
  policy.httpEquiv = "Content-Security-Policy";
  policy.content = REVIEW_ARTIFACT_CSP;
  doc.head.prepend(policy);
  const html = "<!doctype html>" + doc.documentElement.outerHTML;
  if (!page) return html;
  // Only trusted relay code inhabits the outer opaque document. Its frame-src
  // policy also governs self-navigation of the authored child frame.
  const inner = JSON.stringify(html).replace(/</g, "\\u003c");
  const relayChannel = JSON.stringify(channel).replace(/</g, "\\u003c");
  return `<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="${REVIEW_ARTIFACT_CSP}"><style>html,body{margin:0;background:transparent}iframe{display:block;width:100%;border:0}</style></head><body><iframe sandbox="allow-scripts" referrerpolicy="no-referrer" title="Authored review page"></iframe><script>(function(){
    var channel=${relayChannel};
    var frame=document.querySelector('iframe');
    var pending={};
    frame.addEventListener('load',function(){ Object.keys(pending).forEach(function(type){frame.contentWindow.postMessage(pending[type],'*');}); });
    window.addEventListener('message',function(event){
      var d=event.data;
      if(!d || d.channel!==channel) return;
      if(event.source===parent){ if(d.type!=='refact-review-theme' && d.type!=='refact-review-page-data') return; pending[d.type]=d; frame.contentWindow.postMessage(d,'*'); return; }
      if(event.source!==frame.contentWindow) return;
      if(d.type==='refact-artifact-resize' && typeof d.height==='number' && Number.isFinite(d.height)) frame.style.height=Math.max(1,Math.min(100000,d.height))+'px';
      if(['refact-artifact-resize','refact-artifact-error','refact-review-block-action'].indexOf(d.type)!==-1) parent.postMessage(d,'*');
    });
    frame.srcdoc=${inner};
  })();</script></body></html>`;
}
