use async_trait::async_trait;
use libsql::params;

use super::{LibSqlBackend, fmt_ts, get_json, get_text, get_ts};
use crate::db::TemplateStore;
use crate::error::DatabaseError;
use crate::task_runtime::TaskMode;
use crate::task_templates::TaskTemplateRecord;

use chrono::Utc;

#[async_trait]
impl TemplateStore for LibSqlBackend {
    async fn list_task_templates(
        &self,
        user_id: &str,
    ) -> Result<Vec<TaskTemplateRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, name, description, parameter_schema, default_mode, output_expectations, created_at, updated_at
                FROM task_templates
                WHERE user_id = ?1
                ORDER BY updated_at DESC, name ASC
                "#,
                params![user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut templates = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            templates.push(row_to_template(&row));
        }

        Ok(templates)
    }

    async fn get_task_template(
        &self,
        user_id: &str,
        id: &str,
    ) -> Result<Option<TaskTemplateRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, name, description, parameter_schema, default_mode, output_expectations, created_at, updated_at
                FROM task_templates
                WHERE user_id = ?1 AND id = ?2
                "#,
                params![user_id, id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_template(&row))),
            None => Ok(None),
        }
    }

    async fn create_task_template(
        &self,
        user_id: &str,
        template: &TaskTemplateRecord,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            r#"
            INSERT INTO task_templates (
                id, user_id, name, description, parameter_schema, default_mode, output_expectations, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
            "#,
            params![
                template.id.as_str(),
                user_id,
                template.name.as_str(),
                template.description.as_str(),
                template.parameter_schema.to_string(),
                serialize_mode(template.default_mode),
                template.output_expectations.to_string(),
                now.as_str(),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn update_task_template(
        &self,
        user_id: &str,
        template: &TaskTemplateRecord,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let updated = conn
            .execute(
                r#"
                UPDATE task_templates
                SET
                    name = ?3,
                    description = ?4,
                    parameter_schema = ?5,
                    default_mode = ?6,
                    output_expectations = ?7,
                    updated_at = ?8
                WHERE user_id = ?1 AND id = ?2
                "#,
                params![
                    user_id,
                    template.id.as_str(),
                    template.name.as_str(),
                    template.description.as_str(),
                    template.parameter_schema.to_string(),
                    serialize_mode(template.default_mode),
                    template.output_expectations.to_string(),
                    fmt_ts(&Utc::now()),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(updated > 0)
    }

    async fn delete_task_template(&self, user_id: &str, id: &str) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let deleted = conn
            .execute(
                "DELETE FROM task_templates WHERE user_id = ?1 AND id = ?2",
                params![user_id, id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(deleted > 0)
    }
}

fn row_to_template(row: &libsql::Row) -> TaskTemplateRecord {
    TaskTemplateRecord::from_user_row(
        get_text(row, 0),
        get_text(row, 1),
        get_text(row, 2),
        get_json(row, 3),
        parse_mode(&get_text(row, 4)),
        get_json(row, 5),
        get_ts(row, 6),
        get_ts(row, 7),
    )
}

fn parse_mode(value: &str) -> TaskMode {
    match value {
        "yolo" => TaskMode::Yolo,
        _ => TaskMode::Ask,
    }
}

fn serialize_mode(mode: TaskMode) -> &'static str {
    match mode {
        TaskMode::Ask => "ask",
        TaskMode::Yolo => "yolo",
    }
}
