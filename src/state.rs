use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    #[serde(default = "default_true")]
    pub present: bool,
    pub members: Vec<String>,
    #[serde(default = "default_true")]
    pub overwrite_members: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    #[serde(default = "default_true")]
    pub present: bool,
    pub display_name: String,
    pub legal_name: Option<String>,
    pub mail_addresses: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimMap {
    pub join_type: String,
    pub values_by_group: HashMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum StringOrStrings {
    String(String),
    Strings(Vec<String>),
}

impl StringOrStrings {
    pub fn strings(self) -> Vec<String> {
        match self {
            StringOrStrings::String(x) => vec![x],
            StringOrStrings::Strings(xs) => xs,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Oauth2System {
    #[serde(default = "default_true")]
    pub present: bool,
    #[serde(default = "default_false")]
    pub public: bool,
    pub display_name: String,
    pub basic_secret_file: Option<String>,
    pub image_file: Option<String>,
    pub origin_url: StringOrStrings,
    pub origin_landing: String,
    #[serde(default = "default_false")]
    pub enable_localhost_redirects: bool,
    #[serde(default = "default_false")]
    pub enable_legacy_crypto: bool,
    #[serde(default = "default_false")]
    pub allow_insecure_client_disable_pkce: bool,
    #[serde(default = "default_false")]
    pub prefer_short_username: bool,
    #[serde(default)]
    pub scope_maps: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub supplementary_scope_maps: HashMap<String, Vec<String>>,
    #[serde(default = "default_true")]
    pub remove_orphaned_claim_maps: bool,
    #[serde(default)]
    pub claim_maps: HashMap<String, ClaimMap>,
    #[serde(default)]
    pub k8s: Option<Oauth2K8sConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Systems {
    pub oauth2: HashMap<String, Oauth2System>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub groups: HashMap<String, Group>,
    pub persons: HashMap<String, Person>,
    pub systems: Systems,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Oauth2K8sConfig {
    pub image_url: Option<String>,
    #[serde(default = "default_client_id_key")]
    pub client_id_key: String,
    #[serde(default = "default_client_secret_key")]
    pub client_secret_key: String,
}

fn default_client_id_key() -> String {
    "client-id".to_string()
}
fn default_client_secret_key() -> String {
    "client-secret".to_string()
}

fn default_false() -> bool {
    false
}
fn default_true() -> bool {
    true
}
