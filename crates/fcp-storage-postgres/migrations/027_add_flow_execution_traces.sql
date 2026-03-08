-- Flow execution traces: persistent record of every node hit per call.
CREATE TABLE IF NOT EXISTS flow_execution_traces (
    id          BIGSERIAL PRIMARY KEY,
    call_id     TEXT NOT NULL,
    flow_id     TEXT NOT NULL,
    flow_name   TEXT,
    node_id     TEXT,
    event_type  TEXT NOT NULL,         -- started, node_executed, waiting_for_input, completed, failed
    timestamp   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    detail      TEXT,                  -- outcome, error message, variable name, etc.
    flow_version INTEGER,
    elapsed_ms  BIGINT                -- milliseconds since flow started
);

-- Primary lookup: all events for a given call
CREATE INDEX idx_flow_traces_call_id ON flow_execution_traces (call_id);

-- Filter by flow definition
CREATE INDEX idx_flow_traces_flow_id ON flow_execution_traces (flow_id);

-- Time-range queries and retention cleanup
CREATE INDEX idx_flow_traces_timestamp ON flow_execution_traces (timestamp);

-- Composite for common query: call_id + timestamp ordering
CREATE INDEX idx_flow_traces_call_ts ON flow_execution_traces (call_id, timestamp ASC);
