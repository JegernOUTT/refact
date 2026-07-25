pub mod arg_coerce;
pub mod integration_confirmation;
pub mod tool_desc;
pub mod tool_name_alias;

pub use arg_coerce::{
    coerce_args_to_schema, coerce_array, coerce_bool, coerce_hashmap_to_schema, coerce_integer,
    coerce_number, coerce_object, coerce_string,
};
pub use integration_confirmation::IntegrationConfirmation;
pub use tool_desc::{
    command_should_be_confirmed_by_user, command_should_be_denied, is_strict_compatible,
    json_schema_from_params, make_openai_tool_value, MatchConfirmDeny, MatchConfirmDenyResult,
    ToolConfig, ToolDesc, ToolGroupCategory, ToolSource, ToolSourceType,
};
pub use tool_name_alias::{
    build_registry_from_names, generate_tool_alias, ToolAliasRegistry, MAX_TOOL_NAME_LEN,
};
