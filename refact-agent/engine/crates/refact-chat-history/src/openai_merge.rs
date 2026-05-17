use serde_json::json;

/// Maximum number of parallel tool calls to prevent memory DoS
const MAX_TOOL_CALLS: usize = 128;

/// Accumulator for streaming tool calls that avoids O(n²) string concatenation.
/// Use `ToolCallAccumulator` for streaming, then call `finalize()` to get the final JSON.
#[derive(Default)]
pub struct ToolCallAccumulator {
    pub entries: Vec<ToolCallEntry>,
}

#[derive(Default)]
pub struct ToolCallEntry {
    pub id: Option<String>,
    pub tool_type: Option<String>,
    pub name: String,
    pub arguments: String, // Mutable String for efficient append
    pub index: usize,
    pub initialized: bool, // Track if this entry received meaningful data
    pub extra_content: Option<serde_json::Value>, // Gemini thought_signature etc.
}

impl ToolCallAccumulator {
    pub fn merge(&mut self, new_tc: &serde_json::Value) {
        let index = new_tc
            .get("index")
            .and_then(|i| {
                i.as_u64()
                    .or_else(|| i.as_str().and_then(|s| s.parse().ok()))
            })
            .unwrap_or(0) as usize;

        // Prevent memory DoS from huge indices
        if index >= MAX_TOOL_CALLS {
            tracing::warn!(
                "Tool call index {} exceeds maximum {}, ignoring",
                index,
                MAX_TOOL_CALLS
            );
            return;
        }

        while self.entries.len() <= index {
            self.entries.push(ToolCallEntry {
                index: self.entries.len(),
                ..Default::default()
            });
        }

        let entry = &mut self.entries[index];

        // Track if we received meaningful data (not just an empty delta)
        let mut has_meaningful_data = false;

        if let Some(id) = new_tc.get("id").and_then(|v| v.as_str()) {
            if !id.is_empty() {
                entry.id = Some(id.to_string());
                has_meaningful_data = true;
            }
        }

        if let Some(t) = new_tc.get("type").and_then(|v| v.as_str()) {
            entry.tool_type = Some(t.to_string());
            has_meaningful_data = true;
        }

        if let Some(func) = new_tc.get("function") {
            if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                if !name.is_empty() {
                    entry.name = name.to_string();
                    has_meaningful_data = true;
                }
            }

            if let Some(args) = func.get("arguments") {
                if !args.is_null() {
                    // O(1) amortized append to String - avoid unnecessary allocation
                    if let Some(s) = args.as_str() {
                        if !s.is_empty() {
                            entry.arguments.push_str(s);
                            has_meaningful_data = true;
                        }
                    } else {
                        let serialized = serde_json::to_string(args).unwrap_or_default();
                        if !serialized.is_empty() {
                            entry.arguments.push_str(&serialized);
                            has_meaningful_data = true;
                        }
                    }
                }
            }
        }

        if let Some(extra) = new_tc.get("extra_content") {
            if !extra.is_null() {
                entry.extra_content = Some(extra.clone());
                has_meaningful_data = true;
            }
        }

        // Only mark as initialized if we received meaningful data
        if has_meaningful_data {
            entry.initialized = true;
        }
    }

    /// Set the final/complete state of a tool call entry, replacing accumulated arguments.
    /// Used by `.done` events that carry the complete data instead of incremental deltas.
    pub fn set_final(&mut self, new_tc: &serde_json::Value) {
        let index = new_tc
            .get("index")
            .and_then(|i| {
                i.as_u64()
                    .or_else(|| i.as_str().and_then(|s| s.parse().ok()))
            })
            .unwrap_or(0) as usize;

        if index >= MAX_TOOL_CALLS {
            tracing::warn!(
                "Tool call index {} exceeds maximum {}, ignoring finalize",
                index,
                MAX_TOOL_CALLS
            );
            return;
        }

        while self.entries.len() <= index {
            self.entries.push(ToolCallEntry {
                index: self.entries.len(),
                ..Default::default()
            });
        }

        let entry = &mut self.entries[index];
        let mut has_meaningful_data = false;

        if let Some(id) = new_tc.get("id").and_then(|v| v.as_str()) {
            if !id.is_empty() {
                entry.id = Some(id.to_string());
                has_meaningful_data = true;
            }
        }

        if let Some(t) = new_tc.get("type").and_then(|v| v.as_str()) {
            entry.tool_type = Some(t.to_string());
            has_meaningful_data = true;
        }

        if let Some(func) = new_tc.get("function") {
            if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                if !name.is_empty() {
                    entry.name = name.to_string();
                    has_meaningful_data = true;
                }
            }

            if let Some(args) = func.get("arguments") {
                if !args.is_null() {
                    let final_args = if let Some(s) = args.as_str() {
                        s.to_string()
                    } else {
                        serde_json::to_string(args).unwrap_or_default()
                    };
                    // Replace unconditionally — this is finalization, even empty is valid
                    entry.arguments = final_args;
                    has_meaningful_data = true;
                }
            }
        }

        if let Some(extra) = new_tc.get("extra_content") {
            if !extra.is_null() {
                entry.extra_content = Some(extra.clone());
                has_meaningful_data = true;
            }
        }

        if has_meaningful_data {
            entry.initialized = true;
        }
    }

    /// Convert accumulated entries to final JSON format.
    /// Filters out uninitialized placeholder entries (phantom tool calls).
    /// Uses stable synthetic IDs based on index for entries without real IDs.
    pub fn finalize(&self) -> Vec<serde_json::Value> {
        self.entries
            .iter()
            .filter(|entry| entry.initialized && !entry.name.is_empty())
            .map(|entry| {
                // Use stable synthetic ID based on index, not random UUID
                let id = entry
                    .id
                    .clone()
                    .unwrap_or_else(|| format!("pending_call_{}", entry.index));
                let mut tc = json!({
                    "id": id,
                    "type": entry.tool_type.as_deref().unwrap_or("function"),
                    "index": entry.index,
                    "function": {
                        "name": entry.name,
                        "arguments": entry.arguments
                    }
                });
                if let Some(extra) = &entry.extra_content {
                    tc["extra_content"] = extra.clone();
                }
                tc
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_accumulator_basic_streaming() {
        let mut acc = ToolCallAccumulator::default();
        acc.merge(&json!({
            "index": 0,
            "id": "call_123",
            "type": "function",
            "function": {"name": "test", "arguments": "{\"a\":"}
        }));
        acc.merge(&json!({
            "index": 0,
            "function": {"arguments": " 1}"}
        }));

        let result = acc.finalize();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["id"], "call_123");
        assert_eq!(result[0]["function"]["name"], "test");
        assert_eq!(result[0]["function"]["arguments"], "{\"a\": 1}");
    }

    #[test]
    fn test_accumulator_parallel_tool_calls() {
        let mut acc = ToolCallAccumulator::default();
        acc.merge(
            &json!({"index": 0, "id": "call_1", "function": {"name": "func1", "arguments": "{}"}}),
        );
        acc.merge(
            &json!({"index": 1, "id": "call_2", "function": {"name": "func2", "arguments": "{}"}}),
        );

        let result = acc.finalize();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["function"]["name"], "func1");
        assert_eq!(result[1]["function"]["name"], "func2");
    }

    #[test]
    fn test_accumulator_generates_stable_id_if_missing() {
        let mut acc = ToolCallAccumulator::default();
        acc.merge(&json!({"index": 0, "function": {"name": "test", "arguments": "{}"}}));

        // Call finalize multiple times - ID should be stable
        let result1 = acc.finalize();
        let result2 = acc.finalize();
        let id1 = result1[0]["id"].as_str().unwrap();
        let id2 = result2[0]["id"].as_str().unwrap();
        assert_eq!(id1, id2, "ID should be stable across finalize calls");
        assert_eq!(id1, "pending_call_0", "Should use index-based synthetic ID");
    }

    #[test]
    fn test_accumulator_filters_phantom_entries() {
        let mut acc = ToolCallAccumulator::default();
        // Tool call arrives with index 2 first - creates placeholders for 0 and 1
        acc.merge(&json!({"index": 2, "id": "call_real", "function": {"name": "real_func", "arguments": "{}"}}));

        let result = acc.finalize();
        // Should only have 1 entry (the real one), not 3 phantom entries
        assert_eq!(
            result.len(),
            1,
            "Should filter out uninitialized placeholder entries"
        );
        assert_eq!(result[0]["id"], "call_real");
        assert_eq!(result[0]["function"]["name"], "real_func");
        assert_eq!(result[0]["index"], 2);
    }

    #[test]
    fn test_accumulator_large_arguments_efficient() {
        let mut acc = ToolCallAccumulator::default();
        acc.merge(
            &json!({"index": 0, "id": "call_1", "function": {"name": "test", "arguments": ""}}),
        );

        // Simulate streaming many small chunks (would be O(n²) with naive concat)
        for i in 0..1000 {
            acc.merge(&json!({"index": 0, "function": {"arguments": format!("{},", i)}}));
        }

        let result = acc.finalize();
        let args = result[0]["function"]["arguments"].as_str().unwrap();
        assert!(args.starts_with("0,1,2,"));
        assert!(args.len() > 3000); // Should have all the numbers
    }

    #[test]
    fn test_accumulator_rejects_huge_index() {
        let mut acc = ToolCallAccumulator::default();
        // Try to create a tool call with a huge index (memory DoS attempt)
        acc.merge(&json!({"index": 1000000, "id": "call_huge", "function": {"name": "bad", "arguments": "{}"}}));

        // Should be ignored - no entries created
        let result = acc.finalize();
        assert!(result.is_empty(), "Huge index should be rejected");
    }

    #[test]
    fn test_accumulator_accepts_max_valid_index() {
        let mut acc = ToolCallAccumulator::default();
        // Index 127 should be accepted (MAX_TOOL_CALLS = 128)
        acc.merge(&json!({"index": 127, "id": "call_max", "function": {"name": "valid", "arguments": "{}"}}));

        let result = acc.finalize();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["id"], "call_max");
    }

    #[test]
    fn test_accumulator_ignores_empty_delta() {
        let mut acc = ToolCallAccumulator::default();
        // Empty delta with just index - should not mark as initialized
        acc.merge(&json!({"index": 0}));

        let result = acc.finalize();
        assert!(
            result.is_empty(),
            "Empty delta should not create initialized entry"
        );
    }

    #[test]
    fn test_set_final_replaces_accumulated_arguments() {
        let mut acc = ToolCallAccumulator::default();
        // Simulate streaming deltas
        acc.merge(&json!({
            "index": 0,
            "id": "call_123",
            "type": "function",
            "function": {"name": "get_weather", "arguments": "{\"loc"}
        }));
        acc.merge(&json!({
            "index": 0,
            "function": {"arguments": "ation\":"}
        }));
        acc.merge(&json!({
            "index": 0,
            "function": {"arguments": "\"Paris\"}"}
        }));

        // Now finalize with complete arguments (should replace, not append)
        acc.set_final(&json!({
            "index": 0,
            "type": "function",
            "function": {"name": "get_weather", "arguments": "{\"location\":\"Paris\"}"}
        }));

        let result = acc.finalize();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0]["function"]["arguments"],
            "{\"location\":\"Paris\"}"
        );
        assert_eq!(result[0]["id"], "call_123");
    }

    #[test]
    fn test_set_final_without_prior_deltas() {
        let mut acc = ToolCallAccumulator::default();
        acc.set_final(&json!({
            "index": 0,
            "id": "call_456",
            "type": "function",
            "function": {"name": "search", "arguments": "{\"q\":\"test\"}"}
        }));

        let result = acc.finalize();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["id"], "call_456");
        assert_eq!(result[0]["function"]["name"], "search");
        assert_eq!(result[0]["function"]["arguments"], "{\"q\":\"test\"}");
    }

    #[test]
    fn test_accumulator_filters_empty_name_with_arguments() {
        let mut acc = ToolCallAccumulator::default();
        acc.merge(
            &json!({"index": 0, "id": "call_123", "function": {"arguments": "{\"q\":\"test\"}"}}),
        );

        let result = acc.finalize();
        assert!(
            result.is_empty(),
            "Tool call with empty name should be filtered out"
        );
    }

    #[test]
    fn test_accumulator_empty_strings_not_meaningful() {
        let mut acc = ToolCallAccumulator::default();
        // Delta with empty strings - should not mark as initialized
        acc.merge(&json!({"index": 0, "id": "", "function": {"name": "", "arguments": ""}}));

        let result = acc.finalize();
        assert!(
            result.is_empty(),
            "Empty strings should not create initialized entry"
        );
    }
}
