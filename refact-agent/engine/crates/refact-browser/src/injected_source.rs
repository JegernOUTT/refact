pub const INJECTED_BUNDLE: &str = include_str!("generated/injected_bundle.js");

pub fn wrapped_bootstrap() -> String {
    format!(
        r#"(() => {{
const __refact_global = globalThis;
const __refact_Array = __refact_global.Array;
const __refact_Object = __refact_global.Object;
const __refact_JSON = __refact_global.JSON;
const __refact_performance = __refact_global.performance;
const __refact_Document = __refact_global.Document;
const __refact_ShadowRoot = __refact_global.ShadowRoot;
const __refact_EventTarget = __refact_global.EventTarget;
const __refact_Event = __refact_global.Event;
const __refact_builtins = {{
  arrayFrom: __refact_Array.from.bind(__refact_Array),
  objectKeys: __refact_Object.keys.bind(__refact_Object),
  jsonStringify: __refact_JSON.stringify.bind(__refact_JSON),
  jsonParse: __refact_JSON.parse.bind(__refact_JSON),
  requestAnimationFrame: __refact_global.requestAnimationFrame.bind(__refact_global),
  cancelAnimationFrame: __refact_global.cancelAnimationFrame.bind(__refact_global),
  performanceNow: __refact_performance.now.bind(__refact_performance),
  getComputedStyle: __refact_global.getComputedStyle.bind(__refact_global),
  documentElementsFromPoint: (root, x, y) => __refact_Document.prototype.elementsFromPoint.call(root, x, y),
  documentElementFromPoint: (root, x, y) => __refact_Document.prototype.elementFromPoint.call(root, x, y),
  shadowElementsFromPoint: (root, x, y) => __refact_ShadowRoot.prototype.elementsFromPoint.call(root, x, y),
  shadowElementFromPoint: (root, x, y) => __refact_ShadowRoot.prototype.elementFromPoint.call(root, x, y),
  addWindowEventListener: (type, listener, options) => __refact_EventTarget.prototype.addEventListener.call(__refact_global, type, listener, options),
  removeWindowEventListener: (type, listener, options) => __refact_EventTarget.prototype.removeEventListener.call(__refact_global, type, listener, options),
  preventDefault: event => __refact_Event.prototype.preventDefault.call(event),
  stopPropagation: event => __refact_Event.prototype.stopPropagation.call(event),
  stopImmediatePropagation: event => __refact_Event.prototype.stopImmediatePropagation.call(event),
  setTimeout: __refact_global.setTimeout.bind(__refact_global),
  clearTimeout: __refact_global.clearTimeout.bind(__refact_global),
  Map: __refact_global.Map,
  Set: __refact_global.Set,
  WeakMap: __refact_global.WeakMap
}};
const module = {{ exports: {{}} }};
const exports = module.exports;
{}
if (!__refact_global.__refact_injected__) {{
  __refact_global.__refact_injected__ = module.exports.bootstrapRefactInjected()(
    __refact_global,
    __refact_builtins
  );
}}
return __refact_global.__refact_injected__;
}})()"#,
        INJECTED_BUNDLE
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_utility_notice_names_source_and_commit() {
        let notice = include_str!("../injected/NOTICE.md");
        assert!(notice.contains("src/vendor/injected/roleUtils.ts"));
        assert!(notice.contains("packages/injected/src/roleUtils.ts"));
        assert!(notice.contains("d5a185a894ab3ab17ff77a44e116a1339c6bdaed"));
    }

    #[test]
    fn injected_bundle_contains_playwright_selector_generator() {
        for marker in [
            "generateLocator\", {",
            "generateSelector(runtime, element, options)",
            "buildNoTextCandidates",
            "buildTextCandidates",
            "cssFallback",
            "kCSSFallbackScore",
        ] {
            assert!(INJECTED_BUNDLE.contains(marker), "missing {marker}");
        }
        let notice = include_str!("../injected/NOTICE.md");
        assert!(notice.contains("src/vendor/injected/selectorGenerator.ts"));
        assert!(notice.contains("packages/injected/src/selectorGenerator.ts"));
    }
}
