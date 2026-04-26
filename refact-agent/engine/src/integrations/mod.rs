// use std::path::PathBuf;
// use std::sync::Arc;
// use indexmap::IndexMap;
// use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};

// use crate::global_context::GlobalContext;
// use crate::tools::tools_description::Tool;
// use crate::yaml_configs::create_configs::{integrations_enabled_cfg, read_yaml_into_value};

pub mod browser_actions;
pub mod browser_controller;
pub mod browser_locators;
pub mod browser_models;
pub mod browser_runtime;
pub mod browser_types;
pub mod integr_abstract;
pub mod integr_bitbucket;
pub mod integr_cmdline;
pub mod integr_cmdline_service;
pub mod integr_github;
pub mod integr_gitlab;
pub mod integr_mysql;
pub mod integr_pdb;
pub mod integr_postgres;
pub mod mcp;

pub mod config_chat;
pub mod process_io_utils;
pub mod running_integrations;
pub mod sessions;
pub mod setting_up_integrations;
pub mod setup_chat;
pub mod utils;
pub mod yaml_schema;

use integr_abstract::IntegrationTrait;

pub fn integration_from_name(n: &str) -> Result<Box<dyn IntegrationTrait + Send + Sync>, String> {
    match n {
        "github" => Ok(Box::new(integr_github::ToolGithub {
            ..Default::default()
        }) as Box<dyn IntegrationTrait + Send + Sync>),
        "gitlab" => Ok(Box::new(integr_gitlab::ToolGitlab {
            ..Default::default()
        }) as Box<dyn IntegrationTrait + Send + Sync>),
        "bitbucket" => Ok(Box::new(integr_bitbucket::ToolBitbucket {
            ..Default::default()
        }) as Box<dyn IntegrationTrait + Send + Sync>),
        "pdb" => Ok(Box::new(integr_pdb::ToolPdb {
            ..Default::default()
        }) as Box<dyn IntegrationTrait + Send + Sync>),
        "postgres" => Ok(Box::new(integr_postgres::ToolPostgres {
            ..Default::default()
        }) as Box<dyn IntegrationTrait + Send + Sync>),
        "mysql" => Ok(Box::new(integr_mysql::ToolMysql {
            ..Default::default()
        }) as Box<dyn IntegrationTrait + Send + Sync>),
        cmdline if cmdline.starts_with("cmdline_") => {
            // let tool_name = cmdline.strip_prefix("cmdline_").unwrap();
            Ok(Box::new(integr_cmdline::ToolCmdline {
                ..Default::default()
            }) as Box<dyn IntegrationTrait + Send + Sync>)
        }
        service if service.starts_with("service_") => {
            Ok(Box::new(integr_cmdline_service::ToolService {
                ..Default::default()
            }) as Box<dyn IntegrationTrait + Send + Sync>)
        }
        mcp_sse if mcp_sse.starts_with("mcp_sse_") => {
            Ok(Box::new(mcp::integr_mcp_sse::IntegrationMCPSse {
                ..Default::default()
            }) as Box<dyn IntegrationTrait + Send + Sync>)
        }
        mcp_http if mcp_http.starts_with("mcp_http_") => {
            Ok(Box::new(mcp::integr_mcp_http::IntegrationMCPHttp {
                ..Default::default()
            }) as Box<dyn IntegrationTrait + Send + Sync>)
        }
        // mcp_TEMPLATE uses the unified schema
        "mcp_TEMPLATE" => Ok(Box::new(mcp::integr_mcp_stdio::IntegrationMCPUnified {
            ..Default::default()
        }) as Box<dyn IntegrationTrait + Send + Sync>),
        // We support also mcp_* as mcp_stdio_* for backwards compatibility, some users already have it configured.
        mcp_stdio if mcp_stdio.starts_with("mcp_stdio_") || mcp_stdio.starts_with("mcp_") => {
            Ok(Box::new(mcp::integr_mcp_stdio::IntegrationMCPStdio {
                ..Default::default()
            }) as Box<dyn IntegrationTrait + Send + Sync>)
        }
        _ => Err(format!("Unknown integration name: {}", n)),
    }
}

pub fn integrations_list(_allow_experimental: bool) -> Vec<&'static str> {
    let integrations = vec![
        "github",
        "gitlab",
        "bitbucket",
        "pdb",
        "postgres",
        "mysql",
        "cmdline_TEMPLATE",
        "service_TEMPLATE",
        "mcp_TEMPLATE",
    ];
    integrations
}

pub fn go_to_configuration_message(integration_name: &str) -> String {
    format!("🧩 for configuration go to SETTINGS:{integration_name}")
}
