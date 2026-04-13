//! SQLite-dialect migrations for the libSQL/Turso backend.
//!
//! Consolidates earlier schema history into a single SQLite-compatible
//! schema. Run once on database creation; idempotent via `IF NOT EXISTS`.
//!
//! Incremental migrations (V9+) are tracked in the `_migrations` table and run
//! exactly once per database, in version order.

/// Consolidated schema for libSQL.
///
/// Translates legacy schema types and features:
/// - `UUID` -> `TEXT` (store as hex string)
/// - `TIMESTAMPTZ` -> `TEXT` (ISO-8601)
/// - `JSONB` -> `TEXT` (JSON encoded)
/// - `BYTEA` -> `BLOB`
/// - `NUMERIC` -> `TEXT` (preserve precision for rust_decimal)
/// - `TEXT[]` -> `TEXT` (JSON array)
/// - `VECTOR` -> `BLOB` (raw little-endian F32 bytes, any dimension)
/// - `TSVECTOR` -> FTS5 virtual table
/// - `BIGSERIAL` -> `INTEGER PRIMARY KEY AUTOINCREMENT`
/// - PL/pgSQL functions -> SQLite triggers
pub const SCHEMA: &str = r#"

-- ==================== Migration tracking ====================

CREATE TABLE IF NOT EXISTS _migrations (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- ==================== Conversations ====================

CREATE TABLE IF NOT EXISTS conversations (
    id TEXT PRIMARY KEY,
    channel TEXT NOT NULL,
    user_id TEXT NOT NULL,
    thread_id TEXT,
    started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_activity TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    metadata TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_conversations_channel ON conversations(channel);
CREATE INDEX IF NOT EXISTS idx_conversations_user ON conversations(user_id);
CREATE INDEX IF NOT EXISTS idx_conversations_last_activity ON conversations(last_activity);

-- Partial unique indexes to prevent duplicate singleton conversations.
CREATE UNIQUE INDEX IF NOT EXISTS uq_conv_routine
ON conversations (user_id, json_extract(metadata, '$.routine_id'))
WHERE json_extract(metadata, '$.routine_id') IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS uq_conv_heartbeat
ON conversations (user_id)
WHERE json_extract(metadata, '$.thread_type') = 'heartbeat';

CREATE TABLE IF NOT EXISTS conversation_messages (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_conversation_messages_conversation
    ON conversation_messages(conversation_id);

CREATE TABLE IF NOT EXISTS conversation_recall_docs (
    doc_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    channel TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    turn_index INTEGER NOT NULL,
    user_message_id TEXT NOT NULL,
    assistant_message_id TEXT NOT NULL,
    turn_timestamp TEXT NOT NULL,
    user_text TEXT NOT NULL,
    assistant_text TEXT NOT NULL,
    search_text TEXT NOT NULL,
    preview_text TEXT NOT NULL,
    embedding BLOB,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_conversation_recall_docs_user_updated
    ON conversation_recall_docs(user_id, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_conversation_recall_docs_conversation_turn
    ON conversation_recall_docs(conversation_id, turn_index);
CREATE INDEX IF NOT EXISTS idx_conversation_recall_docs_thread
    ON conversation_recall_docs(user_id, thread_id);

CREATE VIRTUAL TABLE IF NOT EXISTS conversation_recall_docs_fts USING fts5(
    user_text,
    assistant_text,
    search_text,
    preview_text,
    content='conversation_recall_docs',
    content_rowid='rowid'
);

CREATE TRIGGER IF NOT EXISTS conversation_recall_docs_fts_insert AFTER INSERT ON conversation_recall_docs BEGIN
    INSERT INTO conversation_recall_docs_fts(rowid, user_text, assistant_text, search_text, preview_text)
    VALUES (new.rowid, new.user_text, new.assistant_text, new.search_text, new.preview_text);
END;

CREATE TRIGGER IF NOT EXISTS conversation_recall_docs_fts_delete AFTER DELETE ON conversation_recall_docs BEGIN
    INSERT INTO conversation_recall_docs_fts(conversation_recall_docs_fts, rowid, user_text, assistant_text, search_text, preview_text)
    VALUES ('delete', old.rowid, old.user_text, old.assistant_text, old.search_text, old.preview_text);
END;

CREATE TRIGGER IF NOT EXISTS conversation_recall_docs_fts_update AFTER UPDATE ON conversation_recall_docs BEGIN
    INSERT INTO conversation_recall_docs_fts(conversation_recall_docs_fts, rowid, user_text, assistant_text, search_text, preview_text)
    VALUES ('delete', old.rowid, old.user_text, old.assistant_text, old.search_text, old.preview_text);
    INSERT INTO conversation_recall_docs_fts(rowid, user_text, assistant_text, search_text, preview_text)
    VALUES (new.rowid, new.user_text, new.assistant_text, new.search_text, new.preview_text);
END;

-- ==================== Agent Jobs ====================

CREATE TABLE IF NOT EXISTS agent_jobs (
    id TEXT PRIMARY KEY,
    marketplace_job_id TEXT,
    conversation_id TEXT REFERENCES conversations(id),
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    category TEXT,
    status TEXT NOT NULL,
    source TEXT NOT NULL,
    user_id TEXT NOT NULL DEFAULT 'default',
    project_dir TEXT,
    job_mode TEXT NOT NULL DEFAULT 'worker',
    budget_amount TEXT,
    budget_token TEXT,
    bid_amount TEXT,
    estimated_cost TEXT,
    estimated_time_secs INTEGER,
    estimated_value TEXT,
    actual_cost TEXT,
    actual_time_secs INTEGER,
    success INTEGER,
    failure_reason TEXT,
    stuck_since TEXT,
    repair_attempts INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    started_at TEXT,
    completed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_agent_jobs_status ON agent_jobs(status);
CREATE INDEX IF NOT EXISTS idx_agent_jobs_marketplace ON agent_jobs(marketplace_job_id);
CREATE INDEX IF NOT EXISTS idx_agent_jobs_conversation ON agent_jobs(conversation_id);
CREATE INDEX IF NOT EXISTS idx_agent_jobs_source ON agent_jobs(source);
CREATE INDEX IF NOT EXISTS idx_agent_jobs_user ON agent_jobs(user_id);
CREATE INDEX IF NOT EXISTS idx_agent_jobs_created ON agent_jobs(created_at DESC);

CREATE TABLE IF NOT EXISTS job_actions (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES agent_jobs(id) ON DELETE CASCADE,
    sequence_num INTEGER NOT NULL,
    tool_name TEXT NOT NULL,
    input TEXT NOT NULL,
    output_raw TEXT,
    output_sanitized TEXT,
    sanitization_warnings TEXT,
    cost TEXT,
    duration_ms INTEGER,
    success INTEGER NOT NULL,
    error_message TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE(job_id, sequence_num)
);

CREATE INDEX IF NOT EXISTS idx_job_actions_job_id ON job_actions(job_id);
CREATE INDEX IF NOT EXISTS idx_job_actions_tool ON job_actions(tool_name);

-- ==================== Dynamic Tools ====================

CREATE TABLE IF NOT EXISTS dynamic_tools (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL,
    parameters_schema TEXT NOT NULL,
    code TEXT NOT NULL,
    sandbox_config TEXT NOT NULL,
    created_by_job_id TEXT REFERENCES agent_jobs(id),
    success_count INTEGER NOT NULL DEFAULT 0,
    failure_count INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_dynamic_tools_status ON dynamic_tools(status);
CREATE INDEX IF NOT EXISTS idx_dynamic_tools_name ON dynamic_tools(name);

-- ==================== LLM Calls ====================

CREATE TABLE IF NOT EXISTS llm_calls (
    id TEXT PRIMARY KEY,
    job_id TEXT REFERENCES agent_jobs(id) ON DELETE CASCADE,
    conversation_id TEXT REFERENCES conversations(id),
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cost TEXT NOT NULL,
    purpose TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_llm_calls_job ON llm_calls(job_id);
CREATE INDEX IF NOT EXISTS idx_llm_calls_conversation ON llm_calls(conversation_id);
CREATE INDEX IF NOT EXISTS idx_llm_calls_provider ON llm_calls(provider);

-- ==================== Estimation ====================

CREATE TABLE IF NOT EXISTS estimation_snapshots (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES agent_jobs(id) ON DELETE CASCADE,
    category TEXT NOT NULL,
    tool_names TEXT NOT NULL DEFAULT '[]',
    estimated_cost TEXT NOT NULL,
    actual_cost TEXT,
    estimated_time_secs INTEGER NOT NULL,
    actual_time_secs INTEGER,
    estimated_value TEXT NOT NULL,
    actual_value TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_estimation_category ON estimation_snapshots(category);
CREATE INDEX IF NOT EXISTS idx_estimation_job ON estimation_snapshots(job_id);

-- ==================== Self Repair ====================

CREATE TABLE IF NOT EXISTS repair_attempts (
    id TEXT PRIMARY KEY,
    target_type TEXT NOT NULL,
    target_id TEXT NOT NULL,
    diagnosis TEXT NOT NULL,
    action_taken TEXT NOT NULL,
    success INTEGER NOT NULL,
    error_message TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_repair_attempts_target ON repair_attempts(target_type, target_id);
CREATE INDEX IF NOT EXISTS idx_repair_attempts_created ON repair_attempts(created_at);

-- ==================== Workspace: Memory Documents ====================

CREATE TABLE IF NOT EXISTS memory_documents (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    agent_id TEXT,
    path TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    metadata TEXT NOT NULL DEFAULT '{}',
    UNIQUE (user_id, agent_id, path)
);

CREATE INDEX IF NOT EXISTS idx_memory_documents_user ON memory_documents(user_id);
CREATE INDEX IF NOT EXISTS idx_memory_documents_path ON memory_documents(user_id, path);
CREATE INDEX IF NOT EXISTS idx_memory_documents_updated ON memory_documents(updated_at DESC);

-- Trigger to auto-update updated_at on memory_documents
CREATE TRIGGER IF NOT EXISTS update_memory_documents_updated_at
    AFTER UPDATE ON memory_documents
    FOR EACH ROW
    WHEN NEW.updated_at = OLD.updated_at
    BEGIN
        UPDATE memory_documents SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = NEW.id;
    END;

-- ==================== Workspace: Memory Chunks ====================

CREATE TABLE IF NOT EXISTS memory_chunks (
    _rowid INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    document_id TEXT NOT NULL REFERENCES memory_documents(id) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL,
    content TEXT NOT NULL,
    embedding BLOB,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (document_id, chunk_index)
);

CREATE INDEX IF NOT EXISTS idx_memory_chunks_document ON memory_chunks(document_id);

-- No vector index in base schema: BLOB column accepts any embedding dimension.
-- Vector index is created dynamically by ensure_vector_index() during
-- run_migrations() when embeddings are configured (EMBEDDING_ENABLED=true).

-- FTS5 virtual table for full-text search
CREATE VIRTUAL TABLE IF NOT EXISTS memory_chunks_fts USING fts5(
    content,
    content='memory_chunks',
    content_rowid='_rowid'
);

-- Triggers to keep FTS5 in sync with memory_chunks
CREATE TRIGGER IF NOT EXISTS memory_chunks_fts_insert AFTER INSERT ON memory_chunks BEGIN
    INSERT INTO memory_chunks_fts(rowid, content) VALUES (new._rowid, new.content);
END;

CREATE TRIGGER IF NOT EXISTS memory_chunks_fts_delete AFTER DELETE ON memory_chunks BEGIN
    INSERT INTO memory_chunks_fts(memory_chunks_fts, rowid, content)
        VALUES ('delete', old._rowid, old.content);
END;

CREATE TRIGGER IF NOT EXISTS memory_chunks_fts_update AFTER UPDATE ON memory_chunks BEGIN
    INSERT INTO memory_chunks_fts(memory_chunks_fts, rowid, content)
        VALUES ('delete', old._rowid, old.content);
    INSERT INTO memory_chunks_fts(rowid, content) VALUES (new._rowid, new.content);
END;

-- ==================== Workspace: Heartbeat State ====================

CREATE TABLE IF NOT EXISTS heartbeat_state (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    agent_id TEXT,
    last_run TEXT,
    next_run TEXT,
    interval_seconds INTEGER NOT NULL DEFAULT 1800,
    enabled INTEGER NOT NULL DEFAULT 1,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    last_checks TEXT NOT NULL DEFAULT '{}',
    UNIQUE (user_id, agent_id)
);

CREATE INDEX IF NOT EXISTS idx_heartbeat_user ON heartbeat_state(user_id);

-- ==================== Secrets ====================

CREATE TABLE IF NOT EXISTS secrets (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    encrypted_value BLOB NOT NULL,
    key_salt BLOB NOT NULL,
    provider TEXT,
    expires_at TEXT,
    last_used_at TEXT,
    usage_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (user_id, name)
);

CREATE INDEX IF NOT EXISTS idx_secrets_user ON secrets(user_id);

-- ==================== WASM Tools ====================

CREATE TABLE IF NOT EXISTS wasm_tools (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    version TEXT NOT NULL DEFAULT '1.0.0',
    wit_version TEXT NOT NULL DEFAULT '0.1.0',
    description TEXT NOT NULL,
    wasm_binary BLOB NOT NULL,
    binary_hash BLOB NOT NULL,
    parameters_schema TEXT NOT NULL,
    source_url TEXT,
    trust_level TEXT NOT NULL DEFAULT 'user',
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (user_id, name, version)
);

CREATE INDEX IF NOT EXISTS idx_wasm_tools_user ON wasm_tools(user_id);
CREATE INDEX IF NOT EXISTS idx_wasm_tools_name ON wasm_tools(user_id, name);
CREATE INDEX IF NOT EXISTS idx_wasm_tools_status ON wasm_tools(status);

-- ==================== Tool Capabilities ====================

CREATE TABLE IF NOT EXISTS tool_capabilities (
    id TEXT PRIMARY KEY,
    wasm_tool_id TEXT NOT NULL REFERENCES wasm_tools(id) ON DELETE CASCADE,
    http_allowlist TEXT NOT NULL DEFAULT '[]',
    allowed_secrets TEXT NOT NULL DEFAULT '[]',
    tool_aliases TEXT NOT NULL DEFAULT '{}',
    requests_per_minute INTEGER NOT NULL DEFAULT 60,
    requests_per_hour INTEGER NOT NULL DEFAULT 1000,
    max_request_body_bytes INTEGER NOT NULL DEFAULT 1048576,
    max_response_body_bytes INTEGER NOT NULL DEFAULT 10485760,
    workspace_read_prefixes TEXT NOT NULL DEFAULT '[]',
    http_timeout_secs INTEGER NOT NULL DEFAULT 30,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (wasm_tool_id)
);

-- ==================== Leak Detection Patterns ====================

CREATE TABLE IF NOT EXISTS leak_detection_patterns (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    pattern TEXT NOT NULL,
    severity TEXT NOT NULL DEFAULT 'high',
    action TEXT NOT NULL DEFAULT 'block',
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- ==================== Rate Limit State ====================

CREATE TABLE IF NOT EXISTS tool_rate_limit_state (
    id TEXT PRIMARY KEY,
    wasm_tool_id TEXT NOT NULL REFERENCES wasm_tools(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    minute_window_start TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    minute_count INTEGER NOT NULL DEFAULT 0,
    hour_window_start TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    hour_count INTEGER NOT NULL DEFAULT 0,
    UNIQUE (wasm_tool_id, user_id)
);

-- ==================== Secret Usage Audit Log ====================

CREATE TABLE IF NOT EXISTS secret_usage_log (
    id TEXT PRIMARY KEY,
    secret_id TEXT NOT NULL REFERENCES secrets(id) ON DELETE CASCADE,
    wasm_tool_id TEXT REFERENCES wasm_tools(id) ON DELETE SET NULL,
    user_id TEXT NOT NULL,
    target_host TEXT NOT NULL,
    target_path TEXT,
    success INTEGER NOT NULL,
    error_message TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_secret_usage_user ON secret_usage_log(user_id);

-- ==================== Leak Detection Events ====================

CREATE TABLE IF NOT EXISTS leak_detection_events (
    id TEXT PRIMARY KEY,
    pattern_id TEXT REFERENCES leak_detection_patterns(id) ON DELETE SET NULL,
    wasm_tool_id TEXT REFERENCES wasm_tools(id) ON DELETE SET NULL,
    user_id TEXT NOT NULL,
    source TEXT NOT NULL,
    action_taken TEXT NOT NULL,
    context_preview TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- ==================== Tool Failures ====================

CREATE TABLE IF NOT EXISTS tool_failures (
    id TEXT PRIMARY KEY,
    tool_name TEXT NOT NULL UNIQUE,
    error_message TEXT,
    error_count INTEGER DEFAULT 1,
    first_failure TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_failure TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_build_result TEXT,
    repaired_at TEXT,
    repair_attempts INTEGER DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_tool_failures_name ON tool_failures(tool_name);

-- ==================== Job Events ====================

CREATE TABLE IF NOT EXISTS job_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id TEXT NOT NULL REFERENCES agent_jobs(id),
    event_type TEXT NOT NULL,
    data TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_job_events_job ON job_events(job_id, id);

-- ==================== Routines ====================

CREATE TABLE IF NOT EXISTS routines (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    user_id TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    trigger_type TEXT NOT NULL,
    trigger_config TEXT NOT NULL,
    action_type TEXT NOT NULL,
    action_config TEXT NOT NULL,
    cooldown_secs INTEGER NOT NULL DEFAULT 300,
    max_concurrent INTEGER NOT NULL DEFAULT 1,
    dedup_window_secs INTEGER,
    notify_channel TEXT,
    notify_user TEXT,
    notify_on_success INTEGER NOT NULL DEFAULT 0,
    notify_on_failure INTEGER NOT NULL DEFAULT 1,
    notify_on_attention INTEGER NOT NULL DEFAULT 1,
    state TEXT NOT NULL DEFAULT '{}',
    last_run_at TEXT,
    next_fire_at TEXT,
    run_count INTEGER NOT NULL DEFAULT 0,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (user_id, name)
);

CREATE INDEX IF NOT EXISTS idx_routines_user ON routines(user_id);

-- ==================== Routine Runs ====================

CREATE TABLE IF NOT EXISTS routine_runs (
    id TEXT PRIMARY KEY,
    routine_id TEXT NOT NULL REFERENCES routines(id) ON DELETE CASCADE,
    trigger_type TEXT NOT NULL,
    trigger_detail TEXT,
    started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    completed_at TEXT,
    status TEXT NOT NULL DEFAULT 'running',
    result_summary TEXT,
    tokens_used INTEGER,
    job_id TEXT REFERENCES agent_jobs(id),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_routine_runs_routine ON routine_runs(routine_id);

-- ==================== Settings ====================

CREATE TABLE IF NOT EXISTS settings (
    user_id TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (user_id, key)
);

CREATE INDEX IF NOT EXISTS idx_settings_user ON settings(user_id);

-- ==================== Missing indexes carried over from earlier schema revisions ====================

-- agent_jobs
CREATE INDEX IF NOT EXISTS idx_agent_jobs_stuck ON agent_jobs(stuck_since);

-- secrets
CREATE INDEX IF NOT EXISTS idx_secrets_provider ON secrets(provider);
CREATE INDEX IF NOT EXISTS idx_secrets_expires ON secrets(expires_at);

-- wasm_tools
CREATE INDEX IF NOT EXISTS idx_wasm_tools_trust ON wasm_tools(trust_level);

-- tool_capabilities
CREATE INDEX IF NOT EXISTS idx_tool_capabilities_tool ON tool_capabilities(wasm_tool_id);

-- leak_detection_patterns
CREATE INDEX IF NOT EXISTS idx_leak_patterns_enabled ON leak_detection_patterns(enabled);

-- tool_rate_limit_state
CREATE INDEX IF NOT EXISTS idx_rate_limit_tool ON tool_rate_limit_state(wasm_tool_id);

-- secret_usage_log
CREATE INDEX IF NOT EXISTS idx_secret_usage_secret ON secret_usage_log(secret_id);
CREATE INDEX IF NOT EXISTS idx_secret_usage_tool ON secret_usage_log(wasm_tool_id);
CREATE INDEX IF NOT EXISTS idx_secret_usage_created ON secret_usage_log(created_at DESC);

-- leak_detection_events
CREATE INDEX IF NOT EXISTS idx_leak_events_pattern ON leak_detection_events(pattern_id);
CREATE INDEX IF NOT EXISTS idx_leak_events_tool ON leak_detection_events(wasm_tool_id);
CREATE INDEX IF NOT EXISTS idx_leak_events_user ON leak_detection_events(user_id);
CREATE INDEX IF NOT EXISTS idx_leak_events_created ON leak_detection_events(created_at DESC);

-- tool_failures
CREATE INDEX IF NOT EXISTS idx_tool_failures_count ON tool_failures(error_count DESC);
CREATE INDEX IF NOT EXISTS idx_tool_failures_unrepaired ON tool_failures(tool_name);

-- routines
CREATE INDEX IF NOT EXISTS idx_routines_next_fire ON routines(next_fire_at);
CREATE INDEX IF NOT EXISTS idx_routines_event_triggers
    ON routines(trigger_type, user_id)
    WHERE enabled = 1 AND trigger_type IN ('event', 'system_event');

-- routine_runs
CREATE INDEX IF NOT EXISTS idx_routine_runs_status ON routine_runs(status);

-- heartbeat_state
CREATE INDEX IF NOT EXISTS idx_heartbeat_next_run ON heartbeat_state(next_run);

-- ==================== Seed data ====================

-- Pre-populate leak detection patterns (matches the earlier V2 schema revision).
INSERT OR IGNORE INTO leak_detection_patterns (id, name, pattern, severity, action, enabled, created_at) VALUES
    ('550e8400-e29b-41d4-a716-446655440001', 'openai_api_key', 'sk-(?:proj-)?[a-zA-Z0-9]{20,}(?:T3BlbkFJ[a-zA-Z0-9_-]*)?', 'critical', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-446655440002', 'anthropic_api_key', 'sk-ant-api[a-zA-Z0-9_-]{90,}', 'critical', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-446655440003', 'aws_access_key', 'AKIA[0-9A-Z]{16}', 'critical', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-446655440004', 'aws_secret_key', '(?<![A-Za-z0-9/+=])[A-Za-z0-9/+=]{40}(?![A-Za-z0-9/+=])', 'high', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-446655440005', 'github_token', 'gh[pousr]_[A-Za-z0-9_]{36,}', 'critical', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-446655440006', 'github_fine_grained_pat', 'github_pat_[a-zA-Z0-9]{22}_[a-zA-Z0-9]{59}', 'critical', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-446655440007', 'stripe_api_key', 'sk_(?:live|test)_[a-zA-Z0-9]{24,}', 'critical', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-446655440009', 'bearer_token', 'Bearer\s+[a-zA-Z0-9_-]{20,}', 'high', 'redact', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-44665544000a', 'pem_private_key', '-----BEGIN\s+(?:RSA\s+)?PRIVATE\s+KEY-----', 'critical', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-44665544000b', 'ssh_private_key', '-----BEGIN\s+(?:OPENSSH|EC|DSA)\s+PRIVATE\s+KEY-----', 'critical', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-44665544000c', 'google_api_key', 'AIza[0-9A-Za-z_-]{35}', 'high', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-44665544000d', 'slack_token', 'xox[baprs]-[0-9a-zA-Z-]{10,}', 'high', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-44665544000e', 'discord_token', '[MN][A-Za-z\d]{23,}\.[\w-]{6}\.[\w-]{27}', 'high', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-44665544000f', 'twilio_api_key', 'SK[a-fA-F0-9]{32}', 'high', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-446655440010', 'sendgrid_api_key', 'SG\.[a-zA-Z0-9_-]{22}\.[a-zA-Z0-9_-]{43}', 'high', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-446655440011', 'mailchimp_api_key', '[a-f0-9]{32}-us[0-9]{1,2}', 'medium', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-446655440012', 'high_entropy_hex', '(?<![a-fA-F0-9])[a-fA-F0-9]{64}(?![a-fA-F0-9])', 'medium', 'warn', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));


-- ==================== User management (V14) ====================

CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    email TEXT UNIQUE,
    display_name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    role TEXT NOT NULL DEFAULT 'member',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_login_at TEXT,
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    metadata TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS api_tokens (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash BLOB NOT NULL,
    token_prefix TEXT NOT NULL,
    name TEXT NOT NULL,
    expires_at TEXT,
    last_used_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    revoked_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_api_tokens_user ON api_tokens(user_id);
CREATE INDEX IF NOT EXISTS idx_api_tokens_hash ON api_tokens(token_hash);

"#;

/// Incremental migrations applied after the base schema.
///
/// Each entry is `(version, name, sql)`. Migrations are idempotent: the
/// `_migrations` table tracks which versions have been applied.
pub const INCREMENTAL_MIGRATIONS: &[(i64, &str, &str)] = &[
    (
        9,
        "flexible_embedding_dimension",
        // Rebuild memory_chunks to remove the fixed F32_BLOB(1536) type
        // constraint so any embedding dimension works. Existing embeddings
        // are preserved; users only need to re-embed if they change models.
        //
        // The vector index is dropped here; ensure_vector_index() recreates
        // it with the correct F32_BLOB(N) dimension during run_migrations()
        // when embeddings are configured.
        //
        // SQLite cannot ALTER COLUMN types, so we recreate the table.
        r#"
-- Drop vector index (requires fixed F32_BLOB(N), incompatible with flexible dimensions)
DROP INDEX IF EXISTS idx_memory_chunks_embedding;

-- Drop FTS triggers that reference the old table
DROP TRIGGER IF EXISTS memory_chunks_fts_insert;
DROP TRIGGER IF EXISTS memory_chunks_fts_delete;
DROP TRIGGER IF EXISTS memory_chunks_fts_update;

-- Recreate table with flexible BLOB column (any embedding dimension)
CREATE TABLE IF NOT EXISTS memory_chunks_new (
    _rowid INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    document_id TEXT NOT NULL REFERENCES memory_documents(id) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL,
    content TEXT NOT NULL,
    embedding BLOB,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (document_id, chunk_index)
);

-- Copy all existing data (embeddings preserved as-is)
INSERT OR IGNORE INTO memory_chunks_new (_rowid, id, document_id, chunk_index, content, embedding, created_at)
    SELECT _rowid, id, document_id, chunk_index, content, embedding, created_at FROM memory_chunks;

-- Swap tables
DROP TABLE memory_chunks;
ALTER TABLE memory_chunks_new RENAME TO memory_chunks;

-- Recreate indexes (no vector index — see comment above)
CREATE INDEX IF NOT EXISTS idx_memory_chunks_document ON memory_chunks(document_id);

-- Recreate FTS triggers
CREATE TRIGGER IF NOT EXISTS memory_chunks_fts_insert AFTER INSERT ON memory_chunks BEGIN
    INSERT INTO memory_chunks_fts(rowid, content) VALUES (new._rowid, new.content);
END;

CREATE TRIGGER IF NOT EXISTS memory_chunks_fts_delete AFTER DELETE ON memory_chunks BEGIN
    INSERT INTO memory_chunks_fts(memory_chunks_fts, rowid, content)
        VALUES ('delete', old._rowid, old.content);
END;

CREATE TRIGGER IF NOT EXISTS memory_chunks_fts_update AFTER UPDATE ON memory_chunks BEGIN
    INSERT INTO memory_chunks_fts(memory_chunks_fts, rowid, content)
        VALUES ('delete', old._rowid, old.content);
    INSERT INTO memory_chunks_fts(rowid, content) VALUES (new._rowid, new.content);
END;
"#,
    ),
    (
        12,
        "job_token_budget",
        // Add token budget tracking columns to agent_jobs.
        // SQLite supports ALTER TABLE ADD COLUMN, so no table rebuild needed.
        r#"
ALTER TABLE agent_jobs ADD COLUMN max_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE agent_jobs ADD COLUMN total_tokens_used INTEGER NOT NULL DEFAULT 0;
"#,
    ),
    (
        13,
        "routine_notify_user_nullable",
        // Remove the legacy 'default' sentinel from routine notify_user.
        // SQLite cannot drop NOT NULL / DEFAULT constraints in place, so we
        // rebuild the table and normalize existing 'default' values to NULL.
        r#"
PRAGMA foreign_keys=OFF;

CREATE TABLE IF NOT EXISTS routines_new (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    user_id TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    trigger_type TEXT NOT NULL,
    trigger_config TEXT NOT NULL,
    action_type TEXT NOT NULL,
    action_config TEXT NOT NULL,
    cooldown_secs INTEGER NOT NULL DEFAULT 300,
    max_concurrent INTEGER NOT NULL DEFAULT 1,
    dedup_window_secs INTEGER,
    notify_channel TEXT,
    notify_user TEXT,
    notify_on_success INTEGER NOT NULL DEFAULT 0,
    notify_on_failure INTEGER NOT NULL DEFAULT 1,
    notify_on_attention INTEGER NOT NULL DEFAULT 1,
    state TEXT NOT NULL DEFAULT '{}',
    last_run_at TEXT,
    next_fire_at TEXT,
    run_count INTEGER NOT NULL DEFAULT 0,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (user_id, name)
);

INSERT INTO routines_new (
    id, name, description, user_id, enabled,
    trigger_type, trigger_config, action_type, action_config,
    cooldown_secs, max_concurrent, dedup_window_secs,
    notify_channel, notify_user, notify_on_success, notify_on_failure, notify_on_attention,
    state, last_run_at, next_fire_at, run_count, consecutive_failures,
    created_at, updated_at
)
SELECT
    id, name, description, user_id, enabled,
    trigger_type, trigger_config, action_type, action_config,
    cooldown_secs, max_concurrent, dedup_window_secs,
    notify_channel,
    CASE WHEN notify_user = 'default' THEN NULL ELSE notify_user END,
    notify_on_success, notify_on_failure, notify_on_attention,
    state, last_run_at, next_fire_at, run_count, consecutive_failures,
    created_at, updated_at
FROM routines;

DROP TABLE routines;
ALTER TABLE routines_new RENAME TO routines;

CREATE INDEX IF NOT EXISTS idx_routines_user ON routines(user_id);
CREATE INDEX IF NOT EXISTS idx_routines_next_fire ON routines(next_fire_at);
CREATE INDEX IF NOT EXISTS idx_routines_event_triggers
    ON routines(trigger_type, user_id)
    WHERE enabled = 1 AND trigger_type IN ('event', 'system_event');

PRAGMA foreign_keys=ON;
"#,
    ),
    (
        14,
        "users",
        r#"
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    email TEXT UNIQUE,
    display_name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    role TEXT NOT NULL DEFAULT 'member',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_login_at TEXT,
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    metadata TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS api_tokens (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash BLOB NOT NULL,
    token_prefix TEXT NOT NULL,
    name TEXT NOT NULL,
    expires_at TEXT,
    last_used_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    revoked_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_api_tokens_user ON api_tokens(user_id);
CREATE INDEX IF NOT EXISTS idx_api_tokens_hash ON api_tokens(token_hash);
"#,
    ),
    (
        15,
        "task_templates",
        r#"
CREATE TABLE IF NOT EXISTS task_templates (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    parameter_schema TEXT NOT NULL,
    default_mode TEXT NOT NULL,
    output_expectations TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_task_templates_user ON task_templates(user_id);
CREATE INDEX IF NOT EXISTS idx_task_templates_user_name ON task_templates(user_id, name);
"#,
    ),
    (
        16,
        "task_history",
        r#"
CREATE TABLE IF NOT EXISTS task_records (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    template_id TEXT NOT NULL,
    mode TEXT NOT NULL,
    status TEXT NOT NULL,
    title TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    current_step TEXT,
    pending_approval TEXT,
    route TEXT NOT NULL,
    last_error TEXT,
    result_metadata TEXT
);

CREATE INDEX IF NOT EXISTS idx_task_records_user_updated
    ON task_records(user_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS task_timeline_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id TEXT NOT NULL,
    task_id TEXT NOT NULL REFERENCES task_records(id) ON DELETE CASCADE,
    event TEXT NOT NULL,
    status TEXT NOT NULL,
    mode TEXT NOT NULL,
    current_step TEXT,
    pending_approval TEXT,
    last_error TEXT,
    result_metadata TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_task_timeline_events_task
    ON task_timeline_events(user_id, task_id, id ASC);
"#,
    ),
    (
        17,
        "workspace_allowlist_branches",
        r#"
CREATE TABLE IF NOT EXISTS workspace_allowlists (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    source_root TEXT NOT NULL,
    bypass_read INTEGER NOT NULL DEFAULT 1,
    bypass_write INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_workspace_allowlists_user
    ON workspace_allowlists(user_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS workspace_allowlist_snapshots (
    id TEXT PRIMARY KEY,
    allowlist_id TEXT NOT NULL REFERENCES workspace_allowlists(id) ON DELETE CASCADE,
    relative_path TEXT NOT NULL,
    content BLOB NOT NULL,
    is_binary INTEGER NOT NULL DEFAULT 0,
    content_hash TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_workspace_allowlist_snapshots_allowlist_path
    ON workspace_allowlist_snapshots(allowlist_id, relative_path, created_at DESC);

CREATE TABLE IF NOT EXISTS workspace_allowlist_files (
    allowlist_id TEXT NOT NULL REFERENCES workspace_allowlists(id) ON DELETE CASCADE,
    relative_path TEXT NOT NULL,
    status TEXT NOT NULL,
    is_binary INTEGER NOT NULL DEFAULT 0,
    base_snapshot_id TEXT REFERENCES workspace_allowlist_snapshots(id) ON DELETE SET NULL,
    working_snapshot_id TEXT REFERENCES workspace_allowlist_snapshots(id) ON DELETE SET NULL,
    remote_hash TEXT,
    base_hash TEXT,
    working_hash TEXT,
    conflict_reason TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (allowlist_id, relative_path)
);

CREATE INDEX IF NOT EXISTS idx_workspace_allowlist_files_allowlist_status
    ON workspace_allowlist_files(allowlist_id, status, updated_at DESC);

CREATE TABLE IF NOT EXISTS workspace_allowlist_checkpoints (
    id TEXT PRIMARY KEY,
    allowlist_id TEXT NOT NULL REFERENCES workspace_allowlists(id) ON DELETE CASCADE,
    parent_checkpoint_id TEXT REFERENCES workspace_allowlist_checkpoints(id) ON DELETE SET NULL,
    label TEXT,
    summary TEXT,
    created_by TEXT NOT NULL,
    is_auto INTEGER NOT NULL DEFAULT 0,
    base_generation INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_workspace_allowlist_checkpoints_allowlist_created
    ON workspace_allowlist_checkpoints(allowlist_id, created_at DESC);

CREATE TABLE IF NOT EXISTS workspace_allowlist_checkpoint_files (
    checkpoint_id TEXT NOT NULL REFERENCES workspace_allowlist_checkpoints(id) ON DELETE CASCADE,
    relative_path TEXT NOT NULL,
    status TEXT NOT NULL,
    snapshot_id TEXT REFERENCES workspace_allowlist_snapshots(id) ON DELETE SET NULL,
    PRIMARY KEY (checkpoint_id, relative_path)
);
"#,
    ),
    (
        18,
        "conversation_message_metadata",
        r#"
ALTER TABLE conversation_messages ADD COLUMN metadata TEXT NOT NULL DEFAULT '{}';
"#,
    ),
    (
        19,
        "native_memory_graph",
        r#"
CREATE TABLE IF NOT EXISTS memory_spaces (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    agent_id TEXT,
    slug TEXT NOT NULL,
    title TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (owner_id, agent_id, slug)
);

CREATE TABLE IF NOT EXISTS memory_nodes (
    id TEXT PRIMARY KEY,
    space_id TEXT NOT NULL REFERENCES memory_spaces(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_memory_nodes_space_kind
    ON memory_nodes(space_id, kind, updated_at DESC);

CREATE TABLE IF NOT EXISTS memory_versions (
    id TEXT PRIMARY KEY,
    node_id TEXT NOT NULL REFERENCES memory_nodes(id) ON DELETE CASCADE,
    supersedes_version_id TEXT REFERENCES memory_versions(id) ON DELETE SET NULL,
    status TEXT NOT NULL,
    content TEXT NOT NULL,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_memory_versions_node_created
    ON memory_versions(node_id, created_at DESC);

CREATE TABLE IF NOT EXISTS memory_edges (
    id TEXT PRIMARY KEY,
    space_id TEXT NOT NULL REFERENCES memory_spaces(id) ON DELETE CASCADE,
    parent_node_id TEXT REFERENCES memory_nodes(id) ON DELETE CASCADE,
    child_node_id TEXT NOT NULL REFERENCES memory_nodes(id) ON DELETE CASCADE,
    relation_kind TEXT NOT NULL DEFAULT 'contains',
    visibility TEXT NOT NULL DEFAULT 'private',
    priority INTEGER NOT NULL DEFAULT 100,
    trigger_text TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_memory_edges_space_child
    ON memory_edges(space_id, child_node_id, priority, updated_at DESC);

CREATE TABLE IF NOT EXISTS memory_routes (
    id TEXT PRIMARY KEY,
    space_id TEXT NOT NULL REFERENCES memory_spaces(id) ON DELETE CASCADE,
    edge_id TEXT REFERENCES memory_edges(id) ON DELETE CASCADE,
    node_id TEXT NOT NULL REFERENCES memory_nodes(id) ON DELETE CASCADE,
    domain TEXT NOT NULL,
    path TEXT NOT NULL,
    is_primary INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (space_id, domain, path)
);

CREATE INDEX IF NOT EXISTS idx_memory_routes_space_node
    ON memory_routes(space_id, node_id, is_primary DESC);

CREATE TABLE IF NOT EXISTS memory_boot_entries (
    route_id TEXT PRIMARY KEY REFERENCES memory_routes(id) ON DELETE CASCADE,
    space_id TEXT NOT NULL REFERENCES memory_spaces(id) ON DELETE CASCADE,
    load_priority INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_memory_boot_entries_space_priority
    ON memory_boot_entries(space_id, load_priority, updated_at DESC);

CREATE TABLE IF NOT EXISTS memory_keywords (
    id TEXT PRIMARY KEY,
    space_id TEXT NOT NULL REFERENCES memory_spaces(id) ON DELETE CASCADE,
    node_id TEXT NOT NULL REFERENCES memory_nodes(id) ON DELETE CASCADE,
    keyword TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (space_id, node_id, keyword)
);

CREATE INDEX IF NOT EXISTS idx_memory_keywords_node
    ON memory_keywords(node_id, keyword);

CREATE TABLE IF NOT EXISTS memory_search_docs (
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
    embedding BLOB,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_memory_search_docs_space_updated
    ON memory_search_docs(space_id, updated_at DESC);

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
END;

CREATE TABLE IF NOT EXISTS memory_changesets (
    id TEXT PRIMARY KEY,
    space_id TEXT NOT NULL REFERENCES memory_spaces(id) ON DELETE CASCADE,
    origin TEXT NOT NULL,
    summary TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_memory_changesets_space_status
    ON memory_changesets(space_id, status, created_at DESC);

CREATE TABLE IF NOT EXISTS memory_changeset_rows (
    id TEXT PRIMARY KEY,
    changeset_id TEXT NOT NULL REFERENCES memory_changesets(id) ON DELETE CASCADE,
    node_id TEXT REFERENCES memory_nodes(id) ON DELETE SET NULL,
    route_id TEXT REFERENCES memory_routes(id) ON DELETE SET NULL,
    operation TEXT NOT NULL,
    before_json TEXT NOT NULL DEFAULT 'null',
    after_json TEXT NOT NULL DEFAULT 'null',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_memory_changeset_rows_changeset
    ON memory_changeset_rows(changeset_id, created_at ASC);
"#,
    ),
    (
        20,
        "memory_recall_boot_and_search_terms",
        r#"
CREATE TABLE IF NOT EXISTS memory_boot_entries (
    route_id TEXT PRIMARY KEY REFERENCES memory_routes(id) ON DELETE CASCADE,
    space_id TEXT NOT NULL REFERENCES memory_spaces(id) ON DELETE CASCADE,
    load_priority INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_memory_boot_entries_space_priority
    ON memory_boot_entries(space_id, load_priority, updated_at DESC);

DROP TRIGGER IF EXISTS memory_search_docs_fts_insert;
DROP TRIGGER IF EXISTS memory_search_docs_fts_delete;
DROP TRIGGER IF EXISTS memory_search_docs_fts_update;
DROP TABLE IF EXISTS memory_search_docs_fts;
DROP TABLE IF EXISTS memory_search_docs_new;

CREATE TABLE memory_search_docs_new (
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
    embedding BLOB,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

INSERT OR REPLACE INTO memory_search_docs_new
    (route_id, space_id, node_id, version_id, uri, title, kind, content, trigger_text, keywords, search_terms, embedding, updated_at)
SELECT
    route_id,
    space_id,
    node_id,
    version_id,
    uri,
    title,
    kind,
    content,
    trigger_text,
    keywords,
    trim(lower(title || ' ' || uri || ' ' || content || ' ' || coalesce(trigger_text, '') || ' ' || keywords)),
    NULL,
    updated_at
FROM memory_search_docs;

DROP TABLE memory_search_docs;
ALTER TABLE memory_search_docs_new RENAME TO memory_search_docs;

CREATE INDEX IF NOT EXISTS idx_memory_search_docs_space_updated
    ON memory_search_docs(space_id, updated_at DESC);

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
END;

INSERT OR IGNORE INTO memory_boot_entries (route_id, space_id, load_priority)
SELECT r.id, r.space_id, coalesce(e.priority, 0)
FROM memory_routes r
JOIN memory_nodes n ON n.id = r.node_id
LEFT JOIN memory_edges e ON e.id = r.edge_id
WHERE n.kind = 'boot';
"#,
    ),
    (
        21,
        "conversation_history_recall",
        r#"
CREATE TABLE IF NOT EXISTS conversation_recall_docs (
    doc_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    channel TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    turn_index INTEGER NOT NULL,
    user_message_id TEXT NOT NULL,
    assistant_message_id TEXT NOT NULL,
    turn_timestamp TEXT NOT NULL,
    user_text TEXT NOT NULL,
    assistant_text TEXT NOT NULL,
    search_text TEXT NOT NULL,
    preview_text TEXT NOT NULL,
    embedding BLOB,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_conversation_recall_docs_user_updated
    ON conversation_recall_docs(user_id, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_conversation_recall_docs_conversation_turn
    ON conversation_recall_docs(conversation_id, turn_index);
CREATE INDEX IF NOT EXISTS idx_conversation_recall_docs_thread
    ON conversation_recall_docs(user_id, thread_id);

CREATE VIRTUAL TABLE IF NOT EXISTS conversation_recall_docs_fts USING fts5(
    user_text,
    assistant_text,
    search_text,
    preview_text,
    content='conversation_recall_docs',
    content_rowid='rowid'
);

CREATE TRIGGER IF NOT EXISTS conversation_recall_docs_fts_insert AFTER INSERT ON conversation_recall_docs BEGIN
    INSERT INTO conversation_recall_docs_fts(rowid, user_text, assistant_text, search_text, preview_text)
    VALUES (new.rowid, new.user_text, new.assistant_text, new.search_text, new.preview_text);
END;

CREATE TRIGGER IF NOT EXISTS conversation_recall_docs_fts_delete AFTER DELETE ON conversation_recall_docs BEGIN
    INSERT INTO conversation_recall_docs_fts(conversation_recall_docs_fts, rowid, user_text, assistant_text, search_text, preview_text)
    VALUES ('delete', old.rowid, old.user_text, old.assistant_text, old.search_text, old.preview_text);
END;

CREATE TRIGGER IF NOT EXISTS conversation_recall_docs_fts_update AFTER UPDATE ON conversation_recall_docs BEGIN
    INSERT INTO conversation_recall_docs_fts(conversation_recall_docs_fts, rowid, user_text, assistant_text, search_text, preview_text)
    VALUES ('delete', old.rowid, old.user_text, old.assistant_text, old.search_text, old.preview_text);
    INSERT INTO conversation_recall_docs_fts(rowid, user_text, assistant_text, search_text, preview_text)
    VALUES (new.rowid, new.user_text, new.assistant_text, new.search_text, new.preview_text);
END;
"#,
    ),
    (
        22,
        "routine_run_trigger_payload",
        r#"
ALTER TABLE routine_runs ADD COLUMN trigger_payload TEXT;
"#,
    ),
    (
        23,
        "workspace_allowlist_revisions",
        r#"
CREATE TABLE IF NOT EXISTS workspace_allowlist_revisions (
    id TEXT PRIMARY KEY,
    allowlist_id TEXT NOT NULL REFERENCES workspace_allowlists(id) ON DELETE CASCADE,
    parent_revision_id TEXT REFERENCES workspace_allowlist_revisions(id) ON DELETE SET NULL,
    kind TEXT NOT NULL,
    source TEXT NOT NULL,
    trigger TEXT,
    summary TEXT,
    created_by TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_workspace_allowlist_revisions_allowlist_created
    ON workspace_allowlist_revisions(allowlist_id, created_at DESC);

CREATE TABLE IF NOT EXISTS workspace_allowlist_revision_files (
    revision_id TEXT NOT NULL REFERENCES workspace_allowlist_revisions(id) ON DELETE CASCADE,
    relative_path TEXT NOT NULL,
    change_kind TEXT NOT NULL,
    before_snapshot_id TEXT REFERENCES workspace_allowlist_snapshots(id) ON DELETE SET NULL,
    after_snapshot_id TEXT REFERENCES workspace_allowlist_snapshots(id) ON DELETE SET NULL,
    before_mode INTEGER,
    after_mode INTEGER,
    is_binary INTEGER NOT NULL DEFAULT 0,
    rename_from TEXT,
    rename_to TEXT,
    PRIMARY KEY (revision_id, relative_path)
);

CREATE INDEX IF NOT EXISTS idx_workspace_allowlist_revision_files_revision
    ON workspace_allowlist_revision_files(revision_id, change_kind, relative_path);

CREATE TABLE IF NOT EXISTS workspace_allowlist_manifests (
    revision_id TEXT NOT NULL REFERENCES workspace_allowlist_revisions(id) ON DELETE CASCADE,
    relative_path TEXT NOT NULL,
    snapshot_id TEXT REFERENCES workspace_allowlist_snapshots(id) ON DELETE SET NULL,
    file_mode INTEGER,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    modified_at INTEGER,
    is_binary INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (revision_id, relative_path)
);

CREATE INDEX IF NOT EXISTS idx_workspace_allowlist_manifests_revision
    ON workspace_allowlist_manifests(revision_id, relative_path);

CREATE TABLE IF NOT EXISTS workspace_allowlist_state (
    allowlist_id TEXT PRIMARY KEY REFERENCES workspace_allowlists(id) ON DELETE CASCADE,
    baseline_revision_id TEXT REFERENCES workspace_allowlist_revisions(id) ON DELETE SET NULL,
    head_revision_id TEXT REFERENCES workspace_allowlist_revisions(id) ON DELETE SET NULL,
    last_reconciled_at TEXT,
    watch_dirty INTEGER NOT NULL DEFAULT 0,
    watch_cursor TEXT
);

ALTER TABLE workspace_allowlist_checkpoints ADD COLUMN revision_id TEXT REFERENCES workspace_allowlist_revisions(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_workspace_allowlist_checkpoints_revision
    ON workspace_allowlist_checkpoints(allowlist_id, revision_id, created_at DESC);
"#,
    ),
    (24, "workspace_allowlist_rename_cleanup", ""),
];

async fn sqlite_object_exists(
    conn: &libsql::Transaction,
    object_type: &str,
    name: &str,
) -> Result<bool, crate::error::DatabaseError> {
    use crate::error::DatabaseError;

    let mut rows = conn
        .query(
            "SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2 LIMIT 1",
            libsql::params![object_type, name],
        )
        .await
        .map_err(|e| {
            DatabaseError::Migration(format!(
                "Failed to inspect sqlite_master for {object_type} {name}: {e}"
            ))
        })?;

    Ok(rows.next().await.ok().flatten().is_some())
}

async fn sqlite_column_exists(
    conn: &libsql::Transaction,
    table: &str,
    column: &str,
) -> Result<bool, crate::error::DatabaseError> {
    use crate::error::DatabaseError;

    let pragma = format!("PRAGMA table_info({table})");
    let mut rows = conn.query(&pragma, libsql::params![]).await.map_err(|e| {
        DatabaseError::Migration(format!("Failed to inspect columns for {table}: {e}"))
    })?;

    while let Some(row) = rows.next().await.ok().flatten() {
        let name = row
            .get_value(1)
            .ok()
            .and_then(|value| value.as_text().cloned());
        if name.as_deref() == Some(column) {
            return Ok(true);
        }
    }

    Ok(false)
}

async fn rename_table_if_needed(
    conn: &libsql::Transaction,
    old: &str,
    new: &str,
) -> Result<(), crate::error::DatabaseError> {
    use crate::error::DatabaseError;

    if !sqlite_object_exists(conn, "table", old).await?
        || sqlite_object_exists(conn, "table", new).await?
    {
        return Ok(());
    }

    conn.execute_batch(&format!("ALTER TABLE {old} RENAME TO {new};"))
        .await
        .map_err(|e| {
            DatabaseError::Migration(format!("Failed to rename table {old} -> {new}: {e}"))
        })?;

    Ok(())
}

async fn rename_column_if_needed(
    conn: &libsql::Transaction,
    table: &str,
    old: &str,
    new: &str,
) -> Result<(), crate::error::DatabaseError> {
    use crate::error::DatabaseError;

    if !sqlite_object_exists(conn, "table", table).await?
        || !sqlite_column_exists(conn, table, old).await?
        || sqlite_column_exists(conn, table, new).await?
    {
        return Ok(());
    }

    conn.execute_batch(&format!(
        "ALTER TABLE {table} RENAME COLUMN {old} TO {new};"
    ))
    .await
    .map_err(|e| {
        DatabaseError::Migration(format!(
            "Failed to rename column {table}.{old} -> {new}: {e}"
        ))
    })?;

    Ok(())
}

async fn execute_batch_if_table_exists(
    conn: &libsql::Transaction,
    table: &str,
    sql: &str,
) -> Result<(), crate::error::DatabaseError> {
    use crate::error::DatabaseError;

    if !sqlite_object_exists(conn, "table", table).await? {
        return Ok(());
    }

    conn.execute_batch(sql).await.map_err(|e| {
        DatabaseError::Migration(format!("Failed to update schema for {table}: {e}"))
    })?;

    Ok(())
}

async fn run_workspace_allowlist_rename_cleanup(
    conn: &libsql::Transaction,
) -> Result<(), crate::error::DatabaseError> {
    use crate::error::DatabaseError;

    rename_table_if_needed(conn, "workspace_mounts", "workspace_allowlists").await?;
    rename_table_if_needed(
        conn,
        "workspace_mount_snapshots",
        "workspace_allowlist_snapshots",
    )
    .await?;
    rename_table_if_needed(conn, "workspace_mount_files", "workspace_allowlist_files").await?;
    rename_table_if_needed(
        conn,
        "workspace_mount_checkpoints",
        "workspace_allowlist_checkpoints",
    )
    .await?;
    rename_table_if_needed(
        conn,
        "workspace_mount_checkpoint_files",
        "workspace_allowlist_checkpoint_files",
    )
    .await?;
    rename_table_if_needed(
        conn,
        "workspace_mount_revisions",
        "workspace_allowlist_revisions",
    )
    .await?;
    rename_table_if_needed(
        conn,
        "workspace_mount_revision_files",
        "workspace_allowlist_revision_files",
    )
    .await?;
    rename_table_if_needed(
        conn,
        "workspace_mount_manifests",
        "workspace_allowlist_manifests",
    )
    .await?;
    rename_table_if_needed(conn, "workspace_mount_state", "workspace_allowlist_state").await?;

    rename_column_if_needed(
        conn,
        "workspace_allowlist_snapshots",
        "mount_id",
        "allowlist_id",
    )
    .await?;
    rename_column_if_needed(
        conn,
        "workspace_allowlist_files",
        "mount_id",
        "allowlist_id",
    )
    .await?;
    rename_column_if_needed(
        conn,
        "workspace_allowlist_checkpoints",
        "mount_id",
        "allowlist_id",
    )
    .await?;
    rename_column_if_needed(
        conn,
        "workspace_allowlist_revisions",
        "mount_id",
        "allowlist_id",
    )
    .await?;
    rename_column_if_needed(
        conn,
        "workspace_allowlist_state",
        "mount_id",
        "allowlist_id",
    )
    .await?;

    execute_batch_if_table_exists(
        conn,
        "workspace_allowlists",
        r#"
DROP INDEX IF EXISTS idx_workspace_mounts_user;
CREATE INDEX IF NOT EXISTS idx_workspace_allowlists_user
    ON workspace_allowlists(user_id, updated_at DESC);
"#,
    )
    .await?;

    execute_batch_if_table_exists(
        conn,
        "workspace_allowlist_snapshots",
        r#"
DROP INDEX IF EXISTS idx_workspace_mount_snapshots_mount_path;
CREATE INDEX IF NOT EXISTS idx_workspace_allowlist_snapshots_allowlist_path
    ON workspace_allowlist_snapshots(allowlist_id, relative_path, created_at DESC);
"#,
    )
    .await?;

    execute_batch_if_table_exists(
        conn,
        "workspace_allowlist_files",
        r#"
DROP INDEX IF EXISTS idx_workspace_mount_files_mount_status;
CREATE INDEX IF NOT EXISTS idx_workspace_allowlist_files_allowlist_status
    ON workspace_allowlist_files(allowlist_id, status, updated_at DESC);
"#,
    )
    .await?;

    execute_batch_if_table_exists(
        conn,
        "workspace_allowlist_checkpoints",
        r#"
DROP INDEX IF EXISTS idx_workspace_mount_checkpoints_mount;
DROP INDEX IF EXISTS idx_workspace_allowlist_checkpoints_mount;
DROP INDEX IF EXISTS idx_workspace_mount_checkpoints_revision;
CREATE INDEX IF NOT EXISTS idx_workspace_allowlist_checkpoints_allowlist_created
    ON workspace_allowlist_checkpoints(allowlist_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_workspace_allowlist_checkpoints_revision
    ON workspace_allowlist_checkpoints(allowlist_id, revision_id, created_at DESC);
"#,
    )
    .await?;

    execute_batch_if_table_exists(
        conn,
        "workspace_allowlist_revisions",
        r#"
DROP INDEX IF EXISTS idx_workspace_mount_revisions_mount_created;
CREATE INDEX IF NOT EXISTS idx_workspace_allowlist_revisions_allowlist_created
    ON workspace_allowlist_revisions(allowlist_id, created_at DESC);
"#,
    )
    .await?;

    execute_batch_if_table_exists(
        conn,
        "workspace_allowlist_revision_files",
        r#"
DROP INDEX IF EXISTS idx_workspace_mount_revision_files_revision;
CREATE INDEX IF NOT EXISTS idx_workspace_allowlist_revision_files_revision
    ON workspace_allowlist_revision_files(revision_id, change_kind, relative_path);
"#,
    )
    .await?;

    execute_batch_if_table_exists(
        conn,
        "workspace_allowlist_manifests",
        r#"
DROP INDEX IF EXISTS idx_workspace_mount_manifests_revision;
CREATE INDEX IF NOT EXISTS idx_workspace_allowlist_manifests_revision
    ON workspace_allowlist_manifests(revision_id, relative_path);
"#,
    )
    .await?;

    conn.execute(
        "UPDATE _migrations SET name = 'workspace_allowlist_branches' WHERE version = 17",
        libsql::params![],
    )
    .await
    .map_err(|e| {
        DatabaseError::Migration(format!(
            "Failed to update migration name for workspace allowlists (V17): {e}"
        ))
    })?;

    conn.execute(
        "UPDATE _migrations SET name = 'workspace_allowlist_revisions' WHERE version = 23",
        libsql::params![],
    )
    .await
    .map_err(|e| {
        DatabaseError::Migration(format!(
            "Failed to update migration name for workspace allowlists (V23): {e}"
        ))
    })?;

    Ok(())
}

/// Run incremental migrations that haven't been applied yet.
///
/// Each migration is wrapped in a transaction. On success the version is
/// recorded in `_migrations` so it won't run again.
pub async fn run_incremental(conn: &libsql::Connection) -> Result<(), crate::error::DatabaseError> {
    use crate::error::DatabaseError;

    let mut applied_count = 0;
    for &(version, name, sql) in INCREMENTAL_MIGRATIONS {
        // Check if already applied
        let mut rows = conn
            .query(
                "SELECT 1 FROM _migrations WHERE version = ?1",
                libsql::params![version],
            )
            .await
            .map_err(|e| {
                DatabaseError::Migration(format!("Failed to check migration {version}: {e}"))
            })?;

        if rows.next().await.ok().flatten().is_some() {
            continue; // Already applied
        }

        // Wrap migration + recording in a transaction for atomicity.
        // If the process crashes mid-migration, the transaction rolls back
        // and the migration will be retried on next startup.
        let tx = conn.transaction().await.map_err(|e| {
            DatabaseError::Migration(format!(
                "libSQL migration V{version}: failed to start transaction: {e}"
            ))
        })?;

        if version == 24 {
            run_workspace_allowlist_rename_cleanup(&tx).await?;
        } else {
            tx.execute_batch(sql).await.map_err(|e| {
                DatabaseError::Migration(format!(
                    "libSQL migration V{version} ({name}) failed: {e}"
                ))
            })?;
        }

        // Record as applied (inside the same transaction)
        tx.execute(
            "INSERT INTO _migrations (version, name) VALUES (?1, ?2)",
            libsql::params![version, name],
        )
        .await
        .map_err(|e| {
            DatabaseError::Migration(format!(
                "Failed to record migration V{version} ({name}): {e}"
            ))
        })?;

        tx.commit().await.map_err(|e| {
            DatabaseError::Migration(format!(
                "libSQL migration V{version} ({name}): commit failed: {e}"
            ))
        })?;

        applied_count += 1;
        tracing::debug!(version, name, "libSQL: migration applied");
    }

    if applied_count > 0 {
        tracing::info!("libSQL: applied {} incremental migrations", applied_count);
    }

    Ok(())
}
