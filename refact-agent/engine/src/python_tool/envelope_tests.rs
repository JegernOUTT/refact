use super::*;
use serde_json::json;

#[test]
fn structured_data_and_failed_tools() {
    let mut message = ChatMessage {
        role: "tool".into(),
        content: ChatContent::SimpleText("failed\ndetail".into()),
        tool_failed: Some(true),
        ..Default::default()
    };
    message
        .extra
        .insert("tool_output".into(), json!({"answer":42}));
    let result = build_envelope(&[message], 4096, &AtomicUsize::new(0));
    assert_eq!(result.envelope["ok"], false);
    assert_eq!(result.envelope["data"]["answer"], 42);
    assert_eq!(result.envelope["error"], "failed\ndetail");
}

#[test]
fn oversized_result_is_truncated() {
    let message = ChatMessage {
        role: "tool".into(),
        content: ChatContent::SimpleText("é".repeat(4000)),
        ..Default::default()
    };
    let result = build_envelope(&[message], 256, &AtomicUsize::new(0));
    assert_eq!(result.envelope["truncated"], true);
    assert!(serde_json::to_vec(&result.envelope).unwrap().len() <= 256);
}

#[test]
fn oversized_structured_data_and_files_are_bounded() {
    let mut message = ChatMessage {
        role: "tool".into(),
        ..Default::default()
    };
    message
        .extra
        .insert("tool_output".into(), json!({"large":"x".repeat(5000)}));
    let files = ChatMessage {
        role: "context_file".into(),
        content: ChatContent::ContextFiles(vec![crate::call_validation::ContextFile {
            file_name: "x".into(),
            line1: 1,
            line2: 2,
            file_content: "large".repeat(400),
            ..Default::default()
        }]),
        ..Default::default()
    };
    let result = build_envelope(&[message, files], 256, &AtomicUsize::new(0));
    assert_eq!(result.envelope["truncated"], true);
    assert!(serde_json::to_vec(&result.envelope).unwrap().len() <= 256);
}

#[test]
fn diffs_privacy_records_and_images_are_collected() {
    let mut message = ChatMessage {
        role: "tool".into(),
        content: ChatContent::Multimodal(vec![MultimodalElement {
            m_type: "image/png".into(),
            m_content: "abc".into(),
        }]),
        ..Default::default()
    };
    message
        .extra
        .insert("privacy".into(), json!({"kind":"guarded"}));
    let diff = ChatMessage {
        role: "diff".into(),
        content: ChatContent::SimpleText(json!([{"file_name":"x"}]).to_string()),
        ..Default::default()
    };
    let result = build_envelope(&[message, diff], 4096, &AtomicUsize::new(3));
    assert_eq!(result.images.len(), 1);
    assert_eq!(result.envelope["images"][0]["index"], 3);
    assert_eq!(result.privacy.len(), 1);
    assert_eq!(result.envelope["diffs"][0]["file_name"], "x");
}

#[test]
fn data_is_read_from_diff_message_for_edit_tools() {
    let mut message = ChatMessage {
        role: "diff".into(),
        content: ChatContent::SimpleText("[]".into()),
        ..Default::default()
    };
    message
        .extra
        .insert("tool_output".into(), json!({"edited":true}));
    let result = build_envelope(&[message], 4096, &AtomicUsize::new(0));
    assert_eq!(result.envelope["data"], json!({"edited":true}));
}

#[test]
fn images_are_limited_per_call_per_run_and_by_size() {
    let message = |count, bytes| ChatMessage {
        role: "tool".into(),
        content: ChatContent::Multimodal(vec![
            MultimodalElement {
                m_type: "image/png".into(),
                m_content: "a".repeat(bytes)
            };
            count
        ]),
        ..Default::default()
    };
    let index = AtomicUsize::new(0);
    let first = build_envelope(&[message(12, 8)], 4096, &index);
    assert_eq!(first.images.len(), 8);
    assert_eq!(first.envelope["images_dropped"], 4);
    let second = build_envelope(&[message(12, 8)], 4096, &index);
    assert_eq!(second.images.len(), 8);
    let third = build_envelope(&[message(1, 8)], 4096, &index);
    assert_eq!(third.images.len(), 0);
    let large = build_envelope(&[message(1, 8 * 1024 * 1024)], 4096, &AtomicUsize::new(0));
    assert_eq!(large.images.len(), 0);
    assert_eq!(large.envelope["images_dropped"], 1);
}

#[test]
fn bounded_envelope_never_exports_unshrunk_record_diffs() {
    let message = ChatMessage {
        role: "diff".into(),
        content: ChatContent::SimpleText(json!([{"lines_add":"x".repeat(20000)}]).to_string()),
        ..Default::default()
    };
    let built = build_envelope(&[message], 1000, &AtomicUsize::new(0));
    assert!(built.diffs.is_empty());
    assert!(built.summary.contains("diff"));
}

#[test]
fn runtime_envelope_keys_match_catalog_tool_result() {
    let result = build_envelope(&[], 4096, &AtomicUsize::new(0));
    let catalog = crate::python_tool::catalog_binding::catalog(&[], false, 100000).description;
    let declared = catalog
        .lines()
        .find(|line| line.starts_with("# ToolResult = {"))
        .unwrap();
    let keys: Vec<_> = result.envelope.as_object().unwrap().keys().collect();
    for key in keys {
        assert!(
            declared.contains(&format!("{{{key}: ")) || declared.contains(&format!(", {key}: ")),
            "undeclared runtime field: {key}"
        );
    }
}
