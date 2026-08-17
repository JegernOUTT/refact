(function() {
    'use strict';
    if (window.__refact_overlays_installed) return;

    var Z_ANNOTATION_BACKDROP = 2147483643;
    var Z_ANNOTATE_OVERLAY = 2147483645;
    var Z_TOOLBAR_HOST = 2147483646;
    var Z_ANNOTATION_MARKER = 2147483647;
    var Z_CAPTION_INPUT = 2147483647;
    var Z_PICKER_OVERLAY = 2147483647;

    var COLOR = '#E7150D';
    var TOOLBAR_HOST_ID = '__refact_toolbar_host';
    var PICKER_OVERLAY_ID = '__refact_picker_overlay';
    var ANNOTATE_OVERLAY_ID = '__refact_annotate_overlay';
    var CAPTION_WRAP_CLASS = '__refact_annotation_caption_wrap';
    var MARKER_CLASS = '__refact_annotation_marker';
    var LABEL_CLASS = '__refact_annotation_label';
    var GUIDE_CLASS = '__refact_annotation_guide';
    var RECT_CLASS = '__refact_annotation_rect';

    function documentRoot() {
        return document.body || document.documentElement;
    }

    function selectorFor(el) {
        if (!el || !el.tagName) return '';
        if (el.id) return '#' + el.id;
        if (el.className && typeof el.className === 'string') {
            var classes = el.className.trim().split(/\s+/).filter(Boolean);
            if (classes.length > 0) {
                return el.tagName.toLowerCase() + '.' + classes.join('.');
            }
        }
        return el.tagName.toLowerCase();
    }

    function roundedBox(rect) {
        return {
            x: Math.round(rect.x),
            y: Math.round(rect.y),
            width: Math.round(rect.width),
            height: Math.round(rect.height)
        };
    }

    function elementUnderPoint(overlay, x, y) {
        overlay.style.display = 'none';
        var el = document.elementFromPoint(x, y);
        overlay.style.display = '';
        return el;
    }

    var picker = {
        active: false,
        overlay: null,
        highlighted: null,
        keyHandler: null,
        timer: null,
        result: null
    };

    function clearPickerHighlight() {
        if (picker.highlighted && picker.highlighted.style) {
            picker.highlighted.style.outline = picker.highlighted.__refact_prev_pick_outline || '';
        }
        picker.highlighted = null;
    }

    function releasePicker() {
        clearPickerHighlight();
        if (picker.timer) {
            clearTimeout(picker.timer);
            picker.timer = null;
        }
        if (picker.keyHandler) {
            document.removeEventListener('keydown', picker.keyHandler, true);
            picker.keyHandler = null;
        }
        if (picker.overlay) {
            picker.overlay.remove();
            picker.overlay = null;
        }
        var stale = document.getElementById(PICKER_OVERLAY_ID);
        if (stale) stale.remove();
        picker.active = false;
        window.__refact_picker_active = false;
    }

    function startPicker(timeoutMs) {
        if (picker.active) return 'already_active';
        var root = documentRoot();
        if (!root) return 'no_document';
        picker.result = null;
        picker.active = true;
        window.__refact_picker_active = true;

        var overlay = document.createElement('div');
        overlay.id = PICKER_OVERLAY_ID;
        overlay.style.cssText = 'position:fixed;top:0;left:0;width:100%;height:100%;cursor:crosshair;z-index:' + Z_PICKER_OVERLAY + ';';
        root.appendChild(overlay);
        picker.overlay = overlay;

        overlay.addEventListener('mousemove', function(e) {
            clearPickerHighlight();
            var el = elementUnderPoint(overlay, e.clientX, e.clientY);
            if (el && el.id !== TOOLBAR_HOST_ID && el.style) {
                el.__refact_prev_pick_outline = el.style.outline;
                el.style.outline = '2px solid ' + COLOR;
                picker.highlighted = el;
            }
        });

        overlay.addEventListener('click', function(e) {
            e.preventDefault();
            e.stopPropagation();
            clearPickerHighlight();
            var el = elementUnderPoint(overlay, e.clientX, e.clientY);
            if (el && el.id !== TOOLBAR_HOST_ID) {
                picker.result = {
                    selector: selectorFor(el),
                    innerText: (el.innerText || '').substring(0, 500),
                    bbox: roundedBox(el.getBoundingClientRect())
                };
            }
            releasePicker();
        });

        picker.keyHandler = function(e) {
            if (e.key === 'Escape') {
                e.preventDefault();
                e.stopPropagation();
                releasePicker();
            }
        };
        document.addEventListener('keydown', picker.keyHandler, true);

        var limit = Number(timeoutMs);
        if (isFinite(limit) && limit > 0) {
            picker.timer = setTimeout(releasePicker, limit);
        }
        return 'started';
    }

    function readPicked() {
        return picker.result;
    }

    var annotate = {
        active: false,
        overlay: null,
        captionInput: null,
        keyHandler: null,
        hovered: null,
        dragStart: null,
        dragRect: null,
        nextIndex: 1,
        entries: [],
        elements: []
    };

    var DRAG_THRESHOLD = 8;

    function syncToolbarAnnotateState(active) {
        if (typeof window.__refact_toolbar_setAnnotateActive === 'function') {
            window.__refact_toolbar_setAnnotateActive(active);
        }
    }

    function addGuides(bbox) {
        var root = documentRoot();
        var guideColor = 'rgba(231,21,13,0.3)';
        var base = 'position:fixed;pointer-events:none;z-index:' + Z_ANNOTATION_BACKDROP + ';';
        var edges = [
            base + 'left:0;width:100%;height:0;border-top:1px dashed ' + guideColor + ';top:' + bbox.y + 'px;',
            base + 'left:0;width:100%;height:0;border-top:1px dashed ' + guideColor + ';top:' + (bbox.y + bbox.height) + 'px;',
            base + 'top:0;height:100%;width:0;border-left:1px dashed ' + guideColor + ';left:' + bbox.x + 'px;',
            base + 'top:0;height:100%;width:0;border-left:1px dashed ' + guideColor + ';left:' + (bbox.x + bbox.width) + 'px;'
        ];
        for (var i = 0; i < edges.length; i++) {
            var guide = document.createElement('div');
            guide.className = GUIDE_CLASS;
            guide.style.cssText = edges[i];
            root.appendChild(guide);
        }
    }

    function addMarker(index, bbox) {
        var top = Math.max(0, bbox.y - 28);
        var left = bbox.x + bbox.width / 2 - 12;
        var marker = document.createElement('div');
        marker.className = MARKER_CLASS;
        marker.style.cssText = 'position:fixed;width:24px;height:24px;border-radius:50%;'
            + 'background:' + COLOR + ';color:white;font-size:12px;font-weight:bold;font-family:sans-serif;'
            + 'display:flex;align-items:center;justify-content:center;pointer-events:none;'
            + 'box-shadow:0 2px 8px rgba(0,0,0,0.3);border:2px solid white;'
            + 'z-index:' + Z_ANNOTATION_MARKER + ';'
            + 'left:' + Math.round(left) + 'px;top:' + Math.round(top) + 'px;';
        marker.textContent = String(index);
        documentRoot().appendChild(marker);
        return { left: left, top: top };
    }

    function addCaptionLabel(text, left, top) {
        if (!text) return;
        var label = document.createElement('div');
        label.className = LABEL_CLASS;
        label.style.cssText = 'position:fixed;pointer-events:none;'
            + 'background:rgba(24,24,27,0.9);color:white;font-size:10px;padding:2px 6px;'
            + 'border-radius:3px;font-family:sans-serif;max-width:200px;overflow:hidden;'
            + 'text-overflow:ellipsis;white-space:nowrap;border:1px solid rgba(231,21,13,0.4);'
            + 'z-index:' + Z_ANNOTATION_MARKER + ';'
            + 'left:' + Math.round(left + 28) + 'px;top:' + Math.round(top + 2) + 'px;';
        label.textContent = text;
        documentRoot().appendChild(label);
    }

    function showCaptionInput(left, top, done) {
        var wrap = document.createElement('div');
        wrap.className = CAPTION_WRAP_CLASS;
        wrap.style.cssText = 'position:fixed;z-index:' + Z_CAPTION_INPUT + ';left:' + Math.round(left + 30) + 'px;top:' + Math.round(top) + 'px;';
        var input = document.createElement('input');
        input.type = 'text';
        input.placeholder = 'Caption (Enter to skip)';
        input.style.cssText = 'width:180px;height:24px;border:1px solid rgba(231,21,13,0.5);border-radius:4px;'
            + 'background:rgba(24,24,27,0.95);color:white;font-size:11px;padding:0 6px;outline:none;'
            + 'font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;';
        wrap.appendChild(input);
        documentRoot().appendChild(wrap);
        annotate.captionInput = input;
        if (annotate.overlay) annotate.overlay.style.pointerEvents = 'none';
        input.focus();

        function finish() {
            var text = input.value.trim();
            wrap.remove();
            annotate.captionInput = null;
            if (annotate.overlay) annotate.overlay.style.pointerEvents = '';
            done(text);
        }

        input.addEventListener('keydown', function(e) {
            e.stopPropagation();
            if (e.key === 'Enter' || e.key === 'Escape') {
                e.preventDefault();
                finish();
            }
        });
        input.addEventListener('blur', function() {
            setTimeout(function() {
                if (annotate.captionInput === input) finish();
            }, 150);
        });
    }

    function removeAll(selector) {
        var nodes = document.querySelectorAll(selector);
        for (var i = 0; i < nodes.length; i++) nodes[i].remove();
    }

    function restoreAnnotatedElements() {
        for (var i = 0; i < annotate.elements.length; i++) {
            var el = annotate.elements[i];
            if (el && el.style) {
                el.style.outline = el.__refact_prev_outline_saved || '';
                el.style.outlineOffset = el.__refact_prev_outlineOffset_saved || '';
            }
        }
        annotate.elements = [];
    }

    function clearAnnotationArtifacts() {
        removeAll('.' + MARKER_CLASS);
        removeAll('.' + LABEL_CLASS);
        removeAll('.' + GUIDE_CLASS);
        removeAll('.' + RECT_CLASS);
        restoreAnnotatedElements();
        annotate.entries = [];
        annotate.nextIndex = 1;
    }

    function clearHovered() {
        if (annotate.hovered && annotate.hovered.style) {
            annotate.hovered.style.outline = annotate.hovered.__refact_prev_outline || '';
            annotate.hovered.style.outlineOffset = annotate.hovered.__refact_prev_outlineOffset || '';
        }
        annotate.hovered = null;
    }

    function exitAnnotate() {
        if (annotate.captionInput) {
            var wrap = annotate.captionInput.parentElement;
            if (wrap) wrap.remove();
            annotate.captionInput = null;
        }
        clearHovered();
        if (annotate.dragRect) {
            annotate.dragRect.remove();
            annotate.dragRect = null;
        }
        annotate.dragStart = null;
        if (annotate.overlay) {
            annotate.overlay.remove();
            annotate.overlay = null;
        }
        var stale = document.getElementById(ANNOTATE_OVERLAY_ID);
        if (stale) stale.remove();
        if (annotate.keyHandler) {
            document.removeEventListener('keydown', annotate.keyHandler, true);
            annotate.keyHandler = null;
        }
        annotate.active = false;
        window.__refact_annotate_active = false;
        syncToolbarAnnotateState(false);
    }

    function undoLast() {
        if (annotate.entries.length === 0) return;
        var last = annotate.entries.pop();
        annotate.nextIndex--;
        var markers = document.querySelectorAll('.' + MARKER_CLASS);
        if (markers.length > 0) markers[markers.length - 1].remove();
        var labels = document.querySelectorAll('.' + LABEL_CLASS);
        if (labels.length > 0) labels[labels.length - 1].remove();
        var guides = document.querySelectorAll('.' + GUIDE_CLASS);
        for (var i = 0; i < 4 && guides.length - 1 - i >= 0; i++) {
            guides[guides.length - 1 - i].remove();
        }
        if (last && last.type === 'rect') {
            var rects = document.querySelectorAll('.' + RECT_CLASS);
            if (rects.length > 0) rects[rects.length - 1].remove();
        } else if (annotate.elements.length > 0) {
            var el = annotate.elements.pop();
            if (el && el.style) {
                el.style.outline = el.__refact_prev_outline_saved || '';
                el.style.outlineOffset = el.__refact_prev_outlineOffset_saved || '';
            }
        }
    }

    function captureRect(bbox) {
        var index = annotate.nextIndex++;
        var rect = document.createElement('div');
        rect.className = RECT_CLASS;
        rect.style.cssText = 'position:fixed;pointer-events:none;border:2px solid ' + COLOR + ';'
            + 'background:rgba(231,21,13,0.06);border-radius:2px;z-index:' + Z_ANNOTATION_BACKDROP + ';'
            + 'left:' + bbox.x + 'px;top:' + bbox.y + 'px;width:' + bbox.width + 'px;height:' + bbox.height + 'px;';
        documentRoot().appendChild(rect);
        var point = addMarker(index, bbox);
        addGuides(bbox);
        showCaptionInput(point.left, point.top, function(caption) {
            annotate.entries.push({
                index: index,
                type: 'rect',
                selector: '',
                innerText: '',
                caption: caption || '',
                bbox: bbox
            });
            addCaptionLabel(caption, point.left, point.top);
        });
    }

    function captureElement(el) {
        var bbox = roundedBox(el.getBoundingClientRect());
        var selector = selectorFor(el);
        var index = annotate.nextIndex++;
        el.__refact_prev_outline_saved = el.style.outline;
        el.__refact_prev_outlineOffset_saved = el.style.outlineOffset;
        el.style.outline = '2px solid ' + COLOR;
        el.style.outlineOffset = '2px';
        annotate.elements.push(el);
        var point = addMarker(index, bbox);
        addGuides(bbox);
        showCaptionInput(point.left, point.top, function(caption) {
            annotate.entries.push({
                index: index,
                type: 'element',
                selector: selector,
                innerText: (el.innerText || '').substring(0, 300),
                caption: caption || '',
                bbox: bbox
            });
            addCaptionLabel(caption, point.left, point.top);
        });
    }

    function startAnnotate() {
        if (annotate.active) return 'already_active';
        var root = documentRoot();
        if (!root) return 'no_document';
        annotate.active = true;
        window.__refact_annotate_active = true;
        annotate.nextIndex = annotate.entries.length + 1;
        syncToolbarAnnotateState(true);

        var overlay = document.createElement('div');
        overlay.id = ANNOTATE_OVERLAY_ID;
        overlay.style.cssText = 'position:fixed;top:0;left:0;width:100%;height:100%;cursor:crosshair;z-index:' + Z_ANNOTATE_OVERLAY + ';';
        root.appendChild(overlay);
        annotate.overlay = overlay;

        overlay.addEventListener('mousemove', function(e) {
            if (annotate.captionInput) return;
            if (annotate.dragStart) {
                var x = Math.min(e.clientX, annotate.dragStart.x);
                var y = Math.min(e.clientY, annotate.dragStart.y);
                var w = Math.abs(e.clientX - annotate.dragStart.x);
                var h = Math.abs(e.clientY - annotate.dragStart.y);
                if (!annotate.dragRect) {
                    annotate.dragRect = document.createElement('div');
                    annotate.dragRect.style.cssText = 'position:fixed;pointer-events:none;border:2px dashed ' + COLOR + ';'
                        + 'background:rgba(231,21,13,0.08);border-radius:2px;z-index:' + Z_ANNOTATION_MARKER + ';';
                    root.appendChild(annotate.dragRect);
                }
                annotate.dragRect.style.left = x + 'px';
                annotate.dragRect.style.top = y + 'px';
                annotate.dragRect.style.width = w + 'px';
                annotate.dragRect.style.height = h + 'px';
                clearHovered();
                return;
            }
            clearHovered();
            var el = elementUnderPoint(overlay, e.clientX, e.clientY);
            if (el && el.id !== TOOLBAR_HOST_ID && !(el.closest && el.closest('.' + CAPTION_WRAP_CLASS)) && el.style) {
                el.__refact_prev_outline = el.style.outline;
                el.__refact_prev_outlineOffset = el.style.outlineOffset;
                el.style.outline = '2px solid ' + COLOR;
                annotate.hovered = el;
            }
        });

        overlay.addEventListener('mousedown', function(e) {
            if (annotate.captionInput || e.button !== 0) return;
            annotate.dragStart = { x: e.clientX, y: e.clientY };
            annotate.dragRect = null;
        });

        overlay.addEventListener('mouseup', function(e) {
            if (annotate.captionInput || e.button !== 0 || !annotate.dragStart) return;
            var dx = Math.abs(e.clientX - annotate.dragStart.x);
            var dy = Math.abs(e.clientY - annotate.dragStart.y);
            if (dx > DRAG_THRESHOLD || dy > DRAG_THRESHOLD) {
                if (annotate.dragRect) annotate.dragRect.remove();
                annotate.dragRect = null;
                var bx = Math.min(e.clientX, annotate.dragStart.x);
                var by = Math.min(e.clientY, annotate.dragStart.y);
                var bw = Math.abs(e.clientX - annotate.dragStart.x);
                var bh = Math.abs(e.clientY - annotate.dragStart.y);
                annotate.dragStart = null;
                if (bw < 5 || bh < 5) return;
                captureRect({ x: Math.round(bx), y: Math.round(by), width: Math.round(bw), height: Math.round(bh) });
                return;
            }
            annotate.dragStart = null;
            if (annotate.dragRect) {
                annotate.dragRect.remove();
                annotate.dragRect = null;
            }
            var el = elementUnderPoint(overlay, e.clientX, e.clientY);
            clearHovered();
            if (!el || el.id === TOOLBAR_HOST_ID) return;
            captureElement(el);
        });

        overlay.addEventListener('click', function(e) {
            e.preventDefault();
            e.stopPropagation();
        });

        overlay.addEventListener('contextmenu', function(e) {
            e.preventDefault();
            e.stopPropagation();
            undoLast();
        });

        annotate.keyHandler = function(e) {
            if (e.key !== 'Escape' || annotate.captionInput) return;
            clearAnnotationArtifacts();
            exitAnnotate();
        };
        document.addEventListener('keydown', annotate.keyHandler, true);
        return 'started';
    }

    function readAnnotations() {
        return { annotations: annotate.entries.slice(), active: annotate.active };
    }

    function clearAnnotations() {
        clearAnnotationArtifacts();
        exitAnnotate();
        return 'cleared';
    }

    function releaseAll() {
        releasePicker();
        clearAnnotations();
        return 'released';
    }

    try {
        window.__refact_overlays = {
            zToolbarHost: Z_TOOLBAR_HOST,
            startPicker: startPicker,
            cancelPicker: releasePicker,
            readPicked: readPicked,
            startAnnotate: startAnnotate,
            readAnnotations: readAnnotations,
            clearAnnotations: clearAnnotations,
            releaseAll: releaseAll
        };
        window.__refact_overlays_blocked = null;
        window.__refact_overlays_installed = true;
    } catch (e) {
        window.__refact_overlays_blocked = 'csp';
    }
})();
