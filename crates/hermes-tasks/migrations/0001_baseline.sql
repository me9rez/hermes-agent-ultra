PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;

CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY NOT NULL,
    owner_user_id TEXT NOT NULL,
    primary_device_id TEXT NOT NULL,
    title TEXT NOT NULL,
    vertical_id TEXT,
    status TEXT NOT NULL,
    parent_task_id TEXT,
    persona_stack_json TEXT NOT NULL DEFAULT '[]',
    schedule_json TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (parent_task_id) REFERENCES tasks(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_tasks_owner_updated ON tasks(owner_user_id, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
CREATE INDEX IF NOT EXISTS idx_tasks_vertical ON tasks(vertical_id);

CREATE TABLE IF NOT EXISTS task_turns (
    id TEXT PRIMARY KEY NOT NULL,
    task_id TEXT NOT NULL,
    instruction_event_id TEXT NOT NULL,
    label TEXT NOT NULL,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    status TEXT NOT NULL,
    artifact_count INTEGER NOT NULL DEFAULT 0,
    approval_count INTEGER NOT NULL DEFAULT 0,
    error_count INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cost_usd_cents INTEGER NOT NULL DEFAULT 0,
    sub_task_ids_json TEXT NOT NULL DEFAULT '[]',
    FOREIGN KEY (task_id) REFERENCES tasks(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_turns_task_started ON task_turns(task_id, started_at ASC);

CREATE TABLE IF NOT EXISTS task_events (
    id TEXT PRIMARY KEY NOT NULL,
    task_id TEXT NOT NULL,
    parent_event_id TEXT,
    kind TEXT NOT NULL,
    actor_json TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    collapsed_by_default INTEGER NOT NULL DEFAULT 0,
    streaming INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    duration_ms INTEGER,
    input_tokens INTEGER,
    output_tokens INTEGER,
    cost_usd_cents INTEGER,
    turn_id TEXT,
    toc_label TEXT,
    toc_icon TEXT,
    anchor_slug TEXT NOT NULL,
    FOREIGN KEY (task_id) REFERENCES tasks(id) ON DELETE CASCADE,
    FOREIGN KEY (parent_event_id) REFERENCES task_events(id) ON DELETE SET NULL,
    FOREIGN KEY (turn_id) REFERENCES task_turns(id) ON DELETE SET NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_events_task_anchor ON task_events(task_id, anchor_slug);
CREATE INDEX IF NOT EXISTS idx_events_task_created ON task_events(task_id, created_at ASC);
CREATE INDEX IF NOT EXISTS idx_events_turn ON task_events(turn_id, created_at ASC);

CREATE TABLE IF NOT EXISTS artifacts (
    id TEXT PRIMARY KEY NOT NULL,
    task_id TEXT NOT NULL,
    owner_user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    ext TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    created_at TEXT NOT NULL,
    metadata_json TEXT,
    FOREIGN KEY (task_id) REFERENCES tasks(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_artifacts_task ON artifacts(task_id, created_at DESC);

CREATE TABLE IF NOT EXISTS task_sessions (
    task_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    PRIMARY KEY (task_id, session_id),
    FOREIGN KEY (task_id) REFERENCES tasks(id) ON DELETE CASCADE
);
