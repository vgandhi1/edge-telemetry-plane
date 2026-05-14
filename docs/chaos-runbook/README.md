# DETCP Chaos Engineering Runbook

## Zero Data Loss Proof — Step-by-Step Execution Guide

> **Stack:** Rust · Go · NATS JetStream · gRPC · Toxiproxy · Prometheus · Grafana  
> **Claim:** The edge-to-cloud pipeline survives a factory-floor network partition without dropping a single telemetry point.

---

## Prerequisites

```bash
# 1. Start the full stack (from repo root)
make up

# 2. Open Grafana: http://localhost:3000  (admin / admin)
#    Dashboard: DETCP Fault Tolerance (auto-provisioned)

# 3. Activate the Python environment
python3 -m venv .venv && source .venv/bin/activate
pip install -r scripts/requirements.txt
```

Verify all containers are healthy:
```bash
docker compose -f deploy/docker-compose.yml ps
```

Expected: all services in `running` or `healthy` state, including `toxiproxy`, `prometheus`, and `grafana`.

---

## Step 1 — Steady State Baseline

Start a continuous robot simulator stream (5 msg/sec across 5 virtual robots):
```bash
python3 scripts/simulate_robot_fleet.py \
  --count 5 --interval 1.0 --trace
```

Wait ~30 seconds, then verify end-to-end flow:

```bash
# Confirm DB is receiving messages
docker compose -f deploy/docker-compose.yml exec timescaledb \
  psql -U detcp -d detcp \
  -c "SELECT COUNT(*), MAX(sequence_id) FROM telemetry_points;"

# Confirm NATS buffer is empty (messages consumed immediately)
docker compose -f deploy/docker-compose.yml exec nats \
  nats stream info TELEMETRY --server nats://localhost:4222
```

**Grafana — Screenshot 1** (`docs/chaos-runbook/01_steady_state.png`):
- `gRPC Send Rate` → stable ~5 msgs/sec
- `NATS Edge Buffer Depth` → 0 or near-zero
- `Cloud Ingress Rate` → stable ~5 msgs/sec
- `Total gRPC Retry Attempts` → 0

---

## Step 2 — Inject the Fault (Network Partition)

Simulate a full Wi-Fi outage between the edge and cloud:

```bash
docker compose -f deploy/docker-compose.yml exec toxiproxy \
  toxiproxy-cli toxic add \
    --type timeout \
    --toxicName timeout_downstream \
    --attribute timeout=0 \
    cloud_ingress_grpc
```

Wait **30 seconds** while the simulator keeps publishing.

**Grafana — Screenshot 2** (`docs/chaos-runbook/02_outage_triggered.png`):
- `gRPC Send Rate` → **drops to 0**
- `NATS Edge Buffer Depth` → **climbing** (buffered messages)
- `Cloud Ingress Rate` → **drops to 0**
- `Total gRPC Retry Attempts` → **incrementing** (Rust backoff active)

---

## Step 3 — Observe Resilience During Outage

```bash
# Buffer filling — messages are durable on disk via FileStorage
docker compose -f deploy/docker-compose.yml exec nats \
  nats stream info TELEMETRY --server nats://localhost:4222
```

Expected excerpt:
```
State:
  Messages: 127        ← buffered, not yet synced to cloud
  Bytes:    48 KiB
```

```bash
# DB count NOT advancing — data held safely at edge
docker compose -f deploy/docker-compose.yml exec timescaledb \
  psql -U detcp -d detcp -c "SELECT COUNT(*) FROM telemetry_points;"
```

**Grafana — Screenshot 3** (`docs/chaos-runbook/03_buffer_filling.png`):  
NATS buffer depth panel showing a steady climb.

---

## Step 4 — Restore Connectivity

```bash
docker compose -f deploy/docker-compose.yml exec toxiproxy \
  toxiproxy-cli toxic remove \
    --toxicName timeout_downstream \
    cloud_ingress_grpc
```

Watch Grafana for the next 60 seconds.

**Grafana — Screenshot 4** (`docs/chaos-runbook/04_recovery_flush.png`):
- `Cloud Ingress Rate` → **spikes above baseline** (batch flush)
- `NATS Edge Buffer Depth` → **draining rapidly to 0**
- `gRPC Send Rate` → **restored to baseline**

---

## Step 5 — Zero Data Loss Verification

```bash
docker compose -f deploy/docker-compose.yml exec timescaledb \
  psql -U detcp -d detcp -c "
    SELECT
      COUNT(*)                                                AS total_rows,
      MIN(sequence_id)                                        AS first_seq,
      MAX(sequence_id)                                        AS last_seq,
      MAX(sequence_id) - MIN(sequence_id) + 1                AS expected_count,
      COUNT(*) = MAX(sequence_id) - MIN(sequence_id) + 1     AS zero_data_loss
    FROM telemetry_points;
  "
```

Expected output:
```
 total_rows | first_seq | last_seq | expected_count | zero_data_loss
------------+-----------+----------+----------------+----------------
       1000 |         1 |     1000 |           1000 | t
```

**Screenshot 5** (`docs/chaos-runbook/05_sql_zero_loss_proof.png`): SQL output with `zero_data_loss = t`.

---

## Step 6 — Confirm Buffer Fully Drained

```bash
docker compose -f deploy/docker-compose.yml exec nats \
  nats stream info TELEMETRY --server nats://localhost:4222
```

Expected: `Messages: 0`

**Screenshot 6** (`docs/chaos-runbook/06_buffer_empty.png`): NATS stream info showing `Messages: 0`.

---

## Available Toxiproxy Failure Scenarios

| Scenario | Command | Simulates |
|---|---|---|
| Full outage | `toxic add --type timeout --attribute timeout=0` | Wi-Fi cut / cloud unreachable |
| High latency | `toxic add --type latency --attribute latency=2000` | Degraded WAN link |
| Bandwidth cap | `toxic add --type bandwidth --attribute rate=10` | Throttled cellular backhaul |
| Half-open TCP | `toxic add --type slow_close --attribute delay=5000` | Stale TCP connections |

Remove any toxic (restore full connectivity):
```bash
docker compose -f deploy/docker-compose.yml exec toxiproxy \
  toxiproxy-cli toxic remove --toxicName <name> cloud_ingress_grpc
```

---

## Acceptance Criteria

| # | Criterion | Screenshot |
|---|---|---|
| 1 | Steady-state: all three Grafana panels show stable non-zero values | `01_steady_state.png` |
| 2 | Chaos injected: gRPC send rate and cloud ingress drop to 0 | `02_outage_triggered.png` |
| 3 | Buffer fills: NATS depth climbs during outage | `03_buffer_filling.png` |
| 4 | Recovery: buffer drains, cloud ingress rate spikes | `04_recovery_flush.png` |
| 5 | SQL: `zero_data_loss = t` in TimescaleDB query | `05_sql_zero_loss_proof.png` |
| 6 | NATS stream info: `Messages = 0` after full drain | `06_buffer_empty.png` |

---

## Key Design Decisions

| Decision | Rationale |
|---|---|
| NATS JetStream `FileStorage` | Survives process crash and power cycle; data persists to disk |
| `RetentionPolicy::WorkQueue` | Messages deleted only after explicit ACK — no premature deletion |
| Explicit ACK (`AckPolicy::Explicit`) | gRPC failure → NAK → JetStream redelivers; no data lost mid-flight |
| Exponential backoff (500ms → 30s) | Prevents tight retry loops during extended outages |
| Toxiproxy as WAN proxy | Network-layer failure injection without touching application code |
| `sequence_id` in every message | Enables gap-free SQL audit: `MAX(seq) - MIN(seq) + 1 = COUNT(*)` |
