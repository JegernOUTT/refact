pub const TOOL_OUTPUT_EXTRA_KEY: &str = "tool_output";

pub fn output_schema_for<T: schemars::JsonSchema>() -> serde_json::Value {
    let mut settings = schemars::generate::SchemaSettings::default().for_serialize();
    settings.inline_subschemas = true;
    let schema = settings.into_generator().into_root_schema_for::<T>();
    serde_json::to_value(schema).expect("JSON Schema is serializable")
}

#[cfg(test)]
mod tests {
    use super::output_schema_for;
    use schemars::JsonSchema;
    use serde::Serialize;

    #[derive(JsonSchema, Serialize)]
    enum State {
        Ready,
        Done,
    }

    #[derive(JsonSchema, Serialize)]
    struct Output {
        state: State,
        detail: Option<String>,
    }

    #[test]
    fn validates_serialized_output() {
        let schema = output_schema_for::<Output>();
        assert!(schema.is_object());
        for output in [
            Output {
                state: State::Ready,
                detail: Some("ok".into()),
            },
            Output {
                state: State::Done,
                detail: None,
            },
        ] {
            crate::schema_validate::validate(&schema, &serde_json::to_value(output).unwrap())
                .unwrap();
        }
    }
}
