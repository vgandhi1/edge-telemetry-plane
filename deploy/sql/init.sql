CREATE EXTENSION IF NOT EXISTS timescaledb;

CREATE TABLE IF NOT EXISTS telemetry_points (
    time TIMESTAMPTZ NOT NULL,
    edge_node_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    sensors JSONB NOT NULL DEFAULT '{}',
    trace_id TEXT NOT NULL DEFAULT '',
    sequence_id BIGINT NOT NULL DEFAULT 0
);

SELECT public.create_hypertable(
    'telemetry_points',
    'time',
    if_not_exists => TRUE
);

CREATE INDEX IF NOT EXISTS idx_telemetry_edge_time
    ON telemetry_points (edge_node_id, time DESC);

CREATE INDEX IF NOT EXISTS idx_telemetry_device_time
    ON telemetry_points (device_id, time DESC);
