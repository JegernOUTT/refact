(function() {
    'use strict';
    if (window.__refact_toolbar_installed) return;
    window.__refact_toolbar_installed = true;

    function send(action) {
        try {
            window.__refact_event(JSON.stringify({ type: 'toolbar_action', action: action, timestamp: Date.now() }));
        } catch(e) {}
    }

    var collapsed = false;
    var counts = { actions: 0, console: 0, network: 0, mutations: 0 };
    var host = document.createElement('div');
    host.id = '__refact_toolbar_host';
    host.style.cssText = 'all:initial;position:fixed;bottom:12px;left:50%;transform:translateX(-50%);z-index:2147483646;font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;';

    var shadow;
    try { shadow = host.attachShadow({ mode: 'closed' }); } catch(e) { return; }

    var style = document.createElement('style');
    style.textContent = [
        ':host{all:initial}',
        '.refact-bar{display:flex;align-items:center;gap:2px;background:rgba(24,24,27,0.92);border:1px solid rgba(255,255,255,0.1);border-radius:10px;padding:4px 6px;box-shadow:0 4px 24px rgba(0,0,0,0.4);backdrop-filter:blur(12px);user-select:none;-webkit-user-select:none}',
        '.refact-logo{width:28px;height:28px;border-radius:6px;border:none;background:transparent;cursor:pointer;display:flex;align-items:center;justify-content:center;flex-shrink:0;padding:0;transition:background 0.15s;color:#7c6aef}',
        '.refact-logo:hover{background:rgba(255,255,255,0.1)}',
        '.refact-logo svg{width:18px;height:18px}',
        '.refact-sep{width:1px;height:20px;background:rgba(255,255,255,0.15);margin:0 4px;flex-shrink:0}',
        '.refact-btn{width:28px;height:28px;border-radius:6px;border:none;background:transparent;cursor:pointer;display:flex;align-items:center;justify-content:center;padding:0;transition:background 0.15s,opacity 0.15s;position:relative}',
        '.refact-btn:hover{background:rgba(255,255,255,0.12)}',
        '.refact-btn:active{background:rgba(255,255,255,0.2)}',
        '.refact-btn svg{width:16px;height:16px;fill:none;stroke:rgba(255,255,255,0.85);stroke-width:1.5;stroke-linecap:round;stroke-linejoin:round}',
        '.refact-btn[data-action="screenshot"] svg,.refact-btn[data-action="screenshot_full"] svg{stroke-width:1.5}',
        '.refact-buttons{display:flex;align-items:center;gap:2px;overflow:hidden;transition:max-width 0.25s ease,opacity 0.2s ease}',
        '.refact-buttons.collapsed{max-width:0;opacity:0;pointer-events:none}',
        '.refact-buttons.expanded{max-width:600px;opacity:1}',
        '.refact-tip{position:absolute;bottom:calc(100% + 8px);left:50%;transform:translateX(-50%);background:rgba(24,24,27,0.95);color:rgba(255,255,255,0.9);font-size:11px;line-height:1;padding:5px 8px;border-radius:5px;white-space:nowrap;pointer-events:none;opacity:0;transition:opacity 0.15s;border:1px solid rgba(255,255,255,0.08)}',
        '.refact-btn:hover .refact-tip{opacity:1}',
        '.refact-badge{position:absolute;top:-4px;right:-4px;min-width:16px;height:16px;padding:0 4px;border-radius:999px;background:rgba(124,106,239,0.95);color:white;font-size:10px;line-height:16px;text-align:center;pointer-events:none}',
        '.refact-counts{display:flex;align-items:center;gap:6px;margin-left:8px;padding-left:8px;border-left:1px solid rgba(255,255,255,0.12);font-size:10px;line-height:1;color:rgba(255,255,255,0.75)}',
    ].join('\n');

    var icons = {
        screenshot: '<svg viewBox="0 0 24 24"><rect x="3" y="5" width="18" height="14" rx="2"/><circle cx="12" cy="12" r="3"/></svg>',
        screenshot_full: '<svg viewBox="0 0 24 24"><rect x="3" y="3" width="18" height="18" rx="2"/><polyline points="3 15 7 11 11 15"/><polyline points="13 12 16 9 21 14"/></svg>',
        pick_element: '<svg viewBox="0 0 24 24"><path d="M5 3l14 8-6 2-2 6z"/></svg>',
        paste_actions: '<svg viewBox="0 0 24 24"><rect x="4" y="4" width="16" height="16" rx="2"/><line x1="8" y1="9" x2="16" y2="9"/><line x1="8" y1="13" x2="13" y2="13"/></svg>',
        paste_console: '<svg viewBox="0 0 24 24"><polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/></svg>',
        paste_network: '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="9"/><line x1="3" y1="12" x2="21" y2="12"/><path d="M12 3c2.5 2.5 4 5.5 4 9s-1.5 6.5-4 9"/><path d="M12 3c-2.5 2.5-4 5.5-4 9s1.5 6.5 4 9"/></svg>',
        curl: '<svg viewBox="0 0 24 24"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/></svg>',
        summarize: '<svg viewBox="0 0 24 24"><path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20"/><path d="M4 4.5A2.5 2.5 0 0 1 6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15z"/></svg>',
        extract_json: '<svg viewBox="0 0 24 24"><rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/></svg>',
        annotate: '<svg viewBox="0 0 24 24"><path d="M4 20h4l10.5-10.5a2 2 0 0 0 0-3L16.5 4a2 2 0 0 0-3 0L3 14.5V20z"/><line x1="13" y1="6" x2="18" y2="11"/></svg>',
        annotate_send: '<svg viewBox="0 0 24 24"><path d="M22 2L11 13"/><path d="M22 2l-7 20-4-9-9-4 20-7z"/></svg>',
        annotate_clear: '<svg viewBox="0 0 24 24"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>',
    };

    var buttons = [
        { action: 'screenshot', tip: 'Screenshot', icon: icons.screenshot },
        { action: 'screenshot_full', tip: 'Full page', icon: icons.screenshot_full },
        { sep: true },
        { action: 'pick_element', tip: 'Pick element', icon: icons.pick_element },
        { sep: true },
        { action: 'paste_actions', tip: 'Actions → draft', icon: icons.paste_actions, countKey: 'actions' },
        { action: 'paste_console', tip: 'Console → draft', icon: icons.paste_console, countKey: 'console' },
        { action: 'paste_network', tip: 'Network → draft', icon: icons.paste_network, countKey: 'network' },
        { action: 'curl', tip: 'cURL → draft', icon: icons.curl },
        { sep: true },
        { action: 'summarize', tip: 'Summarize page', icon: icons.summarize },
        { action: 'extract_json', tip: 'Extract JSON', icon: icons.extract_json },
        { sep: true },
        { action: 'annotate', tip: 'Annotate: click elements, label', icon: icons.annotate },
        { action: 'annotate_send', tip: 'Annotate: send screenshot + labels', icon: icons.annotate_send },
        { action: 'annotate_clear', tip: 'Annotate: clear labels', icon: icons.annotate_clear },
    ];

    var bar = document.createElement('div');
    bar.className = 'refact-bar';

    // Logo toggle
    var logo = document.createElement('button');
    logo.className = 'refact-logo';
    logo.title = 'Refact';
    // Use the Refact icon (same as the app home button).
    logo.innerHTML = '<svg width="18" height="18" viewBox="200 180 400 480" fill="none" xmlns="http://www.w3.org/2000/svg">'
      + '<path d="M527.46 573.548C510.073 573.548 494.568 570.209 480.948 563.531C467.328 557.143 456.605 547.562 448.781 534.786C441.246 522.011 437.479 506.332 437.479 487.749L437.479 449.859C437.479 441.729 434.726 435.196 429.22 430.26C424.004 425.034 416.904 421.985 407.92 421.114L407.92 378.868C416.904 378.287 424.004 375.238 429.22 369.722C434.726 363.915 437.479 357.237 437.479 349.688L437.479 312.668C437.479 294.376 441.391 278.987 449.216 266.502C457.04 253.727 467.762 244 481.383 237.322C495.003 230.353 510.362 226.869 527.46 226.869L547.891 226.869L547.891 273.47H535.285C523.693 273.47 514.419 277.245 507.464 284.794C500.509 292.343 497.032 303.086 497.032 317.023L497.032 344.026C497.032 361.447 492.105 375.384 482.252 385.836C472.689 395.999 460.518 403.112 445.738 407.177L446.173 391.934C460.952 396.289 473.124 403.838 482.687 414.581C492.25 425.034 497.032 438.68 497.032 455.52L497.032 483.394C497.032 497.621 500.509 508.509 507.464 516.059C514.419 523.317 523.693 526.947 535.285 526.947H547.891L547.891 573.548H527.46Z" fill="currentColor"/>'
      + '<path d="M253 573.55L253 226L312.118 226L312.118 573.55L253 573.55ZM272.996 573.55L272.996 526.949L372.106 526.949L372.106 573.55L272.996 573.55ZM272.996 272.601L272.996 226L372.106 226L372.106 272.601L272.996 272.601Z" fill="currentColor"/>'
      + '</svg>';
    logo.addEventListener('click', function(e) {
        e.stopPropagation();
        collapsed = !collapsed;
        buttonsContainer.className = 'refact-buttons ' + (collapsed ? 'collapsed' : 'expanded');
    });
    bar.appendChild(logo);

    // Buttons container
    var buttonsContainer = document.createElement('div');
    buttonsContainer.className = 'refact-buttons expanded';

    var countsEl = document.createElement('div');
    countsEl.className = 'refact-counts';
    function renderCounts() {
        countsEl.textContent = 'A:' + counts.actions + ' C:' + counts.console + ' N:' + counts.network + ' M:' + counts.mutations;
    }
    renderCounts();

    for (var i = 0; i < buttons.length; i++) {
        var def = buttons[i];
        if (def.sep) {
            var sep = document.createElement('div');
            sep.className = 'refact-sep';
            buttonsContainer.appendChild(sep);
            continue;
        }
        var btn = document.createElement('button');
        btn.className = 'refact-btn';
        btn.setAttribute('data-action', def.action);
        var badge = def.countKey ? ('<span class="refact-badge" data-count="' + def.countKey + '">0</span>') : '';
        btn.innerHTML = def.icon + badge + '<span class="refact-tip">' + def.tip + '</span>';
        btn.addEventListener('click', (function(action) {
            return function(e) {
                e.stopPropagation();
                send(action);
                // Brief flash feedback
                var el = e.currentTarget;
                el.style.background = 'rgba(124,106,239,0.3)';
                setTimeout(function() { el.style.background = ''; }, 200);
            };
        })(def.action));
        buttonsContainer.appendChild(btn);
    }

    buttonsContainer.appendChild(countsEl);

    bar.appendChild(buttonsContainer);
    shadow.appendChild(style);
    shadow.appendChild(bar);

    window.__refact_toolbar_setCounts = function(next) {
        if (!next || typeof next !== 'object') return;
        counts.actions = Number(next.actions || 0);
        counts.console = Number(next.console || 0);
        counts.network = Number(next.network || 0);
        counts.mutations = Number(next.mutations || 0);
        renderCounts();

        try {
            var badges = shadow.querySelectorAll('.refact-badge[data-count]');
            for (var i = 0; i < badges.length; i++) {
                var el = badges[i];
                var key = el.getAttribute('data-count');
                if (key && counts.hasOwnProperty(key)) {
                    el.textContent = String(counts[key]);
                }
            }
        } catch(e) {}
    };

    // Wait for body
    function mount() {
        if (document.body) {
            document.body.appendChild(host);
        } else {
            document.addEventListener('DOMContentLoaded', function() {
                document.body.appendChild(host);
            });
        }
    }
    mount();
})();
