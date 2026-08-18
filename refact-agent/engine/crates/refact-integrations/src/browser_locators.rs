use crate::browser_models::{BrowserTextMode, ElementInfo, FieldKind};

pub const INSPECT_ELEMENT_JS: &str = r#"
if (!window.__refact_inspect_element) {
  window.__refact_inspect_element = function(el, count) {
    var rect = el.getBoundingClientRect();
    var tag = el.tagName.toLowerCase();
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
      visible: rect.width > 0 && rect.height > 0,
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

#[allow(dead_code)]
pub fn detect_field_kind(tag: &str, input_type: Option<&str>, content_editable: bool) -> FieldKind {
    let tag_lower = tag.to_lowercase();
    if content_editable {
        return FieldKind::ContentEditable;
    }
    match tag_lower.as_str() {
        "textarea" => FieldKind::Textarea,
        "select" => FieldKind::Select,
        "input" => match input_type.unwrap_or("text") {
            "text" => FieldKind::TextInput,
            "password" => FieldKind::PasswordInput,
            "email" => FieldKind::EmailInput,
            "search" => FieldKind::SearchInput,
            "number" => FieldKind::NumberInput,
            "tel" => FieldKind::TelInput,
            "url" => FieldKind::UrlInput,
            "date" | "datetime-local" | "month" | "week" | "time" => FieldKind::DateInput,
            "file" => FieldKind::FileInput,
            "hidden" => FieldKind::HiddenInput,
            "checkbox" => FieldKind::Checkbox,
            "radio" => FieldKind::Radio,
            _ => FieldKind::TextInput,
        },
        _ => FieldKind::Unknown,
    }
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

pub fn js_dismiss_overlays() -> &'static str {
    r#"(function() {
  var dismissed = 0;
  var selectors = [
    '[id*="cookie"] button[id*="accept"]',
    '[id*="cookie"] button[id*="agree"]',
    '[class*="cookie"] button[class*="accept"]',
    '[id*="consent"] button[id*="accept"]',
    '[class*="consent"] button[class*="accept"]',
    '[id*="gdpr"] button',
    'button[id*="accept-all"]',
    'button[class*="accept-all"]',
    '#onetrust-accept-btn-handler',
    '.cc-btn.cc-dismiss',
    '[data-testid*="cookie"] button',
    '[data-testid*="accept"]',
    'dialog[open] button[aria-label="Close"]',
    'dialog[open] button[aria-label="Dismiss"]',
    '[role="dialog"] button[aria-label="Close"]',
    '[role="dialog"] button[aria-label="Dismiss"]',
  ];
  selectors.forEach(function(sel) {
    try {
      var btn = document.querySelector(sel);
      if (btn && btn.offsetWidth > 0 && btn.offsetHeight > 0) {
        btn.click();
        dismissed++;
      }
    } catch(e) {}
  });
  var overlays = document.querySelectorAll('[style*="position: fixed"], [style*="position:fixed"]');
  overlays.forEach(function(el) {
    var rect = el.getBoundingClientRect();
    if (rect.width > window.innerWidth * 0.5 && rect.height > window.innerHeight * 0.3) {
      var z = parseInt(window.getComputedStyle(el).zIndex) || 0;
      if (z > 1000) {
        el.remove();
        dismissed++;
      }
    }
  });
  return JSON.stringify({ok: true, dismissed: dismissed});
})()"#
}

pub fn js_dismiss_overlays_probe() -> &'static str {
    r#"(function() {
  var dismissable = 0;
  var selectors = [
    '[id*="cookie"] button[id*="accept"]',
    '[id*="cookie"] button[id*="agree"]',
    '[class*="cookie"] button[class*="accept"]',
    '[id*="consent"] button[id*="accept"]',
    '[class*="consent"] button[class*="accept"]',
    '[id*="gdpr"] button',
    'button[id*="accept-all"]',
    'button[class*="accept-all"]',
    '#onetrust-accept-btn-handler',
    '.cc-btn.cc-dismiss',
    '[data-testid*="cookie"] button',
    '[data-testid*="accept"]',
    'dialog[open] button[aria-label="Close"]',
    'dialog[open] button[aria-label="Dismiss"]',
    '[role="dialog"] button[aria-label="Close"]',
    '[role="dialog"] button[aria-label="Dismiss"]',
  ];
  selectors.forEach(function(sel) {
    try {
      var btn = document.querySelector(sel);
      if (btn && btn.offsetWidth > 0 && btn.offsetHeight > 0) dismissable++;
    } catch(e) {}
  });
  var overlays = document.querySelectorAll('[style*="position: fixed"], [style*="position:fixed"]');
  overlays.forEach(function(el) {
    var rect = el.getBoundingClientRect();
    if (rect.width > window.innerWidth * 0.5 && rect.height > window.innerHeight * 0.3) {
      var z = parseInt(window.getComputedStyle(el).zIndex) || 0;
      if (z > 1000) dismissable++;
    }
  });
  return JSON.stringify({ok: true, dismissable: dismissable});
})()"#
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

#[allow(dead_code)]
pub fn js_detect_blocked_page() -> &'static str {
    r#"(function() {
  var body = document.body ? (document.body.innerText || '').toLowerCase() : '';
  var title = (document.title || '').toLowerCase();
  var status = body.substring(0, 2000);
  var reasons = [];
  if (/access denied|403 forbidden|error 403/i.test(status)) reasons.push('403_forbidden');
  if (/you have been blocked|your ip has been/i.test(status)) reasons.push('ip_blocked');
  if (/please enable javascript|javascript is required/i.test(status)) reasons.push('js_required');
  if (/unusual traffic|automated queries/i.test(status)) reasons.push('bot_detection');
  if (/too many requests|rate limit/i.test(status)) reasons.push('rate_limited');
  if (title.includes('just a moment') || title.includes('attention required')) reasons.push('cloudflare_challenge');
  if (document.querySelector('#challenge-running, #challenge-form, .cf-browser-verification')) reasons.push('cloudflare_challenge');
  if (document.querySelector('[action*="captcha"], #captcha, .g-recaptcha, .h-captcha, [data-sitekey]')) reasons.push('captcha_present');
  return JSON.stringify({ok: true, blocked: reasons.length > 0, reasons: reasons});
})()"#
}

#[allow(dead_code)]
pub fn js_detect_captcha() -> &'static str {
    r#"(function() {
  var types = [];
  if (document.querySelector('.g-recaptcha, [data-sitekey], iframe[src*="recaptcha"]')) types.push('recaptcha');
  if (document.querySelector('.h-captcha, iframe[src*="hcaptcha"]')) types.push('hcaptcha');
  if (document.querySelector('[id*="captcha"], [class*="captcha"]')) types.push('generic_captcha');
  if (document.querySelector('#cf-challenge-running, .cf-browser-verification')) types.push('cloudflare');
  if (document.querySelector('[id*="arkose"], iframe[src*="arkoselabs"]')) types.push('arkose');
  return JSON.stringify({ok: true, captcha: types.length > 0, types: types});
})()"#
}

#[allow(dead_code)]
pub fn js_find_search_input() -> &'static str {
    r#"(function() {
  var candidates = [
    document.querySelector('input[name="q"]'),
    document.querySelector('input[name="search"]'),
    document.querySelector('input[name="query"]'),
    document.querySelector('input[type="search"]'),
    document.querySelector('textarea[name="q"]'),
    document.querySelector('[role="searchbox"]'),
    document.querySelector('[role="combobox"][aria-label]'),
    document.querySelector('input[aria-label*="earch"]'),
  ];
  for (var i = 0; i < candidates.length; i++) {
    var el = candidates[i];
    if (el && el.offsetWidth > 0 && el.offsetHeight > 0) {
      var sel = el.id ? '#' + el.id : (el.name ? '[name="' + el.name + '"]' : el.tagName.toLowerCase());
      return JSON.stringify({ok: true, found: true, selector: sel, name: el.name || '', tag: el.tagName.toLowerCase()});
    }
  }
  return JSON.stringify({ok: true, found: false});
})()"#
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
    fn test_detect_field_kind_text_input() {
        assert_eq!(
            detect_field_kind("input", Some("text"), false),
            FieldKind::TextInput
        );
    }

    #[test]
    fn test_detect_field_kind_password() {
        assert_eq!(
            detect_field_kind("input", Some("password"), false),
            FieldKind::PasswordInput
        );
    }

    #[test]
    fn test_detect_field_kind_email() {
        assert_eq!(
            detect_field_kind("input", Some("email"), false),
            FieldKind::EmailInput
        );
    }

    #[test]
    fn test_detect_field_kind_search() {
        assert_eq!(
            detect_field_kind("input", Some("search"), false),
            FieldKind::SearchInput
        );
    }

    #[test]
    fn test_detect_field_kind_textarea() {
        assert_eq!(
            detect_field_kind("textarea", None, false),
            FieldKind::Textarea
        );
    }

    #[test]
    fn test_detect_field_kind_select() {
        assert_eq!(detect_field_kind("select", None, false), FieldKind::Select);
    }

    #[test]
    fn test_detect_field_kind_content_editable() {
        assert_eq!(
            detect_field_kind("div", None, true),
            FieldKind::ContentEditable
        );
    }

    #[test]
    fn test_detect_field_kind_checkbox() {
        assert_eq!(
            detect_field_kind("input", Some("checkbox"), false),
            FieldKind::Checkbox
        );
    }

    #[test]
    fn test_detect_field_kind_input_default_type() {
        assert_eq!(
            detect_field_kind("input", None, false),
            FieldKind::TextInput
        );
    }

    #[test]
    fn test_detect_field_kind_unknown_tag() {
        assert_eq!(detect_field_kind("span", None, false), FieldKind::Unknown);
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
        let js = js_dismiss_overlays();
        assert!(js.contains("cookie"));
        assert!(js.contains("consent"));
        assert!(js.contains("dismissed"));
    }

    #[test]
    fn dismiss_overlays_probe_keeps_the_battle_tested_selectors() {
        let probe = js_dismiss_overlays_probe();

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

    #[test]
    fn test_js_detect_blocked_page_valid_js() {
        let js = js_detect_blocked_page();
        assert!(js.starts_with("(function()"));
        assert!(js.contains("403"));
        assert!(js.contains("cloudflare"));
        assert!(js.contains("captcha"));
        assert!(js.contains("JSON.stringify"));
    }

    #[test]
    fn test_js_detect_captcha_valid_js() {
        let js = js_detect_captcha();
        assert!(js.starts_with("(function()"));
        assert!(js.contains("recaptcha"));
        assert!(js.contains("hcaptcha"));
        assert!(js.contains("cloudflare"));
        assert!(js.contains("arkose"));
    }

    #[test]
    fn test_js_find_search_input_valid_js() {
        let js = js_find_search_input();
        assert!(js.starts_with("(function()"));
        assert!(js.contains("name=\"q\""));
        assert!(js.contains("type=\"search\""));
        assert!(js.contains("searchbox"));
    }
}
