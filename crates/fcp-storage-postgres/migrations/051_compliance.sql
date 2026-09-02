-- Compliance basics: per-tenant retention + recording-consent policy, the
-- admin audit log, and retention run reports. See docs/COMPLIANCE.md.

CREATE TABLE IF NOT EXISTS compliance_policies (
    tenant_id TEXT PRIMARY KEY,
    cdr_retention_days INTEGER,
    sip_trace_retention_days INTEGER,
    flow_trace_retention_days INTEGER,
    recording_retention_days INTEGER,
    audit_retention_days INTEGER,
    recording_consent TEXT NOT NULL DEFAULT 'none',
    recording_consent_prompt_id TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS admin_audit_log (
    id BIGSERIAL PRIMARY KEY,
    tenant_id TEXT,
    actor TEXT NOT NULL,
    actor_kind TEXT NOT NULL,
    method TEXT NOT NULL,
    path TEXT NOT NULL,
    status INTEGER NOT NULL,
    action TEXT NOT NULL,
    detail JSONB,
    source_ip TEXT,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_admin_audit_tenant_time
    ON admin_audit_log (tenant_id, occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_admin_audit_actor_time
    ON admin_audit_log (actor, occurred_at DESC);

CREATE TABLE IF NOT EXISTS retention_runs (
    id BIGSERIAL PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    ran_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    cdrs_deleted BIGINT NOT NULL DEFAULT 0,
    sip_traces_deleted BIGINT NOT NULL DEFAULT 0,
    flow_traces_deleted BIGINT NOT NULL DEFAULT 0,
    recordings_deleted BIGINT NOT NULL DEFAULT 0,
    audit_events_deleted BIGINT NOT NULL DEFAULT 0,
    errors TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[]
);

CREATE INDEX IF NOT EXISTS idx_retention_runs_tenant_time
    ON retention_runs (tenant_id, ran_at DESC);
