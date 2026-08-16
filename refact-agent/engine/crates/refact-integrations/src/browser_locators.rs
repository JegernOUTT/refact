use crate::browser_models::{BrowserLocator, ElementInfo, FieldKind, LocatorStrategy};

pub fn to_css_selector(locator: &BrowserLocator) -> Option<String> {
    if locator.locator.is_some()
        || locator.filter.is_some()
        || locator.and.is_some()
        || locator.or.is_some()
        || locator.first.is_some()
        || locator.last.is_some()
    {
        return None;
    }
    let base = match &locator.strategy {
        LocatorStrategy::Css { value } => value.clone(),
        LocatorStrategy::Id { value } => format!("#{}", css_escape_ident(value)),
        LocatorStrategy::Name { value } => format!("[name={}]", css_escape_attr_value(value)),
        LocatorStrategy::TestId { value, exact: None, regex: None, attribute: None } => {
            format!("[data-testid={}]", css_escape_attr_value(value))
        }
        LocatorStrategy::Placeholder { value, exact: None, regex: None } => {
            format!("[placeholder={}]", css_escape_attr_value(value))
        }
        LocatorStrategy::Autocomplete { value } => {
            format!("[autocomplete={}]", css_escape_attr_value(value))
        }
        LocatorStrategy::Role {
            role, name: None, description: None, exact: None, name_regex: None,
            description_regex: None, checked: None, pressed: None, selected: None,
            expanded: None, disabled: None, level: None, include_hidden: None,
        } => {
            format!("[role={}]", css_escape_attr_value(role))
        }
        LocatorStrategy::Text { .. } => return None,
        LocatorStrategy::Label { .. } => return None,
        LocatorStrategy::AltText { .. } => return None,
        LocatorStrategy::Title { .. } => return None,
        LocatorStrategy::TestId { .. } => return None,
        LocatorStrategy::Placeholder { .. } => return None,
        LocatorStrategy::Role { .. } => return None,
        LocatorStrategy::Xpath { .. } => return None,
    };

    if let Some(within) = &locator.within {
        Some(format!("{} {}", within, base))
    } else {
        Some(base)
    }
}

fn css_escape_ident(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 8);
    let mut chars = s.chars().enumerate().peekable();
    while let Some((i, ch)) = chars.next() {
        if ch == '\0' {
            result.push_str("\\fffd ");
        } else if ch.is_ascii_digit() && i == 0 {
            result.push_str(&format!("\\{:x} ", ch as u32));
        } else if i == 0 && ch == '-' {
            if let Some((_, next_ch)) = chars.peek().copied() {
                if next_ch.is_ascii_digit() {
                    result.push('-');
                    result.push_str(&format!("\\{:x} ", next_ch as u32));
                    chars.next();
                    continue;
                }
            }
            result.push('-');
        } else if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || (ch as u32) >= 0x80 {
            result.push(ch);
        } else if (ch as u32) < 0x20 || ch == '\x7f' {
            result.push_str(&format!("\\{:x} ", ch as u32));
        } else {
            result.push('\\');
            result.push(ch);
        }
    }
    result
}

fn css_escape_attr_value(s: &str) -> String {
    let escaped = s
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\a ");
    format!("\"{}\"", escaped)
}

pub fn generate_resolve_js(locator: &BrowserLocator) -> String {
    if locator.locator.is_some()
        || locator.filter.is_some()
        || locator.and.is_some()
        || locator.or.is_some()
        || locator.first.is_some()
        || locator.last.is_some()
        || locator.nth.is_some_and(|index| index < 0)
    {
        return injected_browser_locator_find_js(locator, true);
    }
    let find_code = generate_find_js(&locator.strategy);
    let scope_code = match &locator.within {
        Some(sel) => format!(
            "var scope = document.querySelector({});\n\
             if (!scope) return JSON.stringify({{error: 'Scope selector not found', selector: {}}});",
            js_string_literal(sel),
            js_string_literal(sel),
        ),
        None => "var scope = document;".to_string(),
    };
    let nth_code = match locator.nth {
        Some(n) => format!(
            "if (elements.length > {n}) {{ elements = [elements[{n}]]; }}\n\
             else if (elements.length <= {n}) {{ elements = []; }}"
        ),
        None => String::new(),
    };

    format!(
        r#"(function() {{
  {scope_code}
  {find_code}
  {nth_code}
  if (elements.length === 0) {{
    throw new Error('Element not found');
  }}
  return elements[0];
}})()"#,
    )
}

pub fn generate_find_fragment_js(locator: &BrowserLocator) -> String {
    if locator.locator.is_some()
        || locator.filter.is_some()
        || locator.and.is_some()
        || locator.or.is_some()
        || locator.first.is_some()
        || locator.last.is_some()
        || locator.nth.is_some_and(|index| index < 0)
    {
        return injected_browser_locator_find_js(locator, false);
    }
    let find_code = generate_find_js(&locator.strategy);
    let nth_code = match locator.nth {
        Some(n) => format!(
            "if (elements.length > {n}) {{ elements = [elements[{n}]]; }}\n\
             else if (elements.length <= {n}) {{ elements = []; }}"
        ),
        None => String::new(),
    };
    match &locator.within {
        Some(sel) => format!(
            "var elements = [];\n\
             var scope = document.querySelector({});\n\
             if (scope) {{\n  {find_code}\n  {nth_code}\n}}",
            js_string_literal(sel),
        ),
        None => format!("var scope = document;\n  {find_code}\n  {nth_code}"),
    }
}

fn injected_browser_locator_find_js(locator: &BrowserLocator, return_element: bool) -> String {
    let locator = serde_json::to_string(locator).expect("browser locators serialize");
    let tail = if return_element { "return elements[0];" } else { "" };
    format!(
        "var injected = globalThis.__refact_injected__;\n\
         if (!injected) throw new Error('RefactInjected is not installed');\n\
         var elements = injected.resolveAll({locator});\n\
         {tail}"
    )
}

#[cfg(test)]
fn resolution_transport_snapshot(locator: &BrowserLocator) -> String {
    generate_resolve_js(locator).replace(
        "  return elements[0];",
        "  var el = elements[0];\n  return el;",
    )
}

fn generate_find_js(strategy: &LocatorStrategy) -> String {
    match strategy {
        LocatorStrategy::Css { value } => {
            format!(
                "var elements = Array.from(scope.querySelectorAll({}));",
                js_string_literal(value)
            )
        }
        LocatorStrategy::Id { value } => {
            format!(
                "var el = scope.querySelector('#' + CSS.escape({}));\n\
                 var elements = el ? [el] : [];",
                js_string_literal(value)
            )
        }
        LocatorStrategy::Name { value } => {
            format!(
                "var elements = Array.from(scope.querySelectorAll('[name=' + JSON.stringify({}) + ']'));",
                js_string_literal(value)
            )
        }
        LocatorStrategy::TestId { .. } => injected_locator_find_js(strategy),
        LocatorStrategy::Placeholder { .. } => injected_locator_find_js(strategy),
        LocatorStrategy::AltText { .. } => injected_locator_find_js(strategy),
        LocatorStrategy::Title { .. } => injected_locator_find_js(strategy),
        LocatorStrategy::Autocomplete { value } => {
            format!(
                "var elements = Array.from(scope.querySelectorAll('[autocomplete=' + JSON.stringify({}) + ']'));",
                js_string_literal(value)
            )
        }
        LocatorStrategy::Text { .. } => injected_locator_find_js(strategy),
        LocatorStrategy::Label { .. } => injected_locator_find_js(strategy),
        LocatorStrategy::Role { .. } => injected_locator_find_js(strategy),
        LocatorStrategy::Xpath { value } => {
            format!(
                "var xpathResult = document.evaluate({}, scope, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null);\n\
                 var elements = [];\n\
                 for (var i = 0; i < xpathResult.snapshotLength; i++) {{\n\
                   elements.push(xpathResult.snapshotItem(i));\n\
                 }}",
                js_string_literal(value),
            )
        }
    }
}

fn injected_locator_find_js(strategy: &LocatorStrategy) -> String {
    let locator = serde_json::to_string(strategy).expect("locator strategies serialize");
    format!(
        "var injected = globalThis.__refact_injected__;\n\
     if (!injected) throw new Error('RefactInjected is not installed');\n\
     var elements = injected.resolveAll({locator}, scope);"
    )
}

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
    r#"(function() {
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
})()"#
}

pub fn js_hover_element() -> &'static str {
    r#"(function() {
  var el = this;
  if (!el) return JSON.stringify({error: 'No resolved element'});
  el.scrollIntoView({block: 'center', behavior: 'instant'});
  el.dispatchEvent(new MouseEvent('mouseover', {bubbles: true}));
  el.dispatchEvent(new MouseEvent('mouseenter', {bubbles: true}));
  return JSON.stringify({ok: true});
})()"#
}

pub fn js_focus_element() -> &'static str {
    r#"(function() {
  var el = this;
  if (!el) return JSON.stringify({error: 'No resolved element'});
  el.scrollIntoView({block: 'center', behavior: 'instant'});
  el.focus();
  return JSON.stringify({ok: true});
})()"#
}

pub fn js_blur_element() -> &'static str {
    r#"(function() {
  var el = this;
  if (!el) return JSON.stringify({error: 'No resolved element'});
  el.blur();
  return JSON.stringify({ok: true});
})()"#
}

pub fn js_scroll_to_element() -> &'static str {
    r#"(function() {
  var el = this;
  if (!el) return JSON.stringify({error: 'No resolved element'});
  el.scrollIntoView({block: 'center', behavior: 'smooth'});
  return JSON.stringify({ok: true});
})()"#
}

pub fn js_get_text() -> &'static str {
    r#"(function() {
  var el = this;
  if (!el) return JSON.stringify({error: 'No resolved element'});
  return JSON.stringify({ok: true, text: el.innerText || ''});
})()"#
}

pub fn js_get_html() -> &'static str {
    r#"(function() {
  var el = this;
  if (!el) return JSON.stringify({error: 'No resolved element'});
  var html = el.outerHTML;
  if (html.length > 5000) html = html.substring(0, 5000) + '... (truncated)';
  return JSON.stringify({ok: true, html: html});
})()"#
}

pub fn js_get_attribute(attribute: &str) -> String {
    format!(
        r#"(function() {{
  var el = this;
  if (!el) return JSON.stringify({{error: 'No resolved element'}});
  var val = el.getAttribute({});
  return JSON.stringify({{ok: true, value: val}});
}})()"#,
        js_string_literal(attribute),
    )
}

pub fn js_extract_links(limit: usize) -> String {
    format!(
        r#"(function() {{
  var scope = this || document;
  var anchors = Array.from(scope.querySelectorAll('a[href]'));
  var links = anchors.slice(0, {limit}).map(function(a) {{
    return {{url: a.href, text: (a.innerText || '').trim().substring(0, 200)}};
  }});
  return JSON.stringify({{ok: true, links: links, total: anchors.length}});
}})()"#,
    )
}

pub fn js_extract_table() -> &'static str {
    r#"(function() {
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
})()"#
}

pub fn js_highlight_element() -> &'static str {
    r#"(function() {
  var el = this;
  if (!el) return JSON.stringify({error: 'No resolved element'});
  el.style.outline = '3px solid #E7150D';
  el.style.outlineOffset = '2px';
  setTimeout(function() { el.style.outline = ''; el.style.outlineOffset = ''; }, 3000);
  return JSON.stringify({ok: true});
})()"#
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

pub fn js_check_selector_present(css: &str) -> String {
    format!(
        r#"(function() {{
  return !!document.querySelector({});
}})()"#,
        js_string_literal(css),
    )
}

pub fn js_check_element_hidden(css: &str) -> String {
    format!(
        r#"(function() {{
  var el = document.querySelector({});
  if (!el) return true;
  var rect = el.getBoundingClientRect();
  return rect.width === 0 || rect.height === 0;
}})()"#,
        js_string_literal(css),
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
    fn test_css_from_css_locator() {
        let loc = BrowserLocator::css("button.submit");
        assert_eq!(to_css_selector(&loc), Some("button.submit".to_string()));
    }

    #[test]
    fn test_css_from_id_locator() {
        let loc = BrowserLocator::id("email");
        assert_eq!(to_css_selector(&loc), Some("#email".to_string()));
    }

    #[test]
    fn test_css_from_id_needs_escape() {
        let loc = BrowserLocator::id("my.field");
        let css = to_css_selector(&loc).unwrap();
        assert_eq!(css, "#my\\.field");
    }

    #[test]
    fn test_css_from_name_locator() {
        let loc = BrowserLocator::name("q");
        let css = to_css_selector(&loc).unwrap();
        assert_eq!(css, "[name=\"q\"]");
    }

    #[test]
    fn test_css_from_testid_locator() {
        let loc = BrowserLocator::test_id("login-btn");
        let css = to_css_selector(&loc).unwrap();
        assert_eq!(css, "[data-testid=\"login-btn\"]");
    }

    #[test]
    fn test_css_from_placeholder_locator() {
        let loc = BrowserLocator::placeholder("Search...");
        let css = to_css_selector(&loc).unwrap();
        assert_eq!(css, "[placeholder=\"Search...\"]");
    }

    #[test]
    fn test_css_from_autocomplete_locator() {
        let loc = BrowserLocator {
            strategy: LocatorStrategy::Autocomplete {
                value: "email".to_string(),
            },
            nth: None,
            within: None,
            locator: None,
            filter: None,
            and: None,
            or: None,
            first: None,
            last: None,
        };
        let css = to_css_selector(&loc).unwrap();
        assert_eq!(css, "[autocomplete=\"email\"]");
    }

    #[test]
    fn test_css_from_role_without_name() {
        let loc = BrowserLocator::role("button", None);
        let css = to_css_selector(&loc).unwrap();
        assert_eq!(css, "[role=\"button\"]");
    }

    #[test]
    fn test_css_from_role_with_name_returns_none() {
        let loc = BrowserLocator::role("textbox", Some("Search"));
        assert_eq!(to_css_selector(&loc), None);
    }

    #[test]
    fn test_css_from_text_returns_none() {
        let loc = BrowserLocator {
            strategy: LocatorStrategy::Text {
                value: "Submit".to_string(),
                exact: false,
                regex: None,
            },
            nth: None,
            within: None,
            locator: None,
            filter: None,
            and: None,
            or: None,
            first: None,
            last: None,
        };
        assert_eq!(to_css_selector(&loc), None);
    }

    #[test]
    fn test_css_from_label_returns_none() {
        let loc = BrowserLocator::label("Email");
        assert_eq!(to_css_selector(&loc), None);
    }

    #[test]
    fn test_css_from_xpath_returns_none() {
        let loc = BrowserLocator {
            strategy: LocatorStrategy::Xpath {
                value: "//button".to_string(),
            },
            nth: None,
            within: None,
            locator: None,
            filter: None,
            and: None,
            or: None,
            first: None,
            last: None,
        };
        assert_eq!(to_css_selector(&loc), None);
    }

    #[test]
    fn test_css_with_within_scope() {
        let loc = BrowserLocator {
            strategy: LocatorStrategy::Name {
                value: "q".to_string(),
            },
            nth: None,
            within: Some("#search-form".to_string()),
            locator: None,
            filter: None,
            and: None,
            or: None,
            first: None,
            last: None,
        };
        let css = to_css_selector(&loc).unwrap();
        assert_eq!(css, "#search-form [name=\"q\"]");
    }

    #[test]
    fn test_css_escape_ident_simple() {
        assert_eq!(css_escape_ident("myId"), "myId");
    }

    #[test]
    fn test_css_escape_ident_with_dot() {
        assert_eq!(css_escape_ident("my.field"), "my\\.field");
    }

    #[test]
    fn test_css_escape_ident_starts_with_digit() {
        assert_eq!(css_escape_ident("1st"), "\\31 st");
    }

    #[test]
    fn test_css_escape_ident_starts_with_dash_digit() {
        assert_eq!(css_escape_ident("-1st"), "-\\31 st");
    }

    #[test]
    fn test_css_escape_ident_digit_only_first() {
        assert_eq!(css_escape_ident("a1b"), "a1b");
    }

    #[test]
    fn test_css_escape_ident_unicode_preserved() {
        assert_eq!(css_escape_ident("café"), "café");
    }

    #[test]
    fn test_css_escape_attr_value_simple() {
        assert_eq!(css_escape_attr_value("hello"), "\"hello\"");
    }

    #[test]
    fn test_css_escape_attr_value_with_quotes() {
        assert_eq!(
            css_escape_attr_value("it's \"here\""),
            "\"it's \\\"here\\\"\""
        );
    }

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
    fn test_generate_resolve_js_css() {
        let loc = BrowserLocator::css("#email");
        let js = generate_resolve_js(&loc);
        assert!(js.contains("querySelectorAll"));
        assert!(js.contains("#email"));
        assert!(js.contains("return elements[0]"));
    }

    #[test]
    fn test_generate_resolve_js_label() {
        let loc = BrowserLocator::label("Email Address");
        let js = generate_resolve_js(&loc);
        assert!(js.contains("resolveAll"));
        assert!(js.contains("Email Address"));
    }

    #[test]
    fn test_generate_resolve_js_text() {
        let loc = BrowserLocator {
            strategy: LocatorStrategy::Text {
                value: "Submit".to_string(),
                exact: true,
                regex: None,
            },
            nth: None,
            within: None,
            locator: None,
            filter: None,
            and: None,
            or: None,
            first: None,
            last: None,
        };
        let js = generate_resolve_js(&loc);
        assert!(js.contains("resolveAll"));
        assert!(js.contains("\"exact\":true"));
    }

    #[test]
    fn test_generate_resolve_js_text_substring() {
        let loc = BrowserLocator {
            strategy: LocatorStrategy::Text {
                value: "Sub".to_string(),
                exact: false,
                regex: None,
            },
            nth: None,
            within: None,
            locator: None,
            filter: None,
            and: None,
            or: None,
            first: None,
            last: None,
        };
        let js = generate_resolve_js(&loc);
        assert!(js.contains("resolveAll"));
        assert!(js.contains("\"value\":\"Sub\""));
    }

    #[test]
    fn test_generate_resolve_js_xpath() {
        let loc = BrowserLocator {
            strategy: LocatorStrategy::Xpath {
                value: "//button[@type='submit']".to_string(),
            },
            nth: None,
            within: None,
            locator: None,
            filter: None,
            and: None,
            or: None,
            first: None,
            last: None,
        };
        let js = generate_resolve_js(&loc);
        assert!(js.contains("document.evaluate"));
        assert!(js.contains("XPathResult"));
    }

    #[test]
    fn test_generate_resolve_js_with_nth() {
        let loc = BrowserLocator {
            strategy: LocatorStrategy::Css {
                value: "input".to_string(),
            },
            nth: Some(2),
            within: None,
            locator: None,
            filter: None,
            and: None,
            or: None,
            first: None,
            last: None,
        };
        let js = generate_resolve_js(&loc);
        assert!(js.contains("elements[2]"));
    }

    #[test]
    fn test_generate_resolve_js_with_within() {
        let loc = BrowserLocator {
            strategy: LocatorStrategy::Css {
                value: "input".to_string(),
            },
            nth: None,
            within: Some("#form".to_string()),
            locator: None,
            filter: None,
            and: None,
            or: None,
            first: None,
            last: None,
        };
        let js = generate_resolve_js(&loc);
        assert!(js.contains("querySelector"));
        assert!(js.contains("#form"));
        assert!(js.contains("Scope selector not found"));
    }

    #[test]
    fn locator_generation_is_unchanged_before_handle_return() {
        for locator in [
            BrowserLocator::css("#email"),
            BrowserLocator::label("Email Address"),
            BrowserLocator::role("button", Some("Submit")),
        ] {
            let generated = generate_resolve_js(&locator);
            let snapshot = resolution_transport_snapshot(&locator);
            let prefix = generated.split("  return elements[0];").next().unwrap();
            assert!(snapshot.starts_with(prefix));
            assert!(generated.ends_with("})()"));
        }
    }

    #[test]
    fn test_generate_resolve_js_role_with_name() {
        let loc = BrowserLocator::role("textbox", Some("Search"));
        let js = generate_resolve_js(&loc);
        assert!(js.contains("resolveAll"));
        assert!(js.contains("\"role\":\"textbox\""));
        assert!(js.contains("Search"));
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
    }

    #[test]
    fn test_js_check_text_present() {
        let js = js_check_text_present("Hello World");
        assert!(js.contains("includes(target)"));
        assert!(js.contains("Hello World"));
    }

    #[test]
    fn test_js_check_selector_present() {
        let js = js_check_selector_present("#main");
        assert!(js.contains("querySelector"));
        assert!(js.contains("#main"));
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
