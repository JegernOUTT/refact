#[cfg(test)]
mod tests {
    use rmcp::model::Tool as McpTool;
    use serde_json::json;

    use crate::integrations::integr_abstract::IntegrationCommon;
    use crate::tools::tools_description::Tool;

    use super::super::tool_mcp::ToolMCP;

    fn make_tool_mcp(config_path: &str, schema: serde_json::Value, tool_name: &str) -> ToolMCP {
        let mcp_tool: McpTool = serde_json::from_value(json!({
            "name": tool_name,
            "description": "A test tool",
            "inputSchema": schema
        }))
        .expect("failed to deserialize McpTool");
        let server_prefix =
            crate::integrations::mcp::integr_mcp_common::tool_name_server_prefix(config_path);
        ToolMCP {
            model_name: crate::integrations::mcp::mcp_naming::model_tool_name(
                &server_prefix,
                tool_name,
            ),
            common: IntegrationCommon::default(),
            config_path: config_path.to_string(),
            mcp_client: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            mcp_tool,
            request_timeout: 30,
            auto_approve: false,
        }
    }

    #[test]
    fn test_mcp_naming_stdio_prefix_stripped() {
        let tool = make_tool_mcp(
            "mcp_stdio_myserver.yaml",
            json!({"type": "object", "properties": {}}),
            "do_something",
        );
        let desc = tool.tool_description();
        assert_eq!(desc.name, "mcp_myserver_do_something");
    }

    #[test]
    fn test_mcp_naming_sse_keeps_full_stem() {
        // Remote configs keep their full stem so a stdio and an sse config for
        // the same server never publish colliding tool names.
        let tool = make_tool_mcp(
            "mcp_sse_myserver.yaml",
            json!({"type": "object", "properties": {}}),
            "fetch_data",
        );
        let desc = tool.tool_description();
        assert_eq!(desc.name, "mcp_sse_myserver_fetch_data");
    }

    #[test]
    fn test_mcp_naming_plain_yaml() {
        let tool = make_tool_mcp(
            "plain_integration.yaml",
            json!({"type": "object", "properties": {}}),
            "run_query",
        );
        let desc = tool.tool_description();
        assert_eq!(desc.name, "plain_integration_run_query");
    }

    #[test]
    fn test_mcp_naming_special_chars_sanitized() {
        let tool = make_tool_mcp(
            "mcp_stdio_my-server.yaml",
            json!({"type": "object", "properties": {}}),
            "tool-with-dashes",
        );
        let desc = tool.tool_description();
        assert!(
            !desc.name.contains('-'),
            "hyphens should be replaced with underscores"
        );
        assert!(
            desc.name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "name should only contain alphanumerics and underscores, got: {}",
            desc.name
        );
    }

    #[test]
    fn test_mcp_naming_display_name_is_original_tool_name() {
        let tool = make_tool_mcp(
            "mcp_stdio_server.yaml",
            json!({"type": "object", "properties": {}}),
            "original_tool",
        );
        let desc = tool.tool_description();
        assert_eq!(desc.display_name, "original_tool");
    }

    #[test]
    fn test_mcp_schema_preserved_verbatim_complex() {
        let complex_schema = json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "List of items"
                },
                "config": {
                    "type": "object",
                    "properties": {
                        "verbose": {"type": "boolean"},
                        "max_count": {"type": "integer"}
                    }
                },
                "mode": {
                    "type": "string",
                    "enum": ["fast", "slow", "medium"]
                }
            },
            "required": ["items"]
        });
        let tool = make_tool_mcp("mcp_stdio_srv.yaml", complex_schema.clone(), "process");
        let desc = tool.tool_description();

        assert_eq!(desc.input_schema["type"], json!("object"));
        assert_eq!(
            desc.input_schema["properties"]["items"]["type"],
            json!("array")
        );
        assert_eq!(
            desc.input_schema["properties"]["items"]["items"]["type"],
            json!("string")
        );
        assert_eq!(
            desc.input_schema["properties"]["config"]["type"],
            json!("object")
        );
        assert_eq!(
            desc.input_schema["properties"]["mode"]["enum"],
            json!(["fast", "slow", "medium"])
        );
        assert_eq!(desc.input_schema["required"], json!(["items"]));
    }

    #[test]
    fn test_mcp_schema_without_type_gets_object_type() {
        let schema_without_type = json!({
            "properties": {
                "a": {"type": "integer"},
                "b": {"type": "string"}
            },
            "required": ["a"]
        });
        let tool = make_tool_mcp("mcp_stdio_srv.yaml", schema_without_type, "add");
        let desc = tool.tool_description();
        assert_eq!(desc.input_schema["type"], json!("object"));
        assert_eq!(
            desc.input_schema["properties"]["a"]["type"],
            json!("integer")
        );
    }

    #[test]
    fn test_mcp_schema_into_openai_style() {
        let schema = json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Search query"}
            },
            "required": ["query"]
        });
        let tool = make_tool_mcp("mcp_stdio_search.yaml", schema, "search");
        let desc = tool.tool_description();
        let openai = desc.into_openai_style(false);
        assert_eq!(openai["type"], json!("function"));
        assert_eq!(
            openai["function"]["parameters"]["properties"]["query"]["type"],
            json!("string")
        );
    }

    #[test]
    fn test_mcp_description_propagated() {
        let mcp_tool: McpTool = serde_json::from_value(json!({
            "name": "my_tool",
            "description": "My special tool description",
            "inputSchema": {"type": "object", "properties": {}}
        }))
        .expect("failed to deserialize");
        let tool = ToolMCP {
            model_name: "mcp_server_test_tool".to_string(),
            common: IntegrationCommon::default(),
            config_path: "mcp_stdio_srv.yaml".to_string(),
            mcp_client: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            mcp_tool,
            request_timeout: 30,
            auto_approve: false,
        };
        let desc = tool.tool_description();
        assert_eq!(desc.description, "My special tool description");
    }

    #[test]
    fn test_mcp_no_description_defaults_empty() {
        let mcp_tool: McpTool = serde_json::from_value(json!({
            "name": "no_desc_tool",
            "inputSchema": {"type": "object", "properties": {}}
        }))
        .expect("failed to deserialize");
        let tool = ToolMCP {
            model_name: "mcp_server_test_tool".to_string(),
            common: IntegrationCommon::default(),
            config_path: "mcp_stdio_srv.yaml".to_string(),
            mcp_client: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            mcp_tool,
            request_timeout: 30,
            auto_approve: false,
        };
        let desc = tool.tool_description();
        assert_eq!(desc.description, "");
    }

    #[test]
    fn test_mcp_http_keeps_full_stem() {
        // Remote configs keep their full stem so a stdio and an http config
        // for the same server never publish colliding tool names.
        let tool = make_tool_mcp(
            "mcp_http_myserver.yaml",
            json!({"type": "object", "properties": {}}),
            "do_something",
        );
        let desc = tool.tool_description();
        assert_eq!(desc.name, "mcp_http_myserver_do_something");
    }
}
