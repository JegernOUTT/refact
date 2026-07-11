use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Clone, Default, Debug)]
pub struct IntegrationConfirmation {
    #[serde(default, alias = "ask_user_default")]
    pub ask_user: Vec<String>,
    #[serde(default, alias = "deny_default")]
    pub deny: Vec<String>,
}
