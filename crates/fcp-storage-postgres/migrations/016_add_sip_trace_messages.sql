-- SIP message trace storage for ladder diagram visualization
CREATE TABLE IF NOT EXISTS sip_trace_messages (
    id              BIGSERIAL PRIMARY KEY,
    call_id         TEXT NOT NULL,
    sip_call_id     TEXT NOT NULL,
    tenant_id       TEXT NOT NULL DEFAULT '_system',
    timestamp       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    direction       TEXT NOT NULL,
    method_or_status TEXT NOT NULL,
    cseq            TEXT NOT NULL DEFAULT '',
    from_uri        TEXT NOT NULL DEFAULT '',
    to_uri          TEXT NOT NULL DEFAULT '',
    source_addr     TEXT NOT NULL DEFAULT '',
    dest_addr       TEXT NOT NULL DEFAULT '',
    transport       TEXT NOT NULL DEFAULT 'udp',
    raw_message     TEXT NOT NULL DEFAULT '',
    node_id         TEXT
);

CREATE INDEX IF NOT EXISTS idx_sip_trace_call_id ON sip_trace_messages(call_id);
CREATE INDEX IF NOT EXISTS idx_sip_trace_sip_call_id ON sip_trace_messages(sip_call_id);
CREATE INDEX IF NOT EXISTS idx_sip_trace_timestamp ON sip_trace_messages(timestamp);
CREATE INDEX IF NOT EXISTS idx_sip_trace_tenant ON sip_trace_messages(tenant_id);
