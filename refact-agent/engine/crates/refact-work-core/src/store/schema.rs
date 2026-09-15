pub const MIGRATIONS: &[&str] = &[
    V1_COORDINATION_CORE,
    V2_ACTIVITY_OBSERVATIONS,
    V3_ENGINE_PRESENCE_MARK,
    V4_ACTIVITY_PROVENANCE,
    V5_LEASE_HOLDER_INSTANCE,
];

pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

const V1_COORDINATION_CORE: &str = "
CREATE TABLE engine_instances (
    project TEXT NOT NULL,
    instance_id TEXT NOT NULL,
    pid INTEGER NOT NULL,
    boot_id TEXT NOT NULL,
    process_started_at_ms INTEGER NOT NULL,
    host_id TEXT NOT NULL,
    heartbeat_at_ms INTEGER NOT NULL,
    PRIMARY KEY (project, instance_id)
) WITHOUT ROWID;
CREATE INDEX idx_engine_instances_heartbeat
    ON engine_instances (project, heartbeat_at_ms);

CREATE TABLE run_intents (
    project TEXT NOT NULL,
    runtime TEXT NOT NULL,
    work_id TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    generation INTEGER NOT NULL,
    state TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (project, runtime, work_id, attempt)
) WITHOUT ROWID;
CREATE INDEX idx_run_intents_state
    ON run_intents (project, state, updated_at_ms);

CREATE TABLE queued_work (
    project TEXT NOT NULL,
    runtime TEXT NOT NULL,
    work_id TEXT NOT NULL,
    root TEXT NOT NULL,
    origin TEXT NOT NULL,
    phase TEXT NOT NULL,
    enqueued_at_ms INTEGER NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (project, runtime, work_id)
) WITHOUT ROWID;
CREATE INDEX idx_queued_work_phase
    ON queued_work (project, phase, enqueued_at_ms);

CREATE TABLE leases (
    project TEXT NOT NULL,
    lease_key TEXT NOT NULL,
    owner_token TEXT NOT NULL,
    fencing_generation INTEGER NOT NULL,
    acquired_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    PRIMARY KEY (project, lease_key)
) WITHOUT ROWID;
CREATE INDEX idx_leases_expiry
    ON leases (project, expires_at_ms);

CREATE TABLE delivery_outbox (
    project TEXT NOT NULL,
    message_id TEXT NOT NULL,
    recipient TEXT NOT NULL,
    state TEXT NOT NULL,
    attempts INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (project, message_id)
) WITHOUT ROWID;
CREATE INDEX idx_delivery_outbox_state
    ON delivery_outbox (project, state, updated_at_ms);
";

const V2_ACTIVITY_OBSERVATIONS: &str = "
CREATE TABLE activity_observations (
    project TEXT NOT NULL,
    record_id TEXT NOT NULL,
    runtime TEXT NOT NULL,
    work_id TEXT NOT NULL,
    path TEXT NOT NULL,
    access TEXT NOT NULL,
    content_hash TEXT,
    observed_at_ms INTEGER NOT NULL,
    PRIMARY KEY (project, record_id)
) WITHOUT ROWID;
CREATE INDEX idx_activity_observations_path
    ON activity_observations (project, path, observed_at_ms);
CREATE INDEX idx_activity_observations_time
    ON activity_observations (project, observed_at_ms);
";

const V3_ENGINE_PRESENCE_MARK: &str = "
ALTER TABLE engine_instances ADD COLUMN presence_mark TEXT NOT NULL DEFAULT 'idle';
";

const V4_ACTIVITY_PROVENANCE: &str = "
ALTER TABLE activity_observations ADD COLUMN file_key TEXT NOT NULL DEFAULT '';
ALTER TABLE activity_observations ADD COLUMN file_alias TEXT;
ALTER TABLE activity_observations ADD COLUMN instance_id TEXT NOT NULL DEFAULT '';
ALTER TABLE activity_observations ADD COLUMN tool_call TEXT;
ALTER TABLE activity_observations ADD COLUMN content_hash_before TEXT;
ALTER TABLE activity_observations ADD COLUMN checkout TEXT;
UPDATE activity_observations SET file_key = path WHERE file_key = '';
CREATE INDEX idx_activity_observations_file_key
    ON activity_observations (project, file_key, access, observed_at_ms);
";

const V5_LEASE_HOLDER_INSTANCE: &str = "
ALTER TABLE leases ADD COLUMN holder_instance TEXT NOT NULL DEFAULT '';
";
