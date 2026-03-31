use async_trait::async_trait;
use libsql::params;

use super::{LibSqlBackend, fmt_ts, get_opt_text, get_text, get_ts};
use crate::db::TaskStore;
use crate::error::DatabaseError;
use crate::task_runtime::{
    TaskCurrentStep, TaskMode, TaskPendingApproval, TaskRecord, TaskStatus, TaskTimelineEntry,
};

#[async_trait]
impl TaskStore for LibSqlBackend {
    async fn upsert_task_record(
        &self,
        user_id: &str,
        task: &TaskRecord,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT INTO task_records (
                id, user_id, template_id, mode, status, title, created_at, updated_at,
                current_step, pending_approval, route, last_error, result_metadata
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            ON CONFLICT (id) DO UPDATE SET
                template_id = excluded.template_id,
                mode = excluded.mode,
                status = excluded.status,
                title = excluded.title,
                updated_at = excluded.updated_at,
                current_step = excluded.current_step,
                pending_approval = excluded.pending_approval,
                route = excluded.route,
                last_error = excluded.last_error,
                result_metadata = excluded.result_metadata
            "#,
            params![
                task.id.to_string(),
                user_id,
                task.template_id.as_str(),
                task.mode.as_str(),
                task.status.as_str(),
                task.title.as_str(),
                fmt_ts(&task.created_at),
                fmt_ts(&task.updated_at),
                json_text(task.current_step.as_ref()),
                json_text(task.pending_approval.as_ref()),
                serde_json::to_string(&task.route)
                    .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
                task.last_error.clone(),
                optional_value_text(task.result_metadata.as_ref()),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_task_record(
        &self,
        user_id: &str,
        task_id: uuid::Uuid,
    ) -> Result<Option<TaskRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, template_id, mode, status, title, created_at, updated_at,
                       current_step, pending_approval, route, last_error, result_metadata
                FROM task_records
                WHERE user_id = ?1 AND id = ?2
                "#,
                params![user_id, task_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_task(&row)?)),
            None => Ok(None),
        }
    }

    async fn list_task_records(&self, user_id: &str) -> Result<Vec<TaskRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, template_id, mode, status, title, created_at, updated_at,
                       current_step, pending_approval, route, last_error, result_metadata
                FROM task_records
                WHERE user_id = ?1
                ORDER BY updated_at DESC
                "#,
                params![user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut tasks = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            tasks.push(row_to_task(&row)?);
        }
        Ok(tasks)
    }

    async fn append_task_timeline(
        &self,
        user_id: &str,
        task_id: uuid::Uuid,
        event: &str,
        task: &TaskRecord,
        metadata: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT INTO task_timeline_events (
                user_id, task_id, event, status, mode, current_step, pending_approval,
                last_error, result_metadata, metadata, created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
            params![
                user_id,
                task_id.to_string(),
                event,
                task.status.as_str(),
                task.mode.as_str(),
                json_text(task.current_step.as_ref()),
                json_text(task.pending_approval.as_ref()),
                task.last_error.clone(),
                optional_value_text(task.result_metadata.as_ref()),
                metadata.to_string(),
                fmt_ts(&task.updated_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_task_timeline(
        &self,
        user_id: &str,
        task_id: uuid::Uuid,
    ) -> Result<Vec<TaskTimelineEntry>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, event, status, mode, current_step, pending_approval,
                       last_error, result_metadata, metadata, created_at
                FROM task_timeline_events
                WHERE user_id = ?1 AND task_id = ?2
                ORDER BY id ASC
                "#,
                params![user_id, task_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut timeline = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            timeline.push(TaskTimelineEntry {
                sequence: row.get::<i64>(0).unwrap_or_default() as u64,
                event: get_text(&row, 1),
                status: TaskStatus::from_str(&get_text(&row, 2)),
                mode: TaskMode::from_str(&get_text(&row, 3)),
                current_step: parse_optional_json::<TaskCurrentStep>(&row, 4)?,
                pending_approval: parse_optional_json::<TaskPendingApproval>(&row, 5)?,
                last_error: get_opt_text(&row, 6),
                result_metadata: parse_optional_value(&row, 7)?,
                created_at: get_ts(&row, 9),
            });
        }
        Ok(timeline)
    }
}

fn row_to_task(row: &libsql::Row) -> Result<TaskRecord, DatabaseError> {
    Ok(TaskRecord {
        id: get_text(row, 0)
            .parse()
            .map_err(|e| DatabaseError::Serialization(format!("invalid task id: {e}")))?,
        template_id: get_text(row, 1),
        mode: TaskMode::from_str(&get_text(row, 2)),
        status: TaskStatus::from_str(&get_text(row, 3)),
        title: get_text(row, 4),
        created_at: get_ts(row, 5),
        updated_at: get_ts(row, 6),
        current_step: parse_optional_json::<TaskCurrentStep>(row, 7)?,
        pending_approval: parse_optional_json::<TaskPendingApproval>(row, 8)?,
        route: parse_json_text(&get_text(row, 9))?,
        last_error: get_opt_text(row, 10),
        result_metadata: parse_optional_value(row, 11)?,
    })
}

fn json_text<T: serde::Serialize>(value: Option<&T>) -> libsql::Value {
    match value {
        Some(value) => match serde_json::to_string(value) {
            Ok(text) => libsql::Value::Text(text),
            Err(_) => libsql::Value::Null,
        },
        None => libsql::Value::Null,
    }
}

fn optional_value_text(value: Option<&serde_json::Value>) -> libsql::Value {
    match value {
        Some(value) => libsql::Value::Text(value.to_string()),
        None => libsql::Value::Null,
    }
}

fn parse_optional_json<T: serde::de::DeserializeOwned>(
    row: &libsql::Row,
    idx: i32,
) -> Result<Option<T>, DatabaseError> {
    match get_opt_text(row, idx) {
        Some(text) => parse_json_text(&text).map(Some),
        None => Ok(None),
    }
}

fn parse_optional_value(
    row: &libsql::Row,
    idx: i32,
) -> Result<Option<serde_json::Value>, DatabaseError> {
    match get_opt_text(row, idx) {
        Some(text) => parse_json_text(&text).map(Some),
        None => Ok(None),
    }
}

fn parse_json_text<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, DatabaseError> {
    serde_json::from_str(text).map_err(|e| DatabaseError::Serialization(e.to_string()))
}
