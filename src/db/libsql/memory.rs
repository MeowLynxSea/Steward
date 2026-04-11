use std::collections::HashSet;

use async_trait::async_trait;
use chrono::Utc;
use libsql::params;
use uuid::Uuid;

use crate::db::libsql::{fmt_ts, get_json, get_opt_text, get_text, get_ts, opt_text};
use crate::db::{MemoryStore, libsql::LibSqlBackend};
use crate::error::DatabaseError;
use crate::memory::search_terms::{
    build_search_terms, matched_keywords as collect_matched_keywords, sqlite_match_query,
};
use crate::memory::{
    CreateMemoryAliasInput, MemoryChangeSet, MemoryChangeSetRow, MemoryChildEntry, MemoryEdge,
    MemoryGlossaryEntry, MemoryIndexEntry, MemoryNode, MemoryNodeDetail, MemoryNodeKind,
    MemoryRelationKind, MemoryRoute, MemorySearchDoc, MemorySearchHit, MemorySidebarItem,
    MemorySidebarSection, MemorySpace, MemoryTimelineEntry, MemoryVersion, MemoryVersionStatus,
    MemoryVisibility, NewMemoryNodeInput, UpdateMemoryNodeInput,
};

fn row_to_memory_space(row: &libsql::Row) -> MemorySpace {
    MemorySpace {
        id: get_text(row, 0).parse().unwrap_or_default(),
        owner_id: get_text(row, 1),
        agent_id: get_opt_text(row, 2).and_then(|value| value.parse().ok()),
        slug: get_text(row, 3),
        title: get_text(row, 4),
        created_at: get_ts(row, 5),
        updated_at: get_ts(row, 6),
    }
}

fn row_to_memory_node(row: &libsql::Row) -> MemoryNode {
    MemoryNode {
        id: get_text(row, 0).parse().unwrap_or_default(),
        space_id: get_text(row, 1).parse().unwrap_or_default(),
        kind: MemoryNodeKind::from_str(&get_text(row, 2)),
        title: get_text(row, 3),
        metadata: get_json(row, 4),
        created_at: get_ts(row, 5),
        updated_at: get_ts(row, 6),
    }
}

fn row_to_memory_version(row: &libsql::Row) -> MemoryVersion {
    MemoryVersion {
        id: get_text(row, 0).parse().unwrap_or_default(),
        node_id: get_text(row, 1).parse().unwrap_or_default(),
        supersedes_version_id: get_opt_text(row, 2).and_then(|value| value.parse().ok()),
        status: MemoryVersionStatus::from_str(&get_text(row, 3)),
        content: get_text(row, 4),
        metadata: get_json(row, 5),
        created_at: get_ts(row, 6),
    }
}

fn row_to_memory_edge(row: &libsql::Row) -> MemoryEdge {
    MemoryEdge {
        id: get_text(row, 0).parse().unwrap_or_default(),
        space_id: get_text(row, 1).parse().unwrap_or_default(),
        parent_node_id: get_opt_text(row, 2).and_then(|value| value.parse().ok()),
        child_node_id: get_text(row, 3).parse().unwrap_or_default(),
        relation_kind: MemoryRelationKind::from_str(&get_text(row, 4)),
        visibility: MemoryVisibility::from_str(&get_text(row, 5)),
        priority: row.get::<i64>(6).unwrap_or(100) as i32,
        trigger_text: get_opt_text(row, 7),
        created_at: get_ts(row, 8),
        updated_at: get_ts(row, 9),
    }
}

fn row_to_memory_route(row: &libsql::Row) -> MemoryRoute {
    MemoryRoute {
        id: get_text(row, 0).parse().unwrap_or_default(),
        space_id: get_text(row, 1).parse().unwrap_or_default(),
        edge_id: get_opt_text(row, 2).and_then(|value| value.parse().ok()),
        node_id: get_text(row, 3).parse().unwrap_or_default(),
        domain: get_text(row, 4),
        path: get_text(row, 5),
        is_primary: row.get::<i64>(6).unwrap_or_default() != 0,
        created_at: get_ts(row, 7),
        updated_at: get_ts(row, 8),
    }
}

fn row_to_memory_keyword(row: &libsql::Row) -> crate::memory::MemoryKeyword {
    crate::memory::MemoryKeyword {
        id: get_text(row, 0).parse().unwrap_or_default(),
        space_id: get_text(row, 1).parse().unwrap_or_default(),
        node_id: get_text(row, 2).parse().unwrap_or_default(),
        keyword: get_text(row, 3),
        created_at: get_ts(row, 4),
    }
}

fn row_to_memory_search_doc(row: &libsql::Row) -> MemorySearchDoc {
    MemorySearchDoc {
        route_id: get_text(row, 0).parse().unwrap_or_default(),
        space_id: get_text(row, 1).parse().unwrap_or_default(),
        node_id: get_text(row, 2).parse().unwrap_or_default(),
        version_id: get_text(row, 3).parse().unwrap_or_default(),
        uri: get_text(row, 4),
        title: get_text(row, 5),
        kind: MemoryNodeKind::from_str(&get_text(row, 6)),
        content: get_text(row, 7),
        trigger_text: get_opt_text(row, 8),
        keywords: split_keywords(&get_text(row, 9)),
        search_terms: get_text(row, 10),
        updated_at: get_ts(row, 11),
    }
}

fn row_to_memory_changeset(row: &libsql::Row) -> MemoryChangeSet {
    MemoryChangeSet {
        id: get_text(row, 0).parse().unwrap_or_default(),
        space_id: get_text(row, 1).parse().unwrap_or_default(),
        origin: get_text(row, 2),
        summary: get_opt_text(row, 3),
        status: get_text(row, 4),
        created_at: get_ts(row, 5),
        updated_at: get_ts(row, 6),
    }
}

fn row_to_changeset_row(row: &libsql::Row) -> MemoryChangeSetRow {
    MemoryChangeSetRow {
        id: get_text(row, 0).parse().unwrap_or_default(),
        changeset_id: get_text(row, 1).parse().unwrap_or_default(),
        node_id: get_opt_text(row, 2).and_then(|value| value.parse().ok()),
        route_id: get_opt_text(row, 3).and_then(|value| value.parse().ok()),
        operation: get_text(row, 4),
        before_json: get_json(row, 5),
        after_json: get_json(row, 6),
        created_at: get_ts(row, 7),
    }
}

fn snippet(content: &str, query: &str) -> String {
    if content.is_empty() {
        return String::new();
    }
    let lower = content.to_lowercase();
    let query_lower = query.to_lowercase();
    let idx = lower.find(&query_lower).unwrap_or(0);
    let start = floor_char_boundary(content, idx.saturating_sub(48));
    let end = ceil_char_boundary(content, (idx + query.len() + 96).min(content.len()));
    let prefix = if start > 0 { "..." } else { "" };
    let suffix = if end < content.len() { "..." } else { "" };
    format!("{prefix}{}{suffix}", &content[start..end])
}

fn floor_char_boundary(value: &str, mut index: usize) -> usize {
    index = index.min(value.len());
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(value: &str, mut index: usize) -> usize {
    index = index.min(value.len());
    while index < value.len() && !value.is_char_boundary(index) {
        index += 1;
    }
    index
}

fn split_keywords(raw: &str) -> Vec<String> {
    raw.split_whitespace().map(ToString::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::snippet;

    #[test]
    fn snippet_respects_utf8_boundaries_for_cjk_content() {
        let content = "## 记忆系统 (The Native Memory System)\n\n你的长期记忆托管于 **Steward 的原生记忆图谱工具**。这是你与用户共同使用的层级化树状记忆系统。";
        let result = snippet(content, "原生记忆");
        assert!(result.contains("原生记忆"));
    }
}

impl LibSqlBackend {
    async fn fetch_memory_route_by_path(
        &self,
        space_id: Uuid,
        domain: &str,
        path: &str,
    ) -> Result<Option<MemoryRoute>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, space_id, edge_id, node_id, domain, path, is_primary, created_at, updated_at
                 FROM memory_routes WHERE space_id = ?1 AND domain = ?2 AND path = ?3",
                params![space_id.to_string(), domain, path],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
            .map(|row| row_to_memory_route(&row)))
    }

    async fn fetch_memory_route(
        &self,
        space_id: Uuid,
        route_or_node: &str,
    ) -> Result<Option<MemoryRoute>, DatabaseError> {
        if let Some((domain, path)) = route_or_node.split_once("://") {
            return self
                .fetch_memory_route_by_path(space_id, domain, path)
                .await;
        }
        Ok(None)
    }

    async fn fetch_memory_node_by_id(
        &self,
        node_id: Uuid,
    ) -> Result<Option<MemoryNode>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, space_id, kind, title, metadata, created_at, updated_at
                 FROM memory_nodes WHERE id = ?1",
                params![node_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
            .map(|row| row_to_memory_node(&row)))
    }

    async fn resolve_node_id(
        &self,
        space_id: Uuid,
        route_or_node: &str,
    ) -> Result<Option<Uuid>, DatabaseError> {
        if let Some(route) = self.fetch_memory_route(space_id, route_or_node).await? {
            return Ok(Some(route.node_id));
        }
        match Uuid::parse_str(route_or_node) {
            Ok(node_id) => Ok(self
                .fetch_memory_node_by_id(node_id)
                .await?
                .map(|node| node.id)),
            Err(_) => Ok(None),
        }
    }

    async fn current_memory_version(
        &self,
        node_id: Uuid,
    ) -> Result<Option<MemoryVersion>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, node_id, supersedes_version_id, status, content, metadata, created_at
                 FROM memory_versions
                 WHERE node_id = ?1 AND status IN ('active', 'orphaned')
                 ORDER BY CASE status WHEN 'active' THEN 0 ELSE 1 END, created_at DESC
                 LIMIT 1",
                params![node_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
            .map(|row| row_to_memory_version(&row)))
    }

    async fn fetch_memory_routes_for_node(
        &self,
        node_id: Uuid,
    ) -> Result<Vec<MemoryRoute>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, space_id, edge_id, node_id, domain, path, is_primary, created_at, updated_at
                 FROM memory_routes
                 WHERE node_id = ?1
                 ORDER BY is_primary DESC, domain, path",
                params![node_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut routes = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            routes.push(row_to_memory_route(&row));
        }
        Ok(routes)
    }

    async fn fetch_memory_edges_for_node(
        &self,
        node_id: Uuid,
    ) -> Result<Vec<MemoryEdge>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, space_id, parent_node_id, child_node_id, relation_kind, visibility, priority, trigger_text, created_at, updated_at
                 FROM memory_edges
                 WHERE child_node_id = ?1
                 ORDER BY priority ASC, created_at ASC",
                params![node_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut edges = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            edges.push(row_to_memory_edge(&row));
        }
        Ok(edges)
    }

    async fn fetch_memory_routes_by_prefix(
        &self,
        space_id: Uuid,
        domain: &str,
        path: &str,
    ) -> Result<Vec<MemoryRoute>, DatabaseError> {
        let conn = self.connect().await?;
        let like = format!("{path}/%");
        let mut rows = conn
            .query(
                "SELECT id, space_id, edge_id, node_id, domain, path, is_primary, created_at, updated_at
                 FROM memory_routes
                 WHERE space_id = ?1
                   AND domain = ?2
                   AND (path = ?3 OR path LIKE ?4)
                 ORDER BY length(path) DESC, path DESC",
                params![space_id.to_string(), domain, path, like],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut routes = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            routes.push(row_to_memory_route(&row));
        }
        Ok(routes)
    }

    async fn count_routes_for_edge(&self, edge_id: Uuid) -> Result<u64, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT count(*) FROM memory_routes WHERE edge_id = ?1",
                params![edge_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let count = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
            .and_then(|row| row.get::<i64>(0).ok())
            .unwrap_or_default();
        Ok(count as u64)
    }

    async fn find_contains_edge(
        &self,
        space_id: Uuid,
        parent_node_id: Option<Uuid>,
        child_node_id: Uuid,
    ) -> Result<Option<MemoryEdge>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = if let Some(parent_node_id) = parent_node_id {
            conn.query(
                "SELECT id, space_id, parent_node_id, child_node_id, relation_kind, visibility, priority, trigger_text, created_at, updated_at
                 FROM memory_edges
                 WHERE space_id = ?1 AND parent_node_id = ?2 AND child_node_id = ?3 AND relation_kind = 'contains'
                 LIMIT 1",
                params![
                    space_id.to_string(),
                    parent_node_id.to_string(),
                    child_node_id.to_string()
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        } else {
            conn.query(
                "SELECT id, space_id, parent_node_id, child_node_id, relation_kind, visibility, priority, trigger_text, created_at, updated_at
                 FROM memory_edges
                 WHERE space_id = ?1 AND parent_node_id IS NULL AND child_node_id = ?2 AND relation_kind = 'contains'
                 LIMIT 1",
                params![space_id.to_string(), child_node_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        };
        Ok(rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
            .map(|row| row_to_memory_edge(&row)))
    }

    async fn create_route_snapshot(
        &self,
        space_id: Uuid,
        edge_id: Uuid,
        node_id: Uuid,
        domain: &str,
        path: &str,
        is_primary: bool,
    ) -> Result<MemoryRoute, DatabaseError> {
        let now = Utc::now();
        let route = MemoryRoute {
            id: Uuid::new_v4(),
            space_id,
            edge_id: Some(edge_id),
            node_id,
            domain: domain.to_string(),
            path: path.to_string(),
            is_primary,
            created_at: now,
            updated_at: now,
        };
        self.upsert_route_snapshot(&route).await?;
        Ok(route)
    }

    async fn would_create_cycle(
        &self,
        parent_node_id: Uuid,
        child_node_id: Uuid,
    ) -> Result<bool, DatabaseError> {
        if parent_node_id == child_node_id {
            return Ok(true);
        }

        let conn = self.connect().await?;
        let mut visited = HashSet::from([child_node_id]);
        let mut queue = vec![child_node_id];
        while let Some(current) = queue.pop() {
            let mut rows = conn
                .query(
                    "SELECT child_node_id
                     FROM memory_edges
                     WHERE parent_node_id = ?1 AND relation_kind = 'contains'",
                    params![current.to_string()],
                )
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?;
            while let Some(row) = rows
                .next()
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?
            {
                let Some(next) = get_opt_text(&row, 0).and_then(|value| value.parse::<Uuid>().ok())
                else {
                    continue;
                };
                if next == parent_node_id {
                    return Ok(true);
                }
                if visited.insert(next) {
                    queue.push(next);
                }
            }
        }
        Ok(false)
    }

    async fn cascade_create_alias_routes(
        &self,
        space_id: Uuid,
        node_id: Uuid,
        domain: &str,
        base_path: &str,
        visited: &mut HashSet<Uuid>,
        created_routes: &mut Vec<MemoryRoute>,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let mut stack = vec![(node_id, base_path.to_string(), visited.clone())];
        while let Some((current_node_id, current_base_path, mut current_visited)) = stack.pop() {
            if !current_visited.insert(current_node_id) {
                continue;
            }

            let mut rows = conn
                .query(
                    "SELECT e.id, e.child_node_id
                     FROM memory_edges e
                     WHERE e.space_id = ?1 AND e.parent_node_id = ?2 AND e.relation_kind = 'contains'
                     ORDER BY e.priority ASC, e.created_at ASC",
                    params![space_id.to_string(), current_node_id.to_string()],
                )
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?;

            let mut next_items = Vec::new();
            while let Some(row) = rows
                .next()
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?
            {
                let edge_id: Uuid = get_text(&row, 0).parse().unwrap_or_default();
                let child_node_id: Uuid = get_text(&row, 1).parse().unwrap_or_default();

                let mut route_rows = conn
                    .query(
                        "SELECT id, space_id, edge_id, node_id, domain, path, is_primary, created_at, updated_at
                         FROM memory_routes
                         WHERE edge_id = ?1
                         ORDER BY is_primary DESC, updated_at DESC
                         LIMIT 1",
                        params![edge_id.to_string()],
                    )
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
                let Some(route_row) = route_rows
                    .next()
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?
                else {
                    continue;
                };
                let existing_route = row_to_memory_route(&route_row);
                let Some(segment) = existing_route.path.rsplit('/').next() else {
                    continue;
                };
                let child_path = format!("{current_base_path}/{segment}");

                if self
                    .fetch_memory_route_by_path(space_id, domain, &child_path)
                    .await?
                    .is_none()
                {
                    let route = self
                        .create_route_snapshot(
                            space_id,
                            edge_id,
                            child_node_id,
                            domain,
                            &child_path,
                            false,
                        )
                        .await?;
                    self.rebuild_search_doc_for_route(route.id).await?;
                    created_routes.push(route);
                }

                next_items.push((child_node_id, child_path, current_visited.clone()));
            }

            for item in next_items.into_iter().rev() {
                stack.push(item);
            }
        }

        Ok(())
    }

    async fn fetch_keywords_for_node(
        &self,
        node_id: Uuid,
    ) -> Result<Vec<crate::memory::MemoryKeyword>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, space_id, node_id, keyword, created_at
                 FROM memory_keywords
                 WHERE node_id = ?1
                 ORDER BY keyword",
                params![node_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut keywords = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            keywords.push(row_to_memory_keyword(&row));
        }
        Ok(keywords)
    }

    async fn write_changeset_row(
        &self,
        changeset_id: Uuid,
        node_id: Option<Uuid>,
        route_id: Option<Uuid>,
        operation: &str,
        before_json: &serde_json::Value,
        after_json: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO memory_changeset_rows
             (id, changeset_id, node_id, route_id, operation, before_json, after_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id.to_string(),
                changeset_id.to_string(),
                opt_text(node_id.map(|value| value.to_string()).as_deref()),
                opt_text(route_id.map(|value| value.to_string()).as_deref()),
                operation,
                before_json.to_string(),
                after_json.to_string()
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn rebuild_search_doc_for_route(&self, route_id: Uuid) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT r.id, r.space_id, r.node_id, r.domain, r.path, n.title, n.kind,
                        v.id, v.content, e.trigger_text, e.priority, n.updated_at
                 FROM memory_routes r
                 JOIN memory_nodes n ON n.id = r.node_id
                 JOIN memory_versions v ON v.node_id = n.id AND v.status = 'active'
                 LEFT JOIN memory_edges e ON e.id = r.edge_id
                 WHERE r.id = ?1
                 LIMIT 1",
                params![route_id.to_string()],
            )
            .await
            .map_err(|e| {
                DatabaseError::Query(format!(
                    "rebuild_search_doc_for_route: select route detail failed: {e}"
                ))
            })?;
        let Some(row) = rows.next().await.map_err(|e| {
            DatabaseError::Query(format!(
                "rebuild_search_doc_for_route: read route detail row failed: {e}"
            ))
        })?
        else {
            let _ = conn
                .execute(
                    "DELETE FROM memory_search_docs WHERE route_id = ?1",
                    params![route_id.to_string()],
                )
                .await;
            return Ok(());
        };

        let node_id = get_text(&row, 2);
        let mut kw_rows = conn
            .query(
                "SELECT keyword FROM memory_keywords WHERE node_id = ?1 ORDER BY keyword",
                params![node_id.clone()],
            )
            .await
            .map_err(|e| {
                DatabaseError::Query(format!(
                    "rebuild_search_doc_for_route: select keywords failed: {e}"
                ))
            })?;
        let mut keywords = Vec::new();
        while let Some(kw_row) = kw_rows.next().await.map_err(|e| {
            DatabaseError::Query(format!(
                "rebuild_search_doc_for_route: read keyword row failed: {e}"
            ))
        })? {
            keywords.push(get_text(&kw_row, 0));
        }

        let domain = get_text(&row, 3);
        let path = get_text(&row, 4);
        let uri = format!("{domain}://{path}");
        let title = get_text(&row, 5);
        let kind = get_text(&row, 6);
        let content = get_text(&row, 8);
        let trigger_text = get_opt_text(&row, 9);
        let keyword_text = keywords.join(" ");
        let search_terms = build_search_terms(&[
            title.as_str(),
            path.as_str(),
            uri.as_str(),
            content.as_str(),
            trigger_text.as_deref().unwrap_or(""),
            keyword_text.as_str(),
        ]);
        let _rows_affected = conn
            .execute(
            "INSERT INTO memory_search_docs
             (route_id, space_id, node_id, version_id, uri, title, kind, content, trigger_text, keywords, search_terms, embedding, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, ?12)
             ON CONFLICT(route_id) DO UPDATE SET
                 version_id = excluded.version_id,
                 uri = excluded.uri,
                 title = excluded.title,
                 kind = excluded.kind,
                 content = excluded.content,
                 trigger_text = excluded.trigger_text,
                 keywords = excluded.keywords,
                 search_terms = excluded.search_terms,
                 embedding = NULL,
                 updated_at = excluded.updated_at",
            params![
                get_text(&row, 0),
                get_text(&row, 1),
                node_id,
                get_text(&row, 7),
                uri,
                title,
                kind,
                content,
                opt_text(trigger_text.as_deref()),
                keyword_text,
                search_terms,
                get_text(&row, 11)
            ],
        )
        .await
        .map_err(|e| {
            DatabaseError::Query(format!(
                "rebuild_search_doc_for_route: upsert memory_search_docs failed: {e}"
            ))
        })?;
        Ok(())
    }

    async fn fetch_detail(&self, node_id: Uuid) -> Result<Option<MemoryNodeDetail>, DatabaseError> {
        let Some(node) = self.fetch_memory_node_by_id(node_id).await? else {
            return Ok(None);
        };
        let Some(active_version) = self.current_memory_version(node_id).await? else {
            return Ok(None);
        };
        let routes = self.fetch_memory_routes_for_node(node_id).await?;
        let edges = self.fetch_memory_edges_for_node(node_id).await?;
        let keywords = self.fetch_keywords_for_node(node_id).await?;
        let related_nodes = self
            .search_memory_graph(node.space_id, &node.title, 4, &[])
            .await?
            .into_iter()
            .filter(|hit| hit.node_id != node_id)
            .collect::<Vec<_>>();
        let primary_route = routes
            .iter()
            .find(|route| route.is_primary)
            .cloned()
            .or_else(|| routes.first().cloned());
        Ok(Some(MemoryNodeDetail {
            node,
            active_version,
            primary_route,
            selected_route: None,
            selected_uri: None,
            routes,
            edges,
            keywords,
            related_nodes,
        }))
    }

    async fn fetch_changeset_rows_internal(
        &self,
        changeset_id: Uuid,
    ) -> Result<Vec<MemoryChangeSetRow>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, changeset_id, node_id, route_id, operation, before_json, after_json, created_at
                 FROM memory_changeset_rows
                 WHERE changeset_id = ?1
                 ORDER BY created_at ASC",
                params![changeset_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            items.push(row_to_changeset_row(&row));
        }
        Ok(items)
    }

    async fn upsert_memory_node_snapshot(&self, node: &MemoryNode) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "INSERT INTO memory_nodes (id, space_id, kind, title, metadata, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                 kind = excluded.kind,
                 title = excluded.title,
                 metadata = excluded.metadata,
                 updated_at = excluded.updated_at",
            params![
                node.id.to_string(),
                node.space_id.to_string(),
                node.kind.as_str(),
                node.title.as_str(),
                node.metadata.to_string(),
                fmt_ts(&node.created_at),
                fmt_ts(&node.updated_at)
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn restore_active_version_snapshot(
        &self,
        version: &MemoryVersion,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "UPDATE memory_versions
             SET status = CASE
                 WHEN id = ?2 THEN 'active'
                 WHEN status = 'orphaned' THEN status
                 ELSE 'deprecated'
             END
             WHERE node_id = ?1",
            params![version.node_id.to_string(), version.id.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        conn.execute(
            "INSERT INTO memory_versions
             (id, node_id, supersedes_version_id, status, content, metadata, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                 supersedes_version_id = excluded.supersedes_version_id,
                 status = excluded.status,
                 content = excluded.content,
                 metadata = excluded.metadata,
                 created_at = excluded.created_at",
            params![
                version.id.to_string(),
                version.node_id.to_string(),
                opt_text(
                    version
                        .supersedes_version_id
                        .map(|value| value.to_string())
                        .as_deref()
                ),
                version.status.as_str(),
                version.content.as_str(),
                version.metadata.to_string(),
                fmt_ts(&version.created_at)
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn replace_keywords_snapshot(
        &self,
        space_id: Uuid,
        node_id: Uuid,
        keywords: &[crate::memory::MemoryKeyword],
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "DELETE FROM memory_keywords WHERE node_id = ?1",
            params![node_id.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        for keyword in keywords {
            conn.execute(
                "INSERT INTO memory_keywords (id, space_id, node_id, keyword, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET keyword = excluded.keyword",
                params![
                    keyword.id.to_string(),
                    space_id.to_string(),
                    node_id.to_string(),
                    keyword.keyword.as_str(),
                    fmt_ts(&keyword.created_at)
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        }
        Ok(())
    }

    async fn upsert_edge_snapshot(&self, edge: &MemoryEdge) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "INSERT INTO memory_edges
             (id, space_id, parent_node_id, child_node_id, relation_kind, visibility, priority, trigger_text, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
                 parent_node_id = excluded.parent_node_id,
                 child_node_id = excluded.child_node_id,
                 relation_kind = excluded.relation_kind,
                 visibility = excluded.visibility,
                 priority = excluded.priority,
                 trigger_text = excluded.trigger_text,
                 updated_at = excluded.updated_at",
            params![
                edge.id.to_string(),
                edge.space_id.to_string(),
                opt_text(edge.parent_node_id.map(|value| value.to_string()).as_deref()),
                edge.child_node_id.to_string(),
                edge.relation_kind.as_str(),
                edge.visibility.as_str(),
                edge.priority as i64,
                opt_text(edge.trigger_text.as_deref()),
                fmt_ts(&edge.created_at),
                fmt_ts(&edge.updated_at)
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn upsert_route_snapshot(&self, route: &MemoryRoute) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let _ = conn
            .execute(
                "DELETE FROM memory_routes
                 WHERE space_id = ?1 AND domain = ?2 AND path = ?3 AND id != ?4",
                params![
                    route.space_id.to_string(),
                    route.domain.as_str(),
                    route.path.as_str(),
                    route.id.to_string()
                ],
            )
            .await;
        conn.execute(
            "INSERT INTO memory_routes
             (id, space_id, edge_id, node_id, domain, path, is_primary, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                 edge_id = excluded.edge_id,
                 node_id = excluded.node_id,
                 domain = excluded.domain,
                 path = excluded.path,
                 is_primary = excluded.is_primary,
                 updated_at = excluded.updated_at",
            params![
                route.id.to_string(),
                route.space_id.to_string(),
                opt_text(route.edge_id.map(|value| value.to_string()).as_deref()),
                route.node_id.to_string(),
                route.domain.as_str(),
                route.path.as_str(),
                if route.is_primary { 1 } else { 0 },
                fmt_ts(&route.created_at),
                fmt_ts(&route.updated_at)
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn remove_route_by_id(&self, route_id: Uuid) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT edge_id FROM memory_routes WHERE id = ?1",
                params![route_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let edge_id = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
            .and_then(|row| get_opt_text(&row, 0))
            .and_then(|value| value.parse::<Uuid>().ok());
        let _ = conn
            .execute(
                "DELETE FROM memory_search_docs WHERE route_id = ?1",
                params![route_id.to_string()],
            )
            .await;
        conn.execute(
            "DELETE FROM memory_routes WHERE id = ?1",
            params![route_id.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        if let Some(edge_id) = edge_id {
            if self.count_routes_for_edge(edge_id).await? == 0 {
                let _ = conn
                    .execute(
                        "DELETE FROM memory_edges WHERE id = ?1",
                        params![edge_id.to_string()],
                    )
                    .await;
            }
        }
        Ok(())
    }

    async fn hard_delete_node_snapshot(&self, node_id: Uuid) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let _ = conn
            .execute(
                "DELETE FROM memory_search_docs WHERE node_id = ?1",
                params![node_id.to_string()],
            )
            .await;
        conn.execute(
            "DELETE FROM memory_keywords WHERE node_id = ?1",
            params![node_id.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        conn.execute(
            "DELETE FROM memory_routes WHERE node_id = ?1",
            params![node_id.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        conn.execute(
            "DELETE FROM memory_edges WHERE child_node_id = ?1 OR parent_node_id = ?1",
            params![node_id.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        conn.execute(
            "DELETE FROM memory_versions WHERE node_id = ?1",
            params![node_id.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        conn.execute(
            "DELETE FROM memory_nodes WHERE id = ?1",
            params![node_id.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn restore_detail_snapshot(
        &self,
        detail: &MemoryNodeDetail,
        route_scope: Option<Uuid>,
    ) -> Result<(), DatabaseError> {
        self.upsert_memory_node_snapshot(&detail.node).await?;
        self.restore_active_version_snapshot(&detail.active_version)
            .await?;

        let routes_to_restore = detail
            .routes
            .iter()
            .filter(|route| match route_scope {
                Some(scope) => scope == route.id,
                None => true,
            })
            .cloned()
            .collect::<Vec<_>>();
        let edge_ids = routes_to_restore
            .iter()
            .filter_map(|route| route.edge_id)
            .collect::<Vec<_>>();

        for edge in detail
            .edges
            .iter()
            .filter(|edge| edge_ids.contains(&edge.id))
        {
            self.upsert_edge_snapshot(edge).await?;
        }

        for route in &routes_to_restore {
            self.upsert_route_snapshot(route).await?;
            self.rebuild_search_doc_for_route(route.id).await?;
        }

        if route_scope.is_none() {
            self.replace_keywords_snapshot(detail.node.space_id, detail.node.id, &detail.keywords)
                .await?;
        }

        Ok(())
    }

    async fn rollback_changeset_row(&self, row: &MemoryChangeSetRow) -> Result<(), DatabaseError> {
        match row.operation.as_str() {
            "create" => {
                let detail: MemoryNodeDetail = serde_json::from_value(row.after_json.clone())
                    .map_err(|e| DatabaseError::Query(format!("invalid create snapshot: {e}")))?;
                self.hard_delete_node_snapshot(detail.node.id).await?;
            }
            "alias" => {
                let route: MemoryRoute = serde_json::from_value(row.after_json.clone())
                    .map_err(|e| DatabaseError::Query(format!("invalid alias snapshot: {e}")))?;
                self.remove_route_by_id(route.id).await?;
            }
            "update" => {
                let detail: MemoryNodeDetail = serde_json::from_value(row.before_json.clone())
                    .map_err(|e| DatabaseError::Query(format!("invalid update snapshot: {e}")))?;
                self.restore_detail_snapshot(&detail, None).await?;
            }
            "delete" => {
                let detail: MemoryNodeDetail = serde_json::from_value(row.before_json.clone())
                    .map_err(|e| DatabaseError::Query(format!("invalid delete snapshot: {e}")))?;
                let route_scope = if row.after_json.is_null() {
                    None
                } else {
                    Some(
                        serde_json::from_value::<MemoryRoute>(row.after_json.clone())
                            .map_err(|e| {
                                DatabaseError::Query(format!("invalid delete route snapshot: {e}"))
                            })?
                            .id,
                    )
                };
                self.restore_detail_snapshot(&detail, route_scope).await?;
            }
            other => {
                return Err(DatabaseError::Query(format!(
                    "unsupported memory rollback operation: {other}"
                )));
            }
        }
        Ok(())
    }
}

#[async_trait]
impl MemoryStore for LibSqlBackend {
    async fn ensure_memory_space(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        slug: &str,
        title: &str,
    ) -> Result<MemorySpace, DatabaseError> {
        let conn = self.connect().await?;
        let agent_id_str = agent_id.map(|value| value.to_string());
        let mut rows = conn
            .query(
                "SELECT id, owner_id, agent_id, slug, title, created_at, updated_at
                 FROM memory_spaces
                 WHERE owner_id = ?1 AND agent_id IS ?2 AND slug = ?3",
                params![owner_id, agent_id_str.as_deref(), slug],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        if let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            return Ok(row_to_memory_space(&row));
        }
        let id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO memory_spaces (id, owner_id, agent_id, slug, title)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                id.to_string(),
                owner_id,
                agent_id_str.as_deref(),
                slug,
                title
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut created = conn
            .query(
                "SELECT id, owner_id, agent_id, slug, title, created_at, updated_at
                 FROM memory_spaces WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let row = created
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
            .ok_or_else(|| DatabaseError::NotFound {
                entity: "memory_space".to_string(),
                id: id.to_string(),
            })?;
        Ok(row_to_memory_space(&row))
    }

    async fn create_memory_changeset(
        &self,
        space_id: Uuid,
        origin: &str,
        summary: Option<&str>,
    ) -> Result<MemoryChangeSet, DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO memory_changesets (id, space_id, origin, summary, status)
             VALUES (?1, ?2, ?3, ?4, 'pending')",
            params![id.to_string(), space_id.to_string(), origin, summary],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut rows = conn
            .query(
                "SELECT id, space_id, origin, summary, status, created_at, updated_at
                 FROM memory_changesets WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let row = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
            .ok_or_else(|| DatabaseError::NotFound {
                entity: "memory_changeset".to_string(),
                id: id.to_string(),
            })?;
        Ok(row_to_memory_changeset(&row))
    }

    async fn complete_memory_changeset(
        &self,
        changeset_id: Uuid,
        status: &str,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "UPDATE memory_changesets SET status = ?2, updated_at = ?3 WHERE id = ?1",
            params![changeset_id.to_string(), status, fmt_ts(&Utc::now())],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_memory_changeset_rows(
        &self,
        changeset_id: Uuid,
    ) -> Result<Vec<MemoryChangeSetRow>, DatabaseError> {
        self.fetch_changeset_rows_internal(changeset_id).await
    }

    async fn rollback_memory_changeset(&self, changeset_id: Uuid) -> Result<(), DatabaseError> {
        let rows = self.fetch_changeset_rows_internal(changeset_id).await?;
        for row in rows.iter().rev() {
            self.rollback_changeset_row(row).await?;
        }
        self.complete_memory_changeset(changeset_id, "rolled_back")
            .await
    }

    async fn create_memory_node(
        &self,
        input: &NewMemoryNodeInput,
    ) -> Result<MemoryNodeDetail, DatabaseError> {
        if self
            .fetch_memory_route_by_path(input.space_id, &input.domain, &input.path)
            .await?
            .is_some()
        {
            return Err(DatabaseError::Constraint(format!(
                "Path '{}://{}' already exists",
                input.domain, input.path
            )));
        }
        let conn = self.connect().await?;
        let now = Utc::now();
        let node_id = Uuid::new_v4();
        let version_id = Uuid::new_v4();
        let edge_id = Uuid::new_v4();
        let route_id = Uuid::new_v4();

        conn.execute(
            "INSERT INTO memory_nodes (id, space_id, kind, title, metadata, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![
                node_id.to_string(),
                input.space_id.to_string(),
                input.kind.as_str(),
                input.title.as_str(),
                input.metadata.to_string(),
                fmt_ts(&now)
            ],
        )
        .await
        .map_err(|e| {
            DatabaseError::Query(format!(
                "create_memory_node: insert memory_nodes failed: {e}"
            ))
        })?;

        conn.execute(
            "INSERT INTO memory_versions (id, node_id, supersedes_version_id, status, content, metadata, created_at)
             VALUES (?1, ?2, NULL, 'active', ?3, ?4, ?5)",
            params![
                version_id.to_string(),
                node_id.to_string(),
                input.content.as_str(),
                input.metadata.to_string(),
                fmt_ts(&now)
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("create_memory_node: insert memory_versions failed: {e}")))?;

        conn.execute(
            "INSERT INTO memory_edges
             (id, space_id, parent_node_id, child_node_id, relation_kind, visibility, priority, trigger_text, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            params![
                edge_id.to_string(),
                input.space_id.to_string(),
                opt_text(input.parent_node_id.map(|value| value.to_string()).as_deref()),
                node_id.to_string(),
                input.relation_kind.as_str(),
                input.visibility.as_str(),
                input.priority as i64,
                opt_text(input.trigger_text.as_deref()),
                fmt_ts(&now)
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("create_memory_node: insert memory_edges failed: {e}")))?;

        conn.execute(
            "INSERT INTO memory_routes
             (id, space_id, edge_id, node_id, domain, path, is_primary, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?7)",
            params![
                route_id.to_string(),
                input.space_id.to_string(),
                edge_id.to_string(),
                node_id.to_string(),
                input.domain.as_str(),
                input.path.as_str(),
                fmt_ts(&now)
            ],
        )
        .await
        .map_err(|e| {
            DatabaseError::Query(format!(
                "create_memory_node: insert memory_routes failed: {e}"
            ))
        })?;

        for keyword in input
            .keywords
            .iter()
            .filter(|keyword| !keyword.trim().is_empty())
        {
            conn.execute(
                "INSERT OR IGNORE INTO memory_keywords (id, space_id, node_id, keyword)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    Uuid::new_v4().to_string(),
                    input.space_id.to_string(),
                    node_id.to_string(),
                    keyword.trim()
                ],
            )
            .await
            .map_err(|e| {
                DatabaseError::Query(format!(
                    "create_memory_node: insert memory_keywords failed: {e}"
                ))
            })?;
        }

        self.rebuild_search_doc_for_route(route_id).await?;
        let detail = self
            .fetch_detail(node_id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound {
                entity: "memory_node".to_string(),
                id: node_id.to_string(),
            })?;
        if let Some(changeset_id) = input.changeset_id {
            self.write_changeset_row(
                changeset_id,
                Some(node_id),
                Some(route_id),
                "create",
                &serde_json::Value::Null,
                &serde_json::to_value(&detail).unwrap_or(serde_json::Value::Null),
            )
            .await?;
        }
        Ok(detail)
    }

    async fn update_memory_node(
        &self,
        space_id: Uuid,
        input: &UpdateMemoryNodeInput,
    ) -> Result<MemoryNodeDetail, DatabaseError> {
        let Some(before) = self.get_memory_node(space_id, &input.route_or_node).await? else {
            return Err(DatabaseError::NotFound {
                entity: "memory_node".to_string(),
                id: input.route_or_node.clone(),
            });
        };
        let conn = self.connect().await?;
        let now = Utc::now();

        if input.title.is_some() || input.metadata.is_some() || input.kind.is_some() {
            let title = input
                .title
                .clone()
                .unwrap_or_else(|| before.node.title.clone());
            let kind = input.kind.unwrap_or(before.node.kind);
            let metadata = input
                .metadata
                .clone()
                .unwrap_or_else(|| before.node.metadata.clone());
            conn.execute(
                "UPDATE memory_nodes SET title = ?2, kind = ?3, metadata = ?4, updated_at = ?5 WHERE id = ?1",
                params![
                    before.node.id.to_string(),
                    title,
                    kind.as_str(),
                    metadata.to_string(),
                    fmt_ts(&now)
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        }

        if let Some(content) = input.content.clone()
            && content != before.active_version.content
        {
            conn.execute(
                "UPDATE memory_versions
                 SET status = 'deprecated'
                 WHERE node_id = ?1 AND status IN ('active', 'orphaned')",
                params![before.node.id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
            let new_status = if before.routes.is_empty() {
                "orphaned"
            } else {
                "active"
            };
            conn.execute(
                "INSERT INTO memory_versions
                 (id, node_id, supersedes_version_id, status, content, metadata, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    Uuid::new_v4().to_string(),
                    before.node.id.to_string(),
                    before.active_version.id.to_string(),
                    new_status,
                    content,
                    input
                        .metadata
                        .clone()
                        .unwrap_or_else(|| before.active_version.metadata.clone())
                        .to_string(),
                    fmt_ts(&now)
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        }

        let target_route = self
            .fetch_memory_route(space_id, &input.route_or_node)
            .await?
            .or_else(|| before.primary_route.clone());
        let route_id_for_changeset = target_route
            .as_ref()
            .and_then(|route| route.edge_id.map(|_| route.id));

        if input.priority.is_some() || input.trigger_text.is_some() || input.visibility.is_some() {
            let target_route = target_route.ok_or_else(|| DatabaseError::NotFound {
                entity: "memory_route".to_string(),
                id: input.route_or_node.clone(),
            })?;
            let edge = before
                .edges
                .iter()
                .find(|edge| Some(edge.id) == target_route.edge_id)
                .cloned()
                .ok_or_else(|| DatabaseError::NotFound {
                    entity: "memory_edge".to_string(),
                    id: target_route
                        .edge_id
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "unknown".to_string()),
                })?;
            let trigger_text = input
                .trigger_text
                .clone()
                .unwrap_or_else(|| edge.trigger_text.clone());
            let visibility = input.visibility.unwrap_or(edge.visibility);
            let priority = input.priority.unwrap_or(edge.priority);
            conn.execute(
                "UPDATE memory_edges
                 SET priority = ?2, trigger_text = ?3, visibility = ?4, updated_at = ?5
                 WHERE id = ?1",
                params![
                    edge.id.to_string(),
                    priority as i64,
                    opt_text(trigger_text.as_deref()),
                    visibility.as_str(),
                    fmt_ts(&now)
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        }

        if let Some(keywords) = input.keywords.clone() {
            conn.execute(
                "DELETE FROM memory_keywords WHERE node_id = ?1",
                params![before.node.id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
            for keyword in keywords
                .into_iter()
                .filter(|keyword| !keyword.trim().is_empty())
            {
                conn.execute(
                    "INSERT OR IGNORE INTO memory_keywords (id, space_id, node_id, keyword)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        Uuid::new_v4().to_string(),
                        space_id.to_string(),
                        before.node.id.to_string(),
                        keyword
                    ],
                )
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?;
            }
        }

        for route in self.fetch_memory_routes_for_node(before.node.id).await? {
            self.rebuild_search_doc_for_route(route.id).await?;
        }
        let after =
            self.fetch_detail(before.node.id)
                .await?
                .ok_or_else(|| DatabaseError::NotFound {
                    entity: "memory_node".to_string(),
                    id: before.node.id.to_string(),
                })?;
        if let Some(changeset_id) = input.changeset_id {
            self.write_changeset_row(
                changeset_id,
                Some(before.node.id),
                route_id_for_changeset,
                "update",
                &serde_json::to_value(&before).unwrap_or(serde_json::Value::Null),
                &serde_json::to_value(&after).unwrap_or(serde_json::Value::Null),
            )
            .await?;
        }
        Ok(after)
    }

    async fn create_memory_alias(
        &self,
        input: &CreateMemoryAliasInput,
    ) -> Result<MemoryRoute, DatabaseError> {
        let Some(detail) = self
            .get_memory_node(input.space_id, &input.target_route_or_node)
            .await?
        else {
            return Err(DatabaseError::NotFound {
                entity: "memory_node".to_string(),
                id: input.target_route_or_node.clone(),
            });
        };
        if let Some(_existing) = self
            .fetch_memory_route_by_path(input.space_id, &input.domain, &input.path)
            .await?
        {
            return Err(DatabaseError::Constraint(format!(
                "Path '{}://{}' already exists",
                input.domain, input.path
            )));
        }

        let parent_node_id = if let Some((parent_path, _)) = input.path.rsplit_once('/') {
            let parent_route = self
                .fetch_memory_route_by_path(input.space_id, &input.domain, parent_path)
                .await?
                .ok_or_else(|| DatabaseError::NotFound {
                    entity: "memory_route".to_string(),
                    id: format!("{}://{}", input.domain, parent_path),
                })?;
            Some(parent_route.node_id)
        } else {
            None
        };

        if let Some(parent_node_id) = parent_node_id
            && self
                .would_create_cycle(parent_node_id, detail.node.id)
                .await?
        {
            return Err(DatabaseError::Constraint(format!(
                "Cannot create alias '{}://{}': target node is an ancestor of the destination parent",
                input.domain, input.path
            )));
        }

        let edge = if let Some(edge) = self
            .find_contains_edge(input.space_id, parent_node_id, detail.node.id)
            .await?
        {
            edge
        } else {
            let now = Utc::now();
            let edge = MemoryEdge {
                id: Uuid::new_v4(),
                space_id: input.space_id,
                parent_node_id,
                child_node_id: detail.node.id,
                relation_kind: MemoryRelationKind::Contains,
                visibility: input.visibility,
                priority: input.priority,
                trigger_text: input.trigger_text.clone(),
                created_at: now,
                updated_at: now,
            };
            self.upsert_edge_snapshot(&edge).await?;
            edge
        };

        let route = self
            .create_route_snapshot(
                input.space_id,
                edge.id,
                detail.node.id,
                &input.domain,
                &input.path,
                false,
            )
            .await?;
        self.rebuild_search_doc_for_route(route.id).await?;

        let mut created_routes = vec![route.clone()];
        self.cascade_create_alias_routes(
            input.space_id,
            detail.node.id,
            &input.domain,
            &input.path,
            &mut HashSet::new(),
            &mut created_routes,
        )
        .await?;
        if let Some(changeset_id) = input.changeset_id {
            for route in &created_routes {
                self.write_changeset_row(
                    changeset_id,
                    Some(route.node_id),
                    Some(route.id),
                    "alias",
                    &serde_json::Value::Null,
                    &serde_json::to_value(route).unwrap_or(serde_json::Value::Null),
                )
                .await?;
            }
        }
        Ok(route)
    }

    async fn delete_memory_node(
        &self,
        space_id: Uuid,
        route_or_node: &str,
        changeset_id: Option<Uuid>,
    ) -> Result<(), DatabaseError> {
        let Some(before) = self.get_memory_node(space_id, route_or_node).await? else {
            return Ok(());
        };
        let deleted_route = self.fetch_memory_route(space_id, route_or_node).await?;
        if let Some(route) = deleted_route.clone() {
            let routes_to_delete = self
                .fetch_memory_routes_by_prefix(space_id, &route.domain, &route.path)
                .await?;
            let mut affected_nodes = HashSet::new();
            for route in &routes_to_delete {
                if let Some(detail) = self.get_memory_node(space_id, &route.uri()).await? {
                    if let Some(changeset_id) = changeset_id {
                        self.write_changeset_row(
                            changeset_id,
                            Some(detail.node.id),
                            Some(route.id),
                            "delete",
                            &serde_json::to_value(&detail).unwrap_or(serde_json::Value::Null),
                            &serde_json::to_value(route).unwrap_or(serde_json::Value::Null),
                        )
                        .await?;
                    }
                    affected_nodes.insert(detail.node.id);
                }
            }

            for route in routes_to_delete {
                self.remove_route_by_id(route.id).await?;
                affected_nodes.insert(route.node_id);
            }

            let conn = self.connect().await?;
            for node_id in affected_nodes {
                if self.fetch_memory_routes_for_node(node_id).await?.is_empty() {
                    conn.execute(
                        "UPDATE memory_versions
                         SET status = 'orphaned'
                         WHERE node_id = ?1 AND status = 'active'",
                        params![node_id.to_string()],
                    )
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
                }
            }
        } else {
            let conn = self.connect().await?;
            conn.execute(
                "DELETE FROM memory_search_docs WHERE node_id = ?1",
                params![before.node.id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
            conn.execute(
                "DELETE FROM memory_routes WHERE node_id = ?1",
                params![before.node.id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
            conn.execute(
                "DELETE FROM memory_edges WHERE child_node_id = ?1",
                params![before.node.id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
            conn.execute(
                "UPDATE memory_versions SET status = 'orphaned' WHERE node_id = ?1 AND status = 'active'",
                params![before.node.id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
            if let Some(changeset_id) = changeset_id {
                self.write_changeset_row(
                    changeset_id,
                    Some(before.node.id),
                    None,
                    "delete",
                    &serde_json::to_value(&before).unwrap_or(serde_json::Value::Null),
                    &serde_json::Value::Null,
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn get_memory_node(
        &self,
        space_id: Uuid,
        route_or_node: &str,
    ) -> Result<Option<MemoryNodeDetail>, DatabaseError> {
        let Some(node_id) = self.resolve_node_id(space_id, route_or_node).await? else {
            return Ok(None);
        };
        let mut detail = match self.fetch_detail(node_id).await? {
            Some(detail) => detail,
            None => return Ok(None),
        };
        if route_or_node.contains("://") {
            let selected_route = detail
                .routes
                .iter()
                .find(|route| route.uri() == route_or_node)
                .cloned()
                .or_else(|| detail.primary_route.clone());
            detail.selected_uri = selected_route.as_ref().map(MemoryRoute::uri);
            detail.selected_route = selected_route;
        }
        Ok(Some(detail))
    }

    async fn search_memory_graph(
        &self,
        space_id: Uuid,
        query: &str,
        limit: usize,
        domains: &[String],
    ) -> Result<Vec<MemorySearchHit>, DatabaseError> {
        let conn = self.connect().await?;
        let match_query = sqlite_match_query(query);
        let domain_filter = if domains.is_empty() {
            None
        } else {
            Some(domains.join(","))
        };
        let sql = if domain_filter.is_some() {
            "SELECT d.node_id, d.route_id, d.version_id, d.uri, d.title, d.kind, d.content, coalesce(e.priority, 100), d.trigger_text, d.keywords, d.updated_at
             FROM memory_search_docs_fts fts
             JOIN memory_search_docs d ON d.rowid = fts.rowid
             JOIN memory_routes r ON r.id = d.route_id
             LEFT JOIN memory_edges e ON e.id = r.edge_id
             WHERE d.space_id = ?1 AND memory_search_docs_fts MATCH ?2 AND instr(?3, r.domain) > 0
             ORDER BY bm25(memory_search_docs_fts), e.priority ASC, d.updated_at DESC
             LIMIT ?4"
        } else {
            "SELECT d.node_id, d.route_id, d.version_id, d.uri, d.title, d.kind, d.content, coalesce(e.priority, 100), d.trigger_text, d.keywords, d.updated_at
             FROM memory_search_docs_fts fts
             JOIN memory_search_docs d ON d.rowid = fts.rowid
             JOIN memory_routes r ON r.id = d.route_id
             LEFT JOIN memory_edges e ON e.id = r.edge_id
             WHERE d.space_id = ?1 AND memory_search_docs_fts MATCH ?2
             ORDER BY bm25(memory_search_docs_fts), e.priority ASC, d.updated_at DESC
             LIMIT ?3"
        };

        let mut rows: libsql::Rows = if let Some(filter) = domain_filter {
            conn.query(
                sql,
                params![space_id.to_string(), match_query, filter, limit as i64],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        } else {
            conn.query(
                sql,
                params![space_id.to_string(), match_query, limit as i64],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        };

        let mut hits = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let content = get_text(&row, 6);
            let keywords = split_keywords(&get_text(&row, 9));
            hits.push(MemorySearchHit {
                node_id: get_text(&row, 0).parse().unwrap_or_default(),
                route_id: get_text(&row, 1).parse().unwrap_or_default(),
                version_id: get_text(&row, 2).parse().unwrap_or_default(),
                uri: get_text(&row, 3),
                title: get_text(&row, 4),
                kind: MemoryNodeKind::from_str(&get_text(&row, 5)),
                content_snippet: snippet(&content, query),
                priority: row.get::<i64>(7).unwrap_or(100) as i32,
                trigger_text: get_opt_text(&row, 8),
                score: 1.0 / (hits.len() as f32 + 1.0),
                fts_rank: Some(hits.len() as u32 + 1),
                vector_rank: None,
                matched_keywords: collect_matched_keywords(query, &keywords),
                updated_at: get_ts(&row, 10),
            });
        }
        Ok(hits)
    }

    async fn vector_search_memory_graph(
        &self,
        space_id: Uuid,
        embedding: &[f32],
        limit: usize,
        domains: &[String],
    ) -> Result<Vec<MemorySearchHit>, DatabaseError> {
        let conn = self.connect().await?;
        let vector_json = format!(
            "[{}]",
            embedding
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );

        let mut rows = conn
            .query(
                "SELECT d.node_id, d.route_id, d.version_id, d.uri, d.title, d.kind, d.content,
                        coalesce(e.priority, 100), d.trigger_text, d.keywords, d.updated_at
                 FROM vector_top_k('idx_memory_search_docs_embedding', vector(?1), ?2) AS top_k
                 JOIN memory_search_docs d ON d.rowid = top_k.id
                 JOIN memory_routes r ON r.id = d.route_id
                 LEFT JOIN memory_edges e ON e.id = r.edge_id
                 WHERE d.space_id = ?3
                   AND (?4 = '' OR instr(?4, r.domain) > 0)",
                params![
                    vector_json,
                    limit as i64,
                    space_id.to_string(),
                    domains.join(",")
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("memory vector query failed: {e}")))?;

        let mut hits = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let content = get_text(&row, 6);
            let title = get_text(&row, 4);
            hits.push(MemorySearchHit {
                node_id: get_text(&row, 0).parse().unwrap_or_default(),
                route_id: get_text(&row, 1).parse().unwrap_or_default(),
                version_id: get_text(&row, 2).parse().unwrap_or_default(),
                uri: get_text(&row, 3),
                title: title.clone(),
                kind: MemoryNodeKind::from_str(&get_text(&row, 5)),
                content_snippet: snippet(&content, &title),
                priority: row.get::<i64>(7).unwrap_or(100) as i32,
                trigger_text: get_opt_text(&row, 8),
                score: 1.0 / (hits.len() as f32 + 1.0),
                fts_rank: None,
                vector_rank: Some(hits.len() as u32 + 1),
                matched_keywords: Vec::new(),
                updated_at: get_ts(&row, 10),
            });
        }
        Ok(hits)
    }

    async fn list_memory_sidebar(
        &self,
        space_id: Uuid,
        limit_per_section: usize,
    ) -> Result<Vec<MemorySidebarSection>, DatabaseError> {
        let boot = self.list_memory_boot_nodes(space_id, None).await?;
        let recent = self
            .list_memory_timeline(space_id, limit_per_section)
            .await?;
        let reviews = self.list_memory_reviews(space_id).await?;
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT d.node_id, d.route_id, d.uri, d.title, d.kind, d.updated_at
                 FROM memory_search_docs d
                 WHERE d.space_id = ?1
                 ORDER BY d.kind, d.updated_at DESC
                 LIMIT ?2",
                params![space_id.to_string(), (limit_per_section * 4) as i64],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut domain_items = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let uri = get_text(&row, 2);
            domain_items.push(MemorySidebarItem {
                node_id: get_text(&row, 0).parse().unwrap_or_default(),
                route_id: Some(get_text(&row, 1).parse().unwrap_or_default()),
                uri: Some(uri.clone()),
                title: get_text(&row, 3),
                subtitle: Some(uri),
                kind: MemoryNodeKind::from_str(&get_text(&row, 4)),
                updated_at: get_ts(&row, 5),
            });
        }

        Ok(vec![
            MemorySidebarSection {
                key: "boot".to_string(),
                title: "Core Memories".to_string(),
                items: boot
                    .into_iter()
                    .take(limit_per_section)
                    .map(|detail| MemorySidebarItem {
                        node_id: detail.node.id,
                        route_id: detail.primary_route.as_ref().map(|route| route.id),
                        uri: detail.primary_route.as_ref().map(|route| route.uri()),
                        title: detail.node.title,
                        subtitle: detail.primary_route.as_ref().map(|route| route.uri()),
                        kind: detail.node.kind,
                        updated_at: detail.node.updated_at,
                    })
                    .collect(),
            },
            MemorySidebarSection {
                key: "recent".to_string(),
                title: "Recent Timeline".to_string(),
                items: recent
                    .into_iter()
                    .map(|item| MemorySidebarItem {
                        node_id: item.node_id,
                        route_id: item.route_id,
                        uri: item.uri,
                        title: item.title,
                        subtitle: Some(item.content_snippet),
                        kind: MemoryNodeKind::Episode,
                        updated_at: item.updated_at,
                    })
                    .collect(),
            },
            MemorySidebarSection {
                key: "graph".to_string(),
                title: "Memory Graph".to_string(),
                items: domain_items.into_iter().take(limit_per_section).collect(),
            },
            MemorySidebarSection {
                key: "reviews".to_string(),
                title: "Review Queue".to_string(),
                items: reviews
                    .into_iter()
                    .take(limit_per_section)
                    .map(|changeset| MemorySidebarItem {
                        node_id: Uuid::nil(),
                        route_id: None,
                        uri: Some(format!("review://{}", changeset.id)),
                        title: changeset
                            .summary
                            .unwrap_or_else(|| "Pending memory review".to_string()),
                        subtitle: Some(changeset.origin),
                        kind: MemoryNodeKind::Procedure,
                        updated_at: changeset.updated_at,
                    })
                    .collect(),
            },
        ])
    }

    async fn list_memory_timeline(
        &self,
        space_id: Uuid,
        limit: usize,
    ) -> Result<Vec<MemoryTimelineEntry>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT d.node_id, d.route_id, d.uri, d.title, d.content, d.updated_at
                 FROM memory_search_docs d
                 WHERE d.space_id = ?1 AND d.kind = 'episode'
                 ORDER BY d.updated_at DESC
                 LIMIT ?2",
                params![space_id.to_string(), limit as i64],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut entries = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let content = get_text(&row, 4);
            entries.push(MemoryTimelineEntry {
                node_id: get_text(&row, 0).parse().unwrap_or_default(),
                route_id: Some(get_text(&row, 1).parse().unwrap_or_default()),
                uri: Some(get_text(&row, 2)),
                title: get_text(&row, 3),
                content_snippet: snippet(&content, content.lines().next().unwrap_or("episode")),
                updated_at: get_ts(&row, 5),
            });
        }
        Ok(entries)
    }

    async fn list_memory_reviews(
        &self,
        space_id: Uuid,
    ) -> Result<Vec<MemoryChangeSet>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, space_id, origin, summary, status, created_at, updated_at
                 FROM memory_changesets
                 WHERE space_id = ?1 AND status = 'pending'
                 ORDER BY created_at DESC",
                params![space_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut changesets = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            changesets.push(row_to_memory_changeset(&row));
        }
        Ok(changesets)
    }

    async fn get_memory_versions(
        &self,
        node_id: Uuid,
    ) -> Result<Vec<MemoryVersion>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, node_id, supersedes_version_id, status, content, metadata, created_at
                 FROM memory_versions
                 WHERE node_id = ?1
                 ORDER BY created_at DESC",
                params![node_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut versions = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            versions.push(row_to_memory_version(&row));
        }
        Ok(versions)
    }

    async fn list_memory_boot_nodes(
        &self,
        space_id: Uuid,
        max_visibility: Option<MemoryVisibility>,
    ) -> Result<Vec<MemoryNodeDetail>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT DISTINCT r.node_id
                 FROM memory_boot_entries b
                 JOIN memory_routes r ON r.id = b.route_id
                 LEFT JOIN memory_edges e ON e.id = r.edge_id
                 WHERE b.space_id = ?1
                   AND (?2 IS NULL OR e.visibility IN ('session', 'shared'))
                 ORDER BY b.load_priority ASC, b.updated_at DESC",
                params![space_id.to_string(), max_visibility.map(|_| "session")],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut details = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let node_id: Uuid = get_text(&row, 0).parse().unwrap_or_default();
            if let Some(detail) = self.fetch_detail(node_id).await? {
                details.push(detail);
            }
        }
        Ok(details)
    }

    async fn upsert_memory_boot_route(
        &self,
        space_id: Uuid,
        route_or_node: &str,
        load_priority: i32,
    ) -> Result<MemoryRoute, DatabaseError> {
        let route = if let Some(route) = self.fetch_memory_route(space_id, route_or_node).await? {
            route
        } else if let Ok(node_id) = Uuid::parse_str(route_or_node) {
            self.fetch_memory_routes_for_node(node_id)
                .await?
                .into_iter()
                .find(|route| route.is_primary)
                .ok_or_else(|| DatabaseError::NotFound {
                    entity: "memory_route".to_string(),
                    id: route_or_node.to_string(),
                })?
        } else {
            return Err(DatabaseError::NotFound {
                entity: "memory_route".to_string(),
                id: route_or_node.to_string(),
            });
        };

        let conn = self.connect().await?;
        conn.execute(
            "INSERT INTO memory_boot_entries (route_id, space_id, load_priority)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(route_id) DO UPDATE SET
                load_priority = excluded.load_priority,
                updated_at = (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![route.id.to_string(), space_id.to_string(), load_priority],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("failed to upsert memory boot entry: {e}")))?;
        Ok(route)
    }

    async fn delete_memory_boot_route(
        &self,
        space_id: Uuid,
        route_or_node: &str,
    ) -> Result<(), DatabaseError> {
        let route = if let Some(route) = self.fetch_memory_route(space_id, route_or_node).await? {
            route
        } else if let Ok(node_id) = Uuid::parse_str(route_or_node) {
            self.fetch_memory_routes_for_node(node_id)
                .await?
                .into_iter()
                .find(|route| route.is_primary)
                .ok_or_else(|| DatabaseError::NotFound {
                    entity: "memory_route".to_string(),
                    id: route_or_node.to_string(),
                })?
        } else {
            return Err(DatabaseError::NotFound {
                entity: "memory_route".to_string(),
                id: route_or_node.to_string(),
            });
        };

        let conn = self.connect().await?;
        conn.execute(
            "DELETE FROM memory_boot_entries WHERE route_id = ?1",
            params![route.id.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("failed to delete memory boot entry: {e}")))?;
        Ok(())
    }

    async fn list_memory_index(
        &self,
        space_id: Uuid,
        domain: Option<&str>,
    ) -> Result<Vec<MemoryIndexEntry>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = if let Some(domain) = domain {
            conn.query(
                "SELECT d.uri, d.title, d.kind, coalesce(e.priority, 100), d.trigger_text, d.updated_at
                 FROM memory_search_docs d
                 JOIN memory_routes r ON r.id = d.route_id
                 LEFT JOIN memory_edges e ON e.id = r.edge_id
                 WHERE d.space_id = ?1 AND r.domain = ?2
                 ORDER BY r.domain ASC, r.path ASC",
                params![space_id.to_string(), domain],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        } else {
            conn.query(
                "SELECT d.uri, d.title, d.kind, coalesce(e.priority, 100), d.trigger_text, d.updated_at
                 FROM memory_search_docs d
                 JOIN memory_routes r ON r.id = d.route_id
                 LEFT JOIN memory_edges e ON e.id = r.edge_id
                 WHERE d.space_id = ?1
                 ORDER BY r.domain ASC, r.path ASC",
                params![space_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        };

        let mut entries = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            entries.push(MemoryIndexEntry {
                uri: get_text(&row, 0),
                title: get_text(&row, 1),
                kind: MemoryNodeKind::from_str(&get_text(&row, 2)),
                priority: row.get::<i64>(3).unwrap_or(100) as i32,
                disclosure: get_opt_text(&row, 4),
                updated_at: get_ts(&row, 5),
            });
        }
        Ok(entries)
    }

    async fn list_memory_recent(
        &self,
        space_id: Uuid,
        limit: usize,
        domain: Option<&str>,
    ) -> Result<Vec<MemoryIndexEntry>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = if let Some(domain) = domain {
            conn.query(
                "SELECT d.uri, d.title, d.kind, coalesce(e.priority, 100), d.trigger_text, d.updated_at
                 FROM memory_search_docs d
                 JOIN memory_routes r ON r.id = d.route_id
                 LEFT JOIN memory_edges e ON e.id = r.edge_id
                 WHERE d.space_id = ?1 AND r.domain = ?2
                 ORDER BY d.updated_at DESC
                 LIMIT ?3",
                params![space_id.to_string(), domain, limit as i64],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        } else {
            conn.query(
                "SELECT d.uri, d.title, d.kind, coalesce(e.priority, 100), d.trigger_text, d.updated_at
                 FROM memory_search_docs d
                 JOIN memory_routes r ON r.id = d.route_id
                 LEFT JOIN memory_edges e ON e.id = r.edge_id
                 WHERE d.space_id = ?1
                 ORDER BY d.updated_at DESC
                 LIMIT ?2",
                params![space_id.to_string(), limit as i64],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        };

        let mut entries = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            entries.push(MemoryIndexEntry {
                uri: get_text(&row, 0),
                title: get_text(&row, 1),
                kind: MemoryNodeKind::from_str(&get_text(&row, 2)),
                priority: row.get::<i64>(3).unwrap_or(100) as i32,
                disclosure: get_opt_text(&row, 4),
                updated_at: get_ts(&row, 5),
            });
        }
        Ok(entries)
    }

    async fn list_memory_glossary(
        &self,
        space_id: Uuid,
    ) -> Result<Vec<MemoryGlossaryEntry>, DatabaseError> {
        use std::collections::{HashMap, HashSet};

        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT k.keyword, r.domain, r.path
                 FROM memory_keywords k
                 JOIN memory_routes r ON r.node_id = k.node_id
                 WHERE k.space_id = ?1
                 ORDER BY k.keyword ASC, r.is_primary DESC, r.updated_at DESC",
                params![space_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut map: HashMap<String, (HashSet<String>, Vec<String>)> = HashMap::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let keyword = get_text(&row, 0);
            let domain = get_text(&row, 1);
            let path = get_text(&row, 2);
            let uri = format!("{domain}://{path}");
            let entry = map
                .entry(keyword)
                .or_insert_with(|| (HashSet::new(), Vec::new()));
            if entry.0.insert(uri.clone()) {
                entry.1.push(uri);
            }
        }

        let mut out = map
            .into_iter()
            .map(|(keyword, (_seen, uris))| MemoryGlossaryEntry { keyword, uris })
            .collect::<Vec<_>>();
        out.sort_by(|a, b| a.keyword.cmp(&b.keyword));
        Ok(out)
    }

    async fn list_memory_children(
        &self,
        space_id: Uuid,
        parent_node_id: Uuid,
        limit: usize,
    ) -> Result<Vec<MemoryChildEntry>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT n.id, n.title, n.kind, n.updated_at, r.domain, r.path, coalesce(e.priority, 100), e.trigger_text
                 FROM memory_edges e
                 JOIN memory_nodes n ON n.id = e.child_node_id
                 LEFT JOIN memory_routes r ON r.node_id = n.id AND r.is_primary = 1
                 WHERE e.space_id = ?1 AND e.parent_node_id = ?2 AND e.relation_kind = 'contains'
                 ORDER BY e.priority ASC, n.updated_at DESC
                 LIMIT ?3",
                params![space_id.to_string(), parent_node_id.to_string(), limit as i64],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let node_id = get_text(&row, 0);
            let domain = get_opt_text(&row, 4);
            let path = get_opt_text(&row, 5);
            let uri = if let (Some(domain), Some(path)) = (domain, path) {
                if !domain.trim().is_empty() && !path.trim().is_empty() {
                    format!("{domain}://{path}")
                } else {
                    format!("node://{node_id}")
                }
            } else {
                format!("node://{node_id}")
            };

            out.push(MemoryChildEntry {
                uri,
                title: get_text(&row, 1),
                kind: MemoryNodeKind::from_str(&get_text(&row, 2)),
                updated_at: get_ts(&row, 3),
                priority: row.get::<i64>(6).unwrap_or(100) as i32,
                disclosure: get_opt_text(&row, 7),
            });
        }
        Ok(out)
    }

    async fn get_memory_search_doc(
        &self,
        space_id: Uuid,
        route_id: Uuid,
    ) -> Result<Option<MemorySearchDoc>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT route_id, space_id, node_id, version_id, uri, title, kind, content, trigger_text, keywords, search_terms, updated_at
                 FROM memory_search_docs
                 WHERE space_id = ?1 AND route_id = ?2
                 LIMIT 1",
                params![space_id.to_string(), route_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
            .map(|row| row_to_memory_search_doc(&row)))
    }

    async fn list_memory_search_docs_without_embeddings(
        &self,
        space_id: Uuid,
        limit: usize,
    ) -> Result<Vec<MemorySearchDoc>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT route_id, space_id, node_id, version_id, uri, title, kind, content, trigger_text, keywords, search_terms, updated_at
                 FROM memory_search_docs
                 WHERE space_id = ?1 AND embedding IS NULL
                 ORDER BY updated_at DESC
                 LIMIT ?2",
                params![space_id.to_string(), limit as i64],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut docs = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            docs.push(row_to_memory_search_doc(&row));
        }
        Ok(docs)
    }

    async fn update_memory_search_doc_embedding(
        &self,
        route_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let mut bytes = Vec::with_capacity(embedding.len() * 4);
        for value in embedding {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        conn.execute(
            "UPDATE memory_search_docs SET embedding = ?2 WHERE route_id = ?1",
            params![route_id.to_string(), bytes],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("failed to update memory embedding: {e}")))?;
        Ok(())
    }
}

impl LibSqlBackend {
    /// Ensure the vector index on `memory_search_docs.embedding` matches the
    /// configured embedding dimension for native memory recall.
    pub async fn ensure_memory_vector_index(&self, dimension: usize) -> Result<(), DatabaseError> {
        if dimension == 0 || dimension > 65536 {
            return Err(DatabaseError::Migration(format!(
                "ensure_memory_vector_index: dimension {dimension} out of valid range (1..=65536)"
            )));
        }

        let conn = self.connect().await?;
        let current_dim = {
            let mut rows = conn
                .query("SELECT name FROM _migrations WHERE version = -2", ())
                .await
                .map_err(|e| {
                    DatabaseError::Migration(format!(
                        "Failed to check memory vector index metadata: {e}"
                    ))
                })?;

            rows.next().await.ok().flatten().and_then(|row| {
                row.get::<String>(0)
                    .ok()
                    .and_then(|value| value.parse::<usize>().ok())
            })
        };

        if current_dim == Some(dimension) {
            return Ok(());
        }

        let expected_bytes = dimension * 4;
        let tx = conn.transaction().await.map_err(|e| {
            DatabaseError::Migration(format!(
                "ensure_memory_vector_index: failed to start transaction: {e}"
            ))
        })?;

        tx.execute_batch(
            "DROP TRIGGER IF EXISTS memory_search_docs_fts_insert;
             DROP TRIGGER IF EXISTS memory_search_docs_fts_delete;
             DROP TRIGGER IF EXISTS memory_search_docs_fts_update;
             DROP TABLE IF EXISTS memory_search_docs_fts;
             DROP INDEX IF EXISTS idx_memory_search_docs_embedding;",
        )
        .await
        .map_err(|e| {
            DatabaseError::Migration(format!(
                "Failed to drop native memory search indexes/triggers: {e}"
            ))
        })?;

        tx.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS memory_search_docs_new (
                route_id TEXT PRIMARY KEY REFERENCES memory_routes(id) ON DELETE CASCADE,
                space_id TEXT NOT NULL REFERENCES memory_spaces(id) ON DELETE CASCADE,
                node_id TEXT NOT NULL REFERENCES memory_nodes(id) ON DELETE CASCADE,
                version_id TEXT NOT NULL REFERENCES memory_versions(id) ON DELETE CASCADE,
                uri TEXT NOT NULL,
                title TEXT NOT NULL,
                kind TEXT NOT NULL,
                content TEXT NOT NULL,
                trigger_text TEXT,
                keywords TEXT NOT NULL DEFAULT '',
                search_terms TEXT NOT NULL DEFAULT '',
                embedding F32_BLOB({dimension}),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            );

            INSERT OR REPLACE INTO memory_search_docs_new
                (route_id, space_id, node_id, version_id, uri, title, kind, content, trigger_text, keywords, search_terms, embedding, updated_at)
            SELECT
                route_id, space_id, node_id, version_id, uri, title, kind, content, trigger_text, keywords, search_terms,
                CASE WHEN length(embedding) = {expected_bytes} THEN embedding ELSE NULL END,
                updated_at
            FROM memory_search_docs;

            DROP TABLE memory_search_docs;
            ALTER TABLE memory_search_docs_new RENAME TO memory_search_docs;

            CREATE INDEX IF NOT EXISTS idx_memory_search_docs_space_updated
                ON memory_search_docs(space_id, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_memory_search_docs_embedding
                ON memory_search_docs(libsql_vector_idx(embedding));

            CREATE VIRTUAL TABLE IF NOT EXISTS memory_search_docs_fts USING fts5(
                title,
                content,
                trigger_text,
                keywords,
                uri,
                search_terms,
                content='memory_search_docs',
                content_rowid='rowid'
            );

            INSERT INTO memory_search_docs_fts(rowid, title, content, trigger_text, keywords, uri, search_terms)
            SELECT rowid, title, content, coalesce(trigger_text, ''), keywords, uri, search_terms
            FROM memory_search_docs;

            CREATE TRIGGER IF NOT EXISTS memory_search_docs_fts_insert AFTER INSERT ON memory_search_docs BEGIN
                INSERT INTO memory_search_docs_fts(rowid, title, content, trigger_text, keywords, uri, search_terms)
                VALUES (new.rowid, new.title, new.content, coalesce(new.trigger_text, ''), new.keywords, new.uri, new.search_terms);
            END;

            CREATE TRIGGER IF NOT EXISTS memory_search_docs_fts_delete AFTER DELETE ON memory_search_docs BEGIN
                INSERT INTO memory_search_docs_fts(memory_search_docs_fts, rowid, title, content, trigger_text, keywords, uri, search_terms)
                VALUES ('delete', old.rowid, old.title, old.content, coalesce(old.trigger_text, ''), old.keywords, old.uri, old.search_terms);
            END;

            CREATE TRIGGER IF NOT EXISTS memory_search_docs_fts_update AFTER UPDATE ON memory_search_docs BEGIN
                INSERT INTO memory_search_docs_fts(memory_search_docs_fts, rowid, title, content, trigger_text, keywords, uri, search_terms)
                VALUES ('delete', old.rowid, old.title, old.content, coalesce(old.trigger_text, ''), old.keywords, old.uri, old.search_terms);
                INSERT INTO memory_search_docs_fts(rowid, title, content, trigger_text, keywords, uri, search_terms)
                VALUES (new.rowid, new.title, new.content, coalesce(new.trigger_text, ''), new.keywords, new.uri, new.search_terms);
            END;"
        ))
        .await
        .map_err(|e| {
            DatabaseError::Migration(format!("Failed to rebuild memory search docs table: {e}"))
        })?;

        tx.execute(
            "INSERT INTO _migrations (version, name) VALUES (-2, ?1)
             ON CONFLICT(version) DO UPDATE SET name = excluded.name, applied_at = (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![dimension.to_string()],
        )
        .await
        .map_err(|e| {
            DatabaseError::Migration(format!(
                "Failed to record memory vector index metadata: {e}"
            ))
        })?;

        tx.commit().await.map_err(|e| {
            DatabaseError::Migration(format!(
                "ensure_memory_vector_index: failed to commit transaction: {e}"
            ))
        })?;

        Ok(())
    }
}
