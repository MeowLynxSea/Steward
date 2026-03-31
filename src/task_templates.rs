use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::task_runtime::TaskMode;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskTemplateRecord {
    pub id: String,
    pub name: String,
    pub description: String,
    pub parameter_schema: Value,
    pub default_mode: TaskMode,
    pub output_expectations: Value,
    pub builtin: bool,
    pub mutable: bool,
    pub clonable: bool,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct TaskTemplateDraft {
    pub name: String,
    pub description: String,
    pub parameter_schema: Value,
    pub default_mode: TaskMode,
    pub output_expectations: Value,
}

#[derive(Debug, Clone)]
pub struct TemplateValidationError {
    pub message: String,
    pub field_errors: HashMap<String, String>,
}

impl TaskTemplateRecord {
    pub fn from_user_row(
        id: String,
        name: String,
        description: String,
        parameter_schema: Value,
        default_mode: TaskMode,
        output_expectations: Value,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            name,
            description,
            parameter_schema,
            default_mode,
            output_expectations,
            builtin: false,
            mutable: true,
            clonable: true,
            created_at: Some(created_at),
            updated_at: Some(updated_at),
        }
    }
}

pub fn builtin_templates() -> Vec<TaskTemplateRecord> {
    vec![
        TaskTemplateRecord {
            id: "builtin:file-archive".to_string(),
            name: "File Archive".to_string(),
            description: "Scan a source directory, propose organization actions, and execute them safely in Ask or Yolo mode.".to_string(),
            parameter_schema: json!({
                "type": "object",
                "properties": {
                    "source_path": {
                        "type": "string",
                        "title": "Source Folder",
                        "description": "Directory to scan for files that should be classified."
                    },
                    "target_root": {
                        "type": "string",
                        "title": "Target Root",
                        "description": "Destination root where categorized folders will be created."
                    },
                    "naming_strategy": {
                        "type": "string",
                        "enum": ["preserve", "normalize"],
                        "default": "preserve"
                    },
                    "exclude_patterns": {
                        "type": "array",
                        "items": { "type": "string" },
                        "default": []
                    }
                },
                "required": ["source_path", "target_root"]
            }),
            default_mode: TaskMode::Ask,
            output_expectations: json!({
                "kind": "file_operation_plan",
                "summary": "Structured preview of rename and move operations before execution.",
                "artifacts": [
                    { "type": "operation_preview" },
                    { "type": "result_summary" }
                ]
            }),
            builtin: true,
            mutable: false,
            clonable: true,
            created_at: None,
            updated_at: None,
        },
        TaskTemplateRecord {
            id: "builtin:periodic-briefing".to_string(),
            name: "Periodic Briefing".to_string(),
            description: "Synthesize MCP and local workspace sources into a Markdown briefing that can run once or on a schedule.".to_string(),
            parameter_schema: json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "title": "Briefing Title"
                    },
                    "sources": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "kind": { "type": "string", "enum": ["mcp", "workspace"] },
                                "target": { "type": "string" }
                            },
                            "required": ["kind", "target"]
                        }
                    },
                    "output_path": {
                        "type": "string",
                        "title": "Output Markdown Path"
                    },
                    "schedule": {
                        "type": "string",
                        "title": "Cron Expression"
                    }
                },
                "required": ["title", "sources", "output_path"]
            }),
            default_mode: TaskMode::Ask,
            output_expectations: json!({
                "kind": "markdown_report",
                "summary": "Writes a deterministic Markdown briefing and records file output metadata.",
                "artifacts": [
                    { "type": "markdown_file" },
                    { "type": "task_result" }
                ]
            }),
            builtin: true,
            mutable: false,
            clonable: true,
            created_at: None,
            updated_at: None,
        },
    ]
}

pub fn builtin_template(template_id: &str) -> Option<TaskTemplateRecord> {
    builtin_templates()
        .into_iter()
        .find(|template| template.id == template_id)
}

pub fn validate_template_draft(
    name: &str,
    description: &str,
    parameter_schema: &Value,
    default_mode: &str,
    output_expectations: &Value,
) -> Result<TaskTemplateDraft, TemplateValidationError> {
    let mut field_errors = HashMap::new();

    let normalized_name = name.trim();
    if normalized_name.is_empty() {
        field_errors.insert("name".to_string(), "name is required".to_string());
    }

    let normalized_description = description.trim();

    let parsed_mode = match default_mode {
        "ask" => Some(TaskMode::Ask),
        "yolo" => Some(TaskMode::Yolo),
        _ => {
            field_errors.insert(
                "default_mode".to_string(),
                "default_mode must be \"ask\" or \"yolo\"".to_string(),
            );
            None
        }
    };

    if !parameter_schema.is_object() {
        field_errors.insert(
            "parameter_schema".to_string(),
            "parameter_schema must be an object".to_string(),
        );
    } else {
        let schema_type = parameter_schema.get("type").and_then(Value::as_str);
        if schema_type != Some("object") {
            field_errors.insert(
                "parameter_schema.type".to_string(),
                "parameter_schema.type must be \"object\"".to_string(),
            );
        }
        let properties = parameter_schema.get("properties");
        if !matches!(properties, Some(Value::Object(_))) {
            field_errors.insert(
                "parameter_schema.properties".to_string(),
                "parameter_schema.properties must be an object".to_string(),
            );
        }
    }

    if !output_expectations.is_object() {
        field_errors.insert(
            "output_expectations".to_string(),
            "output_expectations must be an object".to_string(),
        );
    }

    if !field_errors.is_empty() {
        return Err(TemplateValidationError {
            message: "invalid template definition".to_string(),
            field_errors,
        });
    }

    Ok(TaskTemplateDraft {
        name: normalized_name.to_string(),
        description: normalized_description.to_string(),
        parameter_schema: parameter_schema.clone(),
        default_mode: parsed_mode.expect("validated default_mode"),
        output_expectations: output_expectations.clone(),
    })
}
