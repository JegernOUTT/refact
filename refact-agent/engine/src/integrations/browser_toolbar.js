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
    var host = document.createElement('div');
    host.id = '__refact_toolbar_host';
    host.style.cssText = 'all:initial;position:fixed;bottom:12px;left:50%;transform:translateX(-50%);z-index:2147483646;font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;';

    var shadow;
    try { shadow = host.attachShadow({ mode: 'closed' }); } catch(e) { return; }

    var style = document.createElement('style');
    style.textContent = [
        ':host{all:initial}',
        '.refact-bar{display:flex;align-items:center;gap:2px;background:rgba(24,24,27,0.92);border:1px solid rgba(255,255,255,0.1);border-radius:10px;padding:4px 6px;box-shadow:0 4px 24px rgba(0,0,0,0.4);backdrop-filter:blur(12px);user-select:none;-webkit-user-select:none}',
        '.refact-logo{width:28px;height:28px;border-radius:6px;border:none;background:transparent;cursor:pointer;display:flex;align-items:center;justify-content:center;flex-shrink:0;padding:0;transition:background 0.15s}',
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
    };

    var buttons = [
        { action: 'screenshot', tip: 'Screenshot', icon: icons.screenshot },
        { action: 'screenshot_full', tip: 'Full page', icon: icons.screenshot_full },
        { sep: true },
        { action: 'pick_element', tip: 'Pick element', icon: icons.pick_element },
        { sep: true },
        { action: 'paste_actions', tip: 'Actions → chat', icon: icons.paste_actions },
        { action: 'paste_console', tip: 'Console → chat', icon: icons.paste_console },
        { action: 'paste_network', tip: 'Network → chat', icon: icons.paste_network },
        { action: 'curl', tip: 'cURL → chat', icon: icons.curl },
        { sep: true },
        { action: 'summarize', tip: 'Summarize page', icon: icons.summarize },
        { action: 'extract_json', tip: 'Extract JSON', icon: icons.extract_json },
    ];

    var bar = document.createElement('div');
    bar.className = 'refact-bar';

    // Logo toggle
    var logo = document.createElement('button');
    logo.className = 'refact-logo';
    logo.title = 'Refact';
    logo.innerHTML = '<svg viewBox="0 0 24 24" fill="none"><circle cx="12" cy="12" r="10" stroke="#7c6aef" stroke-width="2"/><path d="M8 8h5a3 3 0 0 1 0 6h-5v-6z" stroke="#7c6aef" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/><line x1="11" y1="14" x2="16" y2="19" stroke="#7c6aef" stroke-width="2" stroke-linecap="round"/></svg>';
    logo.addEventListener('click', function(e) {
        e.stopPropagation();
        collapsed = !collapsed;
        buttonsContainer.className = 'refact-buttons ' + (collapsed ? 'collapsed' : 'expanded');
    });
    bar.appendChild(logo);

    // Buttons container
    var buttonsContainer = document.createElement('div');
    buttonsContainer.className = 'refact-buttons expanded';

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
        btn.innerHTML = def.icon + '<span class="refact-tip">' + def.tip + '</span>';
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

    bar.appendChild(buttonsContainer);
    shadow.appendChild(style);
    shadow.appendChild(bar);

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
