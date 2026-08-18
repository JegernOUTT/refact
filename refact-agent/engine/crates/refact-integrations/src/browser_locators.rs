use crate::browser_models::{BrowserTextMode, ElementInfo};

pub const INSPECT_ELEMENT_JS: &str = r#"
if (!window.__refact_inspect_element) {
  window.__refact_inspect_element = function(el, count) {
    var rect = el.getBoundingClientRect();
    var tag = el.tagName.toLowerCase();
    var visibility = window.getComputedStyle(el).visibility;
    var inputType = (tag === 'input') ? (el.type || 'text').toLowerCase() : null;
    var fieldKind = 'unknown';
    if (tag === 'textarea') { fieldKind = 'textarea'; }
    else if (tag === 'select') { fieldKind = 'select'; }
    else if (el.isContentEditable) { fieldKind = 'content_editable'; }
    else if (tag === 'input') {
      var typeMap = {
        'text': 'text_input', 'password': 'password_input',
        'email': 'email_input', 'search': 'search_input',
        'number': 'number_input', 'tel': 'tel_input',
        'url': 'url_input', 'date': 'date_input',
        'datetime-local': 'date_input', 'month': 'date_input',
        'week': 'date_input', 'time': 'date_input',
        'file': 'file_input', 'hidden': 'hidden_input',
        'checkbox': 'checkbox', 'radio': 'radio'
      };
      fieldKind = typeMap[inputType] || 'text_input';
    }
    return {
      found: true, count: count,
      tag: tag, input_type: inputType,
      id: el.id || null, name: el.name || null,
      placeholder: el.placeholder || null,
      aria_label: el.getAttribute('aria-label') || null,
      role: el.getAttribute('role') || null,
      visible: rect.width > 0 && rect.height > 0 && visibility !== 'hidden' && visibility !== 'collapse',
      enabled: !el.disabled,
      readonly: !!el.readOnly,
      content_editable: !!el.isContentEditable,
      value: (el.value !== undefined) ? String(el.value) : null,
      inner_text: (el.innerText || '').substring(0, 500),
      bbox: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
      field_kind: fieldKind
    };
  };
}
"#;

pub fn parse_element_info(json_str: &str) -> Result<ElementInfo, String> {
    let value: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| format!("Invalid JSON from browser: {}", e))?;

    if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
        return Err(err.to_string());
    }

    serde_json::from_value(value).map_err(|e| format!("Failed to parse ElementInfo: {}", e))
}

pub fn js_click_element() -> &'static str {
    r#"function() {
  var el = this;
  if (!el) return JSON.stringify({error: 'No resolved element'});
  el.scrollIntoView({block: 'center', behavior: 'instant'});
  var rect = el.getBoundingClientRect();
  var cx = rect.left + rect.width / 2;
  var cy = rect.top + rect.height / 2;
  var opts = {bubbles: true, cancelable: true, view: window, clientX: cx, clientY: cy, button: 0};
  var events = ['pointerover', 'pointerenter', 'pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click'];
  for (var i = 0; i < events.length; i++) {
    var type = events[i];
    var ev;
    if (type.indexOf('pointer') === 0 && typeof PointerEvent === 'function') {
      ev = new PointerEvent(type, opts);
    } else {
      ev = new MouseEvent(type, opts);
    }
    el.dispatchEvent(ev);
  }
  return JSON.stringify({ok: true});
}"#
}

pub fn js_hover_element() -> &'static str {
    r#"function() {
  var el = this;
  if (!el) return JSON.stringify({error: 'No resolved element'});
  el.scrollIntoView({block: 'center', behavior: 'instant'});
  el.dispatchEvent(new MouseEvent('mouseover', {bubbles: true}));
  el.dispatchEvent(new MouseEvent('mouseenter', {bubbles: true}));
  return JSON.stringify({ok: true});
}"#
}

pub fn js_focus_element() -> &'static str {
    r#"function() {
  var el = this;
  if (!el) return JSON.stringify({error: 'No resolved element'});
  el.scrollIntoView({block: 'center', behavior: 'instant'});
  el.focus();
  return JSON.stringify({ok: true});
}"#
}

pub fn js_blur_element() -> &'static str {
    r#"function() {
  var el = this;
  if (!el) return JSON.stringify({error: 'No resolved element'});
  el.blur();
  return JSON.stringify({ok: true});
}"#
}

pub fn js_scroll_to_element() -> &'static str {
    r#"function() {
  var el = this;
  if (!el) return JSON.stringify({error: 'No resolved element'});
  el.scrollIntoView({block: 'center', behavior: 'smooth'});
  return JSON.stringify({ok: true});
}"#
}

pub fn js_get_text() -> &'static str {
    r#"function() {
  var el = this;
  if (!el) return JSON.stringify({error: 'No resolved element'});
  return JSON.stringify({ok: true, text: el.innerText || ''});
}"#
}

pub fn js_get_html() -> &'static str {
    r#"function() {
  var el = this;
  if (!el) return JSON.stringify({error: 'No resolved element'});
  var html = el.outerHTML;
  if (html.length > 5000) html = html.substring(0, 5000) + '... (truncated)';
  return JSON.stringify({ok: true, html: html});
}"#
}

pub fn js_get_attribute(attribute: &str) -> String {
    format!(
        r#"function() {{
  var el = this;
  if (!el) return JSON.stringify({{error: 'No resolved element'}});
  var val = el.getAttribute({});
  return JSON.stringify({{ok: true, value: val}});
}}"#,
        js_string_literal(attribute),
    )
}

pub fn js_input_value() -> &'static str {
    r#"function() {
  var el = this;
  if (!el) return JSON.stringify({error: 'No resolved element'});
  var tag = el.tagName.toLowerCase();
  if (tag !== 'input' && tag !== 'textarea' && tag !== 'select') {
    return JSON.stringify({error: 'Node is not an <input>, <textarea> or <select> element'});
  }
  return JSON.stringify({ok: true, value: String(el.value)});
}"#
}

pub fn js_element_text(mode: BrowserTextMode) -> &'static str {
    match mode {
        BrowserTextMode::InnerText => {
            r#"function() {
  var el = this;
  if (!el) return JSON.stringify({error: 'No resolved element'});
  return JSON.stringify({ok: true, text: el.innerText || ''});
}"#
        }
        BrowserTextMode::TextContent => {
            r#"function() {
  var el = this;
  if (!el) return JSON.stringify({error: 'No resolved element'});
  return JSON.stringify({ok: true, text: el.textContent || ''});
}"#
        }
    }
}

pub fn js_extract_links(limit: usize) -> String {
    format!(
        r#"(function() {{
  var scope = document;
  var anchors = Array.from(scope.querySelectorAll('a[href]'));
  var links = anchors.slice(0, {limit}).map(function(a) {{
    return {{url: a.href, text: (a.innerText || '').trim().substring(0, 200)}};
  }});
  return JSON.stringify({{ok: true, links: links, total: anchors.length}});
}})()"#,
    )
}

pub fn js_extract_table() -> &'static str {
    r#"function() {
  var el = this;
  if (!el) return JSON.stringify({error: 'No resolved element'});
  var table = (el.tagName === 'TABLE') ? el : el.querySelector('table');
  if (!table) return JSON.stringify({error: 'No table found'});
  var rows = Array.from(table.rows);
  var data = rows.slice(0, 100).map(function(row) {
    return Array.from(row.cells).map(function(cell) {
      return (cell.innerText || '').trim().substring(0, 500);
    });
  });
  return JSON.stringify({ok: true, rows: data, total_rows: rows.length});
}"#
}

pub fn js_highlight_element() -> &'static str {
    r#"function() {
  var el = this;
  if (!el) return JSON.stringify({error: 'No resolved element'});
  el.style.outline = '3px solid #E7150D';
  el.style.outlineOffset = '2px';
  setTimeout(function() { el.style.outline = ''; el.style.outlineOffset = ''; }, 3000);
  return JSON.stringify({ok: true});
}"#
}

pub const DISMISS_OVERLAY_SELECTORS: [&str; 16] = [
    "[id*=\"cookie\"] button[id*=\"accept\"]",
    "[id*=\"cookie\"] button[id*=\"agree\"]",
    "[class*=\"cookie\"] button[class*=\"accept\"]",
    "[id*=\"consent\"] button[id*=\"accept\"]",
    "[class*=\"consent\"] button[class*=\"accept\"]",
    "[id*=\"gdpr\"] button",
    "button[id*=\"accept-all\"]",
    "button[class*=\"accept-all\"]",
    "#onetrust-accept-btn-handler",
    ".cc-btn.cc-dismiss",
    "[data-testid*=\"cookie\"] button",
    "[data-testid*=\"accept\"]",
    "dialog[open] button[aria-label=\"Close\"]",
    "dialog[open] button[aria-label=\"Dismiss\"]",
    "[role=\"dialog\"] button[aria-label=\"Close\"]",
    "[role=\"dialog\"] button[aria-label=\"Dismiss\"]",
];

const DISMISS_OVERLAY_REMOVAL_JS: &str = r#"
  var overlays = document.querySelectorAll('[style*="position: fixed"], [style*="position:fixed"]');
  overlays.forEach(function(el) {
    var rect = el.getBoundingClientRect();
    if (rect.width > window.innerWidth * 0.5 && rect.height > window.innerHeight * 0.3) {
      var z = parseInt(window.getComputedStyle(el).zIndex) || 0;
      if (z > 1000) {
        if (!dryRun) el.remove();
        count++;
        removed++;
      }
    }
  });"#;

pub fn js_dismiss_overlays(dry_run: bool, aggressive: bool) -> String {
    let selectors = DISMISS_OVERLAY_SELECTORS
        .iter()
        .map(|selector| js_string_literal(selector))
        .collect::<Vec<_>>()
        .join(", ");
    let removal = if aggressive {
        DISMISS_OVERLAY_REMOVAL_JS
    } else {
        ""
    };
    format!(
        r#"(function() {{
  var dryRun = {dry_run};
  var count = 0;
  var removed = 0;
  var selectors = [{selectors}];
  selectors.forEach(function(sel) {{
    try {{
      var btn = document.querySelector(sel);
      if (btn && btn.offsetWidth > 0 && btn.offsetHeight > 0) {{
        if (!dryRun) btn.click();
        count++;
      }}
    }} catch(e) {{}}
  }});{removal}
  return JSON.stringify({{ok: true, count: count, removed: removed}});
}})()"#
    )
}

pub fn js_check_text_present(text: &str) -> String {
    format!(
        r#"(function() {{
  var target = {};
  return document.body && document.body.innerText && document.body.innerText.includes(target);
}})()"#,
        js_string_literal(text),
    )
}

pub fn js_string_literal(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 8);
    result.push('\'');
    for ch in s.chars() {
        match ch {
            '\'' => result.push_str("\\'"),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            '\0' => result.push_str("\\0"),
            _ => result.push(ch),
        }
    }
    result.push('\'');
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_models::FieldKind;

    #[test]
    fn test_js_string_literal_simple() {
        assert_eq!(js_string_literal("hello"), "'hello'");
    }

    #[test]
    fn test_js_string_literal_with_quotes() {
        assert_eq!(js_string_literal("it's"), "'it\\'s'");
    }

    #[test]
    fn test_js_string_literal_with_backslash() {
        assert_eq!(js_string_literal("a\\b"), "'a\\\\b'");
    }

    #[test]
    fn test_js_string_literal_with_newline() {
        assert_eq!(js_string_literal("a\nb"), "'a\\nb'");
    }

    #[test]
    fn test_js_string_literal_complex_selector() {
        let s = "button[data-testid='submit'], .form-submit";
        let lit = js_string_literal(s);
        assert_eq!(lit, "'button[data-testid=\\'submit\\'], .form-submit'");
    }

    #[test]
    fn test_parse_element_info_success() {
        let json = r#"{
            "found": true, "count": 1,
            "tag": "input", "input_type": "text",
            "id": "email", "name": "email",
            "placeholder": "Enter email",
            "aria_label": null, "role": null,
            "visible": true, "enabled": true,
            "readonly": false, "content_editable": false,
            "value": "", "inner_text": null,
            "bbox": {"x": 10, "y": 20, "width": 300, "height": 40},
            "field_kind": "text_input"
        }"#;
        let info = parse_element_info(json).unwrap();
        assert_eq!(info.tag, "input");
        assert_eq!(info.field_kind, FieldKind::TextInput);
        assert!(info.visible);
    }

    #[test]
    fn test_parse_element_info_error() {
        let json = r#"{"error": "Element not found", "count": 0}"#;
        let result = parse_element_info(json);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Element not found"));
    }

    #[test]
    fn test_parse_element_info_invalid_json() {
        let result = parse_element_info("not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_js_click_element_valid_js() {
        let js = js_click_element();
        assert!(js.contains("var el = this"));
        assert!(js.contains("scrollIntoView"));
        assert!(js.contains("dispatchEvent"));
        assert!(js.contains("pointerdown"));
        assert!(js.contains("mouseup"));
        assert!(js.contains("'click'"));
    }

    #[test]
    fn test_js_dismiss_overlays_valid_js() {
        let js = js_dismiss_overlays(false, false);
        assert!(js.contains("cookie"));
        assert!(js.contains("consent"));
        assert!(js.contains("count"));
    }

    #[test]
    fn the_default_dismiss_overlays_pass_never_removes_an_element() {
        let default_pass = js_dismiss_overlays(false, false);
        assert!(!default_pass.contains("el.remove()"));
        assert!(default_pass.contains("if (!dryRun) btn.click();"));

        let aggressive = js_dismiss_overlays(false, true);
        assert!(aggressive.contains("if (!dryRun) el.remove();"));

        let probe = js_dismiss_overlays(true, false);
        assert!(probe.contains("var dryRun = true;"));
        assert!(!probe.contains("el.remove()"));
    }

    #[test]
    fn dismiss_overlays_keeps_the_battle_tested_selectors() {
        let probe = js_dismiss_overlays(true, true);

        for selector in [
            "#onetrust-accept-btn-handler",
            ".cc-btn.cc-dismiss",
            "button[id*=\"accept-all\"]",
            "dialog[open] button[aria-label=\"Close\"]",
            "[role=\"dialog\"] button[aria-label=\"Dismiss\"]",
            "[style*=\"position: fixed\"]",
        ] {
            assert!(probe.contains(selector), "missing selector {selector}");
        }
    }

    #[test]
    fn test_js_extract_links_limit() {
        let js = js_extract_links(5);
        assert!(js.contains(".slice(0, 5)"));
        assert!(js.contains("var scope = document"));
    }

    #[test]
    fn handle_bound_scripts_are_function_declarations() {
        for script in [
            js_click_element().to_string(),
            js_hover_element().to_string(),
            js_focus_element().to_string(),
            js_blur_element().to_string(),
            js_scroll_to_element().to_string(),
            js_get_text().to_string(),
            js_get_html().to_string(),
            js_get_attribute("href"),
            js_input_value().to_string(),
            js_element_text(BrowserTextMode::InnerText).to_string(),
            js_element_text(BrowserTextMode::TextContent).to_string(),
            js_extract_table().to_string(),
            js_highlight_element().to_string(),
        ] {
            assert!(script.trim_start().starts_with("function()"));
            assert!(!script.trim_end().ends_with(")()"));
        }
    }

    #[test]
    fn test_js_check_text_present() {
        let js = js_check_text_present("Hello World");
        assert!(js.contains("includes(target)"));
        assert!(js.contains("Hello World"));
    }

    #[test]
    fn test_js_get_attribute() {
        let js = js_get_attribute("href");
        assert!(js.contains("getAttribute"));
        assert!(js.contains("href"));
    }
}
