use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SecretFieldInfo {
    pub name: String,
    pub prompt: String,
    pub optional: bool,
    pub provided: bool,
    pub auto_generate: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetupFieldInfo {
    pub name: String,
    pub prompt: String,
    pub optional: bool,
    pub provided: bool,
    pub input_type: crate::tools::wasm::ToolSetupFieldInputType,
}
