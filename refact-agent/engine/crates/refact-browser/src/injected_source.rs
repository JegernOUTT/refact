pub const INJECTED_BUNDLE: &str = include_str!("generated/injected_bundle.js");

pub fn wrapped_bootstrap() -> String {
    format!(
        r#"(() => {{
const __refact_global = globalThis;
const __refact_Array = __refact_global.Array;
const __refact_Object = __refact_global.Object;
const __refact_JSON = __refact_global.JSON;
const __refact_performance = __refact_global.performance;
const __refact_builtins = {{
  arrayFrom: __refact_Array.from.bind(__refact_Array),
  objectKeys: __refact_Object.keys.bind(__refact_Object),
  jsonStringify: __refact_JSON.stringify.bind(__refact_JSON),
  jsonParse: __refact_JSON.parse.bind(__refact_JSON),
  requestAnimationFrame: __refact_global.requestAnimationFrame.bind(__refact_global),
  performanceNow: __refact_performance.now.bind(__refact_performance),
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
    fn injected_bundle_is_non_empty() {
        assert!(INJECTED_BUNDLE.len() > 100);
        assert!(INJECTED_BUNDLE.len() < 500 * 1024);
    }

    #[test]
    fn injected_bundle_exports_commonjs_module() {
        assert!(INJECTED_BUNDLE.contains("module.exports"));
        assert!(INJECTED_BUNDLE.contains("RefactInjected"));
        assert!(INJECTED_BUNDLE.contains("bootstrapRefactInjected"));
    }

    #[test]
    fn injected_bundle_has_source_hash_header() {
        let hash = INJECTED_BUNDLE
            .lines()
            .next()
            .unwrap()
            .strip_prefix("// @refact-injected-hash ")
            .unwrap();
        assert_eq!(hash.len(), 64);
        assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn bootstrap_snapshots_page_replaceable_globals() {
        let source = wrapped_bootstrap();
        for marker in [
            "const __refact_builtins",
            "arrayFrom:",
            "objectKeys:",
            "jsonStringify:",
            "jsonParse:",
            "requestAnimationFrame:",
            "performanceNow:",
            "setTimeout:",
            "clearTimeout:",
            "Map:",
            "Set:",
            "WeakMap:",
        ] {
            assert!(source.contains(marker), "missing {marker}");
        }
    }

    #[test]
    fn bootstrap_evaluates_bundle_in_commonjs_wrapper() {
        let source = wrapped_bootstrap();
        assert!(source.contains("const module = { exports: {} };"));
        assert!(source.contains("module.exports.bootstrapRefactInjected()"));
        assert!(source.contains("__refact_injected__"));
        assert!(source.contains("__refact_binding"));
        assert!(source.ends_with("})()"));
    }

    #[test]
    fn injected_bundle_is_fresh_when_requested() {
        if std::env::var("REFACT_CHECK_INJECTED_FRESH").as_deref() != Ok("1") {
            return;
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
        let status = std::process::Command::new("bash")
            .arg("tools/dev/build_injected.sh")
            .current_dir(root)
            .status()
            .unwrap();
        assert!(status.success());
        let rebuilt = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/generated/injected_bundle.js"),
        )
        .unwrap();
        assert_eq!(rebuilt, INJECTED_BUNDLE);
    }
}
