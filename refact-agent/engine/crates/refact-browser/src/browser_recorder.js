(function() {
    'use strict';
    if (window.__refact_recorder_installed) return;
    window.__refact_recorder_installed = true;

    var MASK_PASSWORDS = __REFACT_MASK_PASSWORDS__;

    function cssEscape(str) {
        return str.replace(/[\\"']/g, '\\$&').replace(/[\x00-\x1f]/g, '');
    }

    function getSelector(el) {
        if (!el || !el.tagName) return '';
        if (el.id) return '#' + cssEscape(el.id);
        if (el.name) return el.tagName.toLowerCase() + '[name="' + cssEscape(el.name) + '"]';
        if (el.className && typeof el.className === 'string') {
            var cls = el.className.trim().split(/\s+/).slice(0, 2).map(cssEscape).join('.');
            if (cls) return el.tagName.toLowerCase() + '.' + cls;
        }
        return el.tagName.toLowerCase();
    }

    function getTimestamp() {
        return Date.now();
    }

    function send(data) {
        try {
            window.__refact_event(JSON.stringify(data));
        } catch(e) {}
    }

    function isPasswordField(el) {
        if (!el) return false;
        if (el.type === 'password') return true;
        var ac = (el.autocomplete || '').toLowerCase();
        if (ac === 'current-password' || ac === 'new-password') return true;
        return false;
    }

    send({
        type: 'navigation',
        url: location.href,
        title: document.title || '',
        timestamp: getTimestamp()
    });

    document.addEventListener('click', function(e) {
        var el = e.target;
        send({
            type: 'click',
            selector: getSelector(el),
            text: (el.textContent || '').substring(0, 100).trim(),
            x: e.clientX,
            y: e.clientY,
            timestamp: getTimestamp()
        });
    }, true);

    document.addEventListener('input', function(e) {
        var el = e.target;
        var masked = MASK_PASSWORDS && isPasswordField(el);
        var data = {
            type: 'input',
            selector: getSelector(el),
            value: masked ? '*'.repeat((el.value || '').length) : (el.value || ''),
            masked: masked,
            timestamp: getTimestamp()
        };
        if (el.tagName) data.tag = el.tagName.toLowerCase();
        if (el.type) data.input_type = el.type;
        if (el.name) data.field_name = el.name;
        if (el.placeholder) data.placeholder = el.placeholder;
        var ariaLabel = el.getAttribute && el.getAttribute('aria-label');
        if (ariaLabel) data.aria_label = ariaLabel;
        var role = el.getAttribute && el.getAttribute('role');
        if (role) data.role = role;
        send(data);
    }, true);

    document.addEventListener('keydown', function(e) {
        if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) return;
        var modifiers = [];
        if (e.ctrlKey) modifiers.push('Ctrl');
        if (e.altKey) modifiers.push('Alt');
        if (e.metaKey) modifiers.push('Meta');
        if (e.shiftKey) modifiers.push('Shift');
        send({
            type: 'keypress',
            key: e.key,
            modifiers: modifiers,
            timestamp: getTimestamp()
        });
    }, true);

    document.addEventListener('submit', function(e) {
        var el = e.target;
        send({
            type: 'submit',
            selector: getSelector(el),
            action: el.action || '',
            method: (el.method || 'GET').toUpperCase(),
            timestamp: getTimestamp()
        });
    }, true);

    var scrollTimeout = null;
    window.addEventListener('scroll', function() {
        if (scrollTimeout) clearTimeout(scrollTimeout);
        scrollTimeout = setTimeout(function() {
            send({
                type: 'scroll',
                scroll_x: window.scrollX || window.pageXOffset || 0,
                scroll_y: window.scrollY || window.pageYOffset || 0,
                timestamp: getTimestamp()
            });
        }, 150);
    }, true);

    if (typeof MutationObserver !== 'undefined') {
        var mutationBatch = { added: 0, removed: 0, changed: 0 };
        var mutationTimeout = null;

        var observer = new MutationObserver(function(mutations) {
            for (var i = 0; i < mutations.length; i++) {
                var m = mutations[i];
                if (m.type === 'childList') {
                    mutationBatch.added += m.addedNodes.length;
                    mutationBatch.removed += m.removedNodes.length;
                } else if (m.type === 'attributes' || m.type === 'characterData') {
                    mutationBatch.changed += 1;
                }
            }

            if (mutationTimeout) clearTimeout(mutationTimeout);
            mutationTimeout = setTimeout(function() {
                if (mutationBatch.added || mutationBatch.removed || mutationBatch.changed) {
                    send({
                        type: 'mutation_summary',
                        added: mutationBatch.added,
                        removed: mutationBatch.removed,
                        changed: mutationBatch.changed,
                        timestamp: getTimestamp()
                    });
                    mutationBatch = { added: 0, removed: 0, changed: 0 };
                }
            }, 500);
        });

        function startObserver() {
            try {
                var root = document.documentElement || document.body;
                if (!root) return false;
                observer.observe(root, {
                    childList: true,
                    subtree: true,
                    attributes: true,
                    characterData: true
                });
                return true;
            } catch (e) {
                return false;
            }
        }

        if (!startObserver()) {
            document.addEventListener('DOMContentLoaded', function() {
                startObserver();
            }, { once: true });
        }
    }
})();
