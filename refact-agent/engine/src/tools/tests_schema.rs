#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::tools::tools_description::{
        json_schema_from_params, make_openai_tool_value, ToolDesc, ToolSource, ToolSourceType, Tool,
    };

    fn make_tool_desc(input_schema: serde_json::Value) -> ToolDesc {
        ToolDesc {
            name: "test_tool".to_string(),
            experimental: false,
            allow_parallel: false,
            description: "A test tool".to_string(),
            input_schema,
            output_schema: None,
            annotations: None,
            display_name: "Test Tool".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: "".to_string(),
            },
        }
    }

    #[test]
    fn test_json_schema_from_params_simple() {
        let schema = json_schema_from_params(
            &[
                ("path", "string", "File path"),
                ("content", "string", "Content"),
            ],
            &["path"],
        );
        assert_eq!(schema["type"], json!("object"));
        assert_eq!(schema["properties"]["path"]["type"], json!("string"));
        assert_eq!(
            schema["properties"]["path"]["description"],
            json!("File path")
        );
        assert_eq!(schema["properties"]["content"]["type"], json!("string"));
        assert_eq!(schema["required"], json!(["path"]));
    }

    #[test]
    fn test_json_schema_from_params_all_required() {
        let schema = json_schema_from_params(
            &[("a", "string", "First"), ("b", "integer", "Second")],
            &["a", "b"],
        );
        assert_eq!(schema["type"], json!("object"));
        assert_eq!(schema["required"], json!(["a", "b"]));
        assert_eq!(schema["properties"]["b"]["type"], json!("integer"));
    }

    #[test]
    fn test_json_schema_from_params_no_params() {
        let schema = json_schema_from_params(&[], &[]);
        assert_eq!(schema["type"], json!("object"));
        assert_eq!(schema["properties"], json!({}));
        assert_eq!(schema["required"], json!([]));
    }

    #[test]
    fn test_openai_style_simple_not_strict() {
        let schema = json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Search query"}
            },
            "required": ["query"]
        });
        let desc = make_tool_desc(schema);
        let openai = desc.into_openai_style(false);
        assert_eq!(openai["type"], json!("function"));
        assert_eq!(openai["function"]["name"], json!("test_tool"));
        assert_eq!(openai["function"]["parameters"]["type"], json!("object"));
        assert!(openai["function"]["strict"].is_null());
        assert!(openai["function"]["parameters"]["additionalProperties"].is_null());
    }

    #[test]
    fn test_openai_style_strict_adds_additional_properties_false() {
        let schema = json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"}
            },
            "required": ["query"]
        });
        let desc = make_tool_desc(schema);
        let openai = desc.into_openai_style(true);
        assert_eq!(openai["function"]["strict"], json!(true));
        assert_eq!(
            openai["function"]["parameters"]["additionalProperties"],
            json!(false)
        );
    }

    #[test]
    fn test_strict_preserves_existing_additional_properties_true() {
        let schema = json!({
            "type": "object",
            "properties": {},
            "additionalProperties": true
        });
        let openai = make_openai_tool_value("test".to_string(), "A tool".to_string(), schema, true);
        assert_eq!(
            openai["function"]["parameters"]["additionalProperties"],
            json!(true)
        );
    }

    #[test]
    fn test_strict_preserves_existing_additional_properties_false() {
        let schema = json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        });
        let openai = make_openai_tool_value("test".to_string(), "A tool".to_string(), schema, true);
        assert_eq!(
            openai["function"]["parameters"]["additionalProperties"],
            json!(false)
        );
    }

    #[test]
    fn test_complex_schema_passthrough_nested_objects() {
        let schema = json!({
            "type": "object",
            "properties": {
                "config": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "value": {"type": "number"}
                    }
                }
            },
            "required": ["config"]
        });
        let desc = make_tool_desc(schema);
        let openai = desc.into_openai_style(false);
        let params = &openai["function"]["parameters"];
        assert_eq!(params["properties"]["config"]["type"], json!("object"));
        assert_eq!(
            params["properties"]["config"]["properties"]["name"]["type"],
            json!("string")
        );
        assert_eq!(
            params["properties"]["config"]["properties"]["value"]["type"],
            json!("number")
        );
    }

    #[test]
    fn test_complex_schema_passthrough_arrays() {
        let schema = json!({
            "type": "object",
            "properties": {
                "tags": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "List of tags"
                }
            },
            "required": []
        });
        let desc = make_tool_desc(schema);
        let openai = desc.into_openai_style(false);
        let params = &openai["function"]["parameters"];
        assert_eq!(params["properties"]["tags"]["type"], json!("array"));
        assert_eq!(
            params["properties"]["tags"]["items"]["type"],
            json!("string")
        );
    }

    #[test]
    fn test_complex_schema_passthrough_enums() {
        let schema = json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["fast", "slow", "auto"]
                }
            },
            "required": ["mode"]
        });
        let desc = make_tool_desc(schema);
        let openai = desc.into_openai_style(false);
        let params = &openai["function"]["parameters"];
        assert_eq!(
            params["properties"]["mode"]["enum"],
            json!(["fast", "slow", "auto"])
        );
        assert_eq!(
            params["properties"]["mode"]["enum"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn test_complex_schema_all_types_preserved() {
        let schema = json!({
            "type": "object",
            "properties": {
                "config": {
                    "type": "object",
                    "properties": {
                        "verbose": {"type": "boolean"},
                        "max_count": {"type": "integer"}
                    }
                },
                "tags": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "List of items"
                },
                "mode": {
                    "type": "string",
                    "enum": ["fast", "slow", "auto"]
                }
            },
            "required": ["config"]
        });
        let desc = make_tool_desc(schema);
        let openai = desc.into_openai_style(false);
        let params = &openai["function"]["parameters"];
        assert_eq!(params["properties"]["config"]["type"], json!("object"));
        assert_eq!(
            params["properties"]["tags"]["items"]["type"],
            json!("string")
        );
        assert_eq!(
            params["properties"]["mode"]["enum"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn test_into_openai_style_preserves_name_and_description() {
        let schema = json!({"type": "object", "properties": {}});
        let desc = ToolDesc {
            name: "my_custom_tool".to_string(),
            experimental: false,
            allow_parallel: true,
            description: "Does something useful".to_string(),
            input_schema: schema,
            output_schema: None,
            annotations: None,
            display_name: "My Custom Tool".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: "".to_string(),
            },
        };
        let openai = desc.into_openai_style(false);
        assert_eq!(openai["function"]["name"], json!("my_custom_tool"));
        assert_eq!(
            openai["function"]["description"],
            json!("Does something useful")
        );
    }

    #[test]
    fn test_all_builtin_tools_have_valid_schema() {
        let tools: Vec<Box<dyn Tool + Send>> = vec![
            Box::new(crate::tools::tool_cat::ToolCat {
                config_path: "".to_string(),
            }),
            Box::new(crate::tools::tool_tree::ToolTree {
                config_path: "".to_string(),
            }),
            Box::new(crate::tools::tool_regex_search::ToolRegexSearch {
                config_path: "".to_string(),
            }),
            Box::new(crate::tools::tool_mv::ToolMv {
                config_path: "".to_string(),
            }),
            Box::new(crate::tools::tool_rm::ToolRm {
                config_path: "".to_string(),
            }),
            Box::new(crate::tools::tool_web::ToolWeb {
                config_path: "".to_string(),
            }),
            Box::new(crate::tools::tool_web_search::ToolWebSearch {
                config_path: "".to_string(),
            }),
            Box::new(crate::tools::tool_shell::ToolShell {
                cfg: Default::default(),
                config_path: "".to_string(),
            }),
            Box::new(
                crate::tools::file_edit::tool_create_textdoc::ToolCreateTextDoc {
                    config_path: "".to_string(),
                },
            ),
            Box::new(
                crate::tools::file_edit::tool_update_textdoc::ToolUpdateTextDoc {
                    config_path: "".to_string(),
                },
            ),
            Box::new(
                crate::tools::file_edit::tool_update_textdoc_by_lines::ToolUpdateTextDocByLines {
                    config_path: "".to_string(),
                },
            ),
            Box::new(
                crate::tools::file_edit::tool_update_textdoc_regex::ToolUpdateTextDocRegex {
                    config_path: "".to_string(),
                },
            ),
        ];

        for tool in &tools {
            let desc = tool.tool_description();
            let schema = &desc.input_schema;
            assert_eq!(
                schema["type"],
                json!("object"),
                "Tool '{}' input_schema must have type=object",
                desc.name
            );
            assert!(
                schema["properties"].is_object(),
                "Tool '{}' input_schema must have a properties object",
                desc.name
            );
            let openai = desc.clone().into_openai_style(false);
            assert_eq!(
                openai["type"],
                json!("function"),
                "Tool '{}' into_openai_style must produce type=function",
                desc.name
            );
            assert!(
                !openai["function"]["name"].as_str().unwrap_or("").is_empty(),
                "Tool '{}' must have non-empty name in openai format",
                desc.name
            );
        }
    }

    #[test]
    fn test_schema_roundtrip_tool_desc_to_openai() {
        let input_schema = json!({
            "type": "object",
            "properties": {
                "filename": {"type": "string", "description": "The filename"},
                "line_start": {"type": "integer", "description": "Start line"},
                "line_end": {"type": "integer", "description": "End line"}
            },
            "required": ["filename"]
        });
        let desc = make_tool_desc(input_schema.clone());
        let openai = desc.into_openai_style(false);
        assert_eq!(openai["function"]["parameters"], input_schema);
    }

    #[test]
    fn test_anthropic_roundtrip_via_openai() {
        let input_schema = json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Search query"},
                "limit": {"type": "integer"}
            },
            "required": ["query"]
        });
        let desc = make_tool_desc(input_schema.clone());
        let openai_tool = desc.into_openai_style(false);

        let func = openai_tool.get("function").unwrap();
        let anthropic = json!({
            "name": func["name"],
            "description": func["description"],
            "input_schema": func["parameters"]
        });

        assert_eq!(anthropic["name"], json!("test_tool"));
        assert_eq!(anthropic["input_schema"]["type"], json!("object"));
        assert_eq!(
            anthropic["input_schema"]["properties"]["query"]["type"],
            json!("string")
        );
        assert_eq!(anthropic["input_schema"]["required"], json!(["query"]));
        assert!(anthropic.get("parameters").is_none());
    }
}
