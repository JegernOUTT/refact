import { wrapArtifactHtml } from "./renderUtils";

export const REVIEW_ARTIFACT_CSP =
  "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; font-src 'none'; media-src data:; connect-src 'none'; frame-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'; navigate-to 'none'";

export const REVIEW_THEME_TOKENS = [
  "--rf-color-fg",
  "--rf-color-muted",
  "--rf-color-accent",
  "--rf-color-accent-soft",
  "--rf-color-danger",
  "--rf-color-danger-soft",
  "--rf-color-success",
  "--rf-color-success-soft",
  "--rf-color-warning",
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
:where(input:not([type=checkbox]):not([type=radio]), select) { height: var(--rf-control-h); padding: 0 var(--rf-space-2); }
:where(button) { min-height: var(--rf-control-h); padding: var(--rf-space-1) var(--rf-space-2); font-weight: var(--rf-weight-medium); cursor: pointer; }
:where(a) { color: var(--rf-color-accent); }
:where(code, pre) { font-family: var(--rf-font-mono); }
html[data-refact-reduced-motion=true] *, html[data-refact-reduced-motion=true] *::before, html[data-refact-reduced-motion=true] *::after { animation: none !important; transition: none !important; scroll-behavior: auto !important; }
refact-slot { display: block; margin: var(--rf-space-3) 0; }
.rf-insert { border: 1px solid var(--rf-border); border-radius: var(--rf-radius-ctl); overflow: hidden; background: var(--rf-surface-1); }
.rf-insert-head { display: flex; align-items: center; justify-content: space-between; gap: var(--rf-space-2); padding: var(--rf-space-2) var(--rf-space-3); border-bottom: 1px solid var(--rf-border); background: var(--rf-surface-2); color: var(--rf-color-muted); font-family: var(--rf-font-mono); font-size: 0.85em; }
.rf-insert-title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.rf-insert-actions { display: flex; flex: none; gap: var(--rf-space-2); }
.rf-insert-actions button { min-height: 0; padding: 0 var(--rf-space-2); font-size: 0.95em; }
.rf-insert-body { overflow: auto; max-height: 28em; font-family: var(--rf-font-mono); font-size: 0.85em; line-height: 1.55; }
.rf-line { display: flex; min-width: max-content; white-space: pre; }
.rf-line-n { flex: none; min-width: 4ch; padding: 0 var(--rf-space-2); text-align: right; color: var(--rf-color-muted); user-select: none; }
.rf-line-t { flex: auto; padding-right: var(--rf-space-3); }
.rf-line[data-kind=add] { background: var(--rf-color-success-soft); }
.rf-line[data-kind=del] { background: var(--rf-color-danger-soft); }
.rf-line[data-kind=gap] { background: var(--rf-surface-2); color: var(--rf-color-muted); }
.rf-line[data-hl=true] { background: var(--rf-color-accent-soft); box-shadow: inset 3px 0 0 var(--rf-color-accent); }
.rf-tok-k { color: var(--rf-color-accent); }
.rf-tok-s { color: var(--rf-color-success); }
.rf-tok-n { color: var(--rf-color-warning); }
.rf-tok-c { color: var(--rf-color-muted); font-style: italic; }
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
    function wanted(spec) {
      var set = {};
      String(spec || '').split(',').forEach(function(part) {
        var m = /^\\s*L?(\\d+)(?:\\s*[-:]\\s*L?(\\d+))?\\s*$/i.exec(part);
        if (!m) return;
        var a = Number(m[1]), b = Number(m[2] || m[1]);
        for (var n = a; n <= b && n - a < 2000; n++) set[n] = true;
      });
      return set;
    }
    function paint(slot, scroll) {
      var set = wanted(slot.getAttribute('data-highlight'));
      var first = null;
      slot.querySelectorAll('.rf-line').forEach(function(line) {
        var on = line.getAttribute('data-kind') !== 'del' && set[Number(line.getAttribute('data-n'))] === true;
        line.setAttribute('data-hl', String(on));
        if (on && !first) first = line;
      });
      var body = slot.querySelector('.rf-insert-body');
      if (scroll && first && body) body.scrollTop = Math.max(0, first.offsetTop - body.offsetTop - body.clientHeight / 3);
    }
    function find(id) {
      var found = [];
      document.querySelectorAll('refact-slot').forEach(function(slot) { if (slot.getAttribute('data-block') === id) found.push(slot); });
      return found;
    }
    window.refactReview = Object.freeze({
      revealBlock: function(id) { action('reveal', id); },
      reveal: function(id) { action('reveal', id); },
      commentBlock: function(id) { action('comment', id); },
      highlight: function(id, lines) {
        find(String(id)).forEach(function(slot) {
          slot.setAttribute('data-highlight', String(lines == null ? '' : lines).slice(0, 512));
          paint(slot, true);
        });
      }
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
        var box = document.createElement('div');
        box.className = 'rf-insert';
        var head = document.createElement('div');
        head.className = 'rf-insert-head';
        var title = document.createElement('span');
        title.className = 'rf-insert-title';
        title.textContent = slot.getAttribute('data-title') || (p ? p.title : 'Block unavailable (unknown or duplicate slot)');
        head.append(title);
        var body = document.createElement('div');
        body.className = 'rf-insert-body';
        if (p && Array.isArray(p.lines)) {
          p.lines.forEach(function(item) {
            var line = document.createElement('div');
            line.className = 'rf-line';
            line.setAttribute('data-kind', item.kind);
            if (item.n != null) line.setAttribute('data-n', String(item.n));
            var n = document.createElement('span');
            n.className = 'rf-line-n';
            n.textContent = item.n == null ? '' : String(item.n);
            var t = document.createElement('span');
            t.className = 'rf-line-t';
            t.textContent = item.kind === 'add' ? '+ ' : item.kind === 'del' ? '- ' : item.kind === 'gap' ? '' : '  ';
            if (Array.isArray(item.tokens)) {
              item.tokens.forEach(function(token) {
                if (!Array.isArray(token) || typeof token[1] !== 'string') return;
                var span = document.createElement('span');
                if (/^[kscn]$/.test(token[0])) span.className = 'rf-tok-' + token[0];
                span.textContent = token[1];
                t.append(span);
              });
            } else {
              t.append(String(item.text));
            }
            line.append(n, t);
            body.append(line);
          });
        } else {
          var pre = document.createElement('pre');
          pre.style.cssText = 'margin:0;padding:var(--rf-space-2) var(--rf-space-3);white-space:pre-wrap';
          pre.textContent = p ? p.text : 'No registered preview is available.';
          body.append(pre);
        }
        if (p && p.available) {
          var actions = document.createElement('span');
          actions.className = 'rf-insert-actions';
          ['reveal', 'comment'].forEach(function(kind) {
            var button = document.createElement('button');
            button.type = 'button';
            button.textContent = kind === 'reveal' ? 'Open' : 'Comment';
            button.onclick = function() { action(kind, id); };
            actions.append(button);
          });
          head.append(actions);
        }
        box.append(head, body);
        slot.append(box);
        paint(slot, false);
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
