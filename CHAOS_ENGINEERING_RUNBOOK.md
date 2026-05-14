# DETCP — Fault Tolerance Execution Document
## Distributed Edge Telemetry & Control Plane: Chaos Engineering Runbook

> **Project Type:** Robotics SWE / SRE · **Stack:** Rust · Go · NATS JetStream · gRPC · Toxiproxy · Prometheus · Grafana  
> **Repo:** [vgandhi1/edge-telemetry-plane](https://github.com/vgandhi1/edge-telemetry-plane)  
> **Strategic Purpose:** Prove the edge-to-cloud pipeline survives factory-floor network failures without losing a single telemetry point.

---

## Table of Contents

1. [Engineering Problem Statement](#1-engineering-problem-statement)
2. [Architecture Deep-Dive](#2-architecture-deep-dive)
3. [Phase 1 — Baseline Architecture Verification](#3-phase-1--baseline-architecture-verification)
4. [Phase 2 — Toxiproxy Chaos Engineering Setup](#4-phase-2--toxiproxy-chaos-engineering-setup)
5. [Phase 3 — Observability Stack](#5-phase-3--observability-stack)
6. [Phase 4 — Fault Tolerance Demonstration Runbook](#6-phase-4--fault-tolerance-demonstration-runbook)
7. [NATS JetStream Durability Configuration](#7-nats-jetstream-durability-configuration)
8. [Rust Edge-Sync: Resilient gRPC with Backoff](#8-rust-edge-sync-resilient-grpc-with-backoff)
9. [Prometheus Metrics Instrumentation](#9-prometheus-metrics-instrumentation)
10. [Docker Compose — Complete Fault-Tolerant Stack](#10-docker-compose--complete-fault-tolerant-stack)
11. [Acceptance Criteria — Zero Data Loss Proof](#11-acceptance-criteria--zero-data-loss-proof)

---

## 1. Engineering Problem Statement

### Context

Factory floors and vehicle fleets operate on unreliable networks. A robot generating 500 telemetry messages/second cannot afford to drop data during a 30-second Wi-Fi roam, a Kafka restart, or a cloud ingress deployment rollout.

### Failure Modes This Project Addresses

| Failure Scenario | Expected System Behavior | Proof Mechanism |
|---|---|---|
| Cloud ingress unreachable (network partition) | Edge buffer fills; no messages dropped | NATS buffer depth climbs in Grafana |
| gRPC connection timeout during sync | Rust sync worker retries with exponential backoff | Prometheus `edge_sync_retries_total` counter |
| Network restored after outage | Buffered messages flush in order; no duplicates | Cloud ingress counter catches up to edge-sent counter |
| Cloud ingress crashes and restarts | Edge continues buffering; resumes sync on reconnect | Buffer drains after Go process restart |

### What "Zero Data Loss" Means Here

Every MQTT message published by the robot simulator is assigned a monotonically increasing `sequence_id`. After a full chaos cycle (outage → recovery), the TimescaleDB row count must equal the total published count, and no sequence gaps should appear in the database. This is the verifiable contract.

---

## 2. Architecture Deep-Dive

### Component Responsibilities

```
┌─────────────────────────────────────────────────────────────┐
│  FACTORY EDGE (single device, e.g., NVIDIA Jetson)          │
│                                                             │
│  Robot MQTT Publisher                                       │
│    └─→ Mosquitto Broker (:1883)                             │
│          └─→ edge-gateway (Rust)                            │
│                └─→ NATS JetStream (:4222)  ← FileStorage    │
│                      └─→ edge-sync (Rust)                   │
│                            └─→ [Toxiproxy sits here]        │
└────────────────────────────┬────────────────────────────────┘
                             │ gRPC (:50051 via Toxiproxy :8474)
┌────────────────────────────▼────────────────────────────────┐
│  CLOUD PLANE                                                │
│                                                             │
│  cloud-ingress (Go) ← gRPC receiver                        │
│    └─→ Kafka (:9092)                                        │
│          └─→ cloud-processor (Go) ← consumer               │
│                └─→ TimescaleDB (:5432)                      │
└─────────────────────────────────────────────────────────────┘
                             │
┌────────────────────────────▼────────────────────────────────┐
│  OBSERVABILITY                                              │
│  Prometheus (:9090) ← scrapes edge-sync, cloud-ingress      │
│  Grafana (:3000) ← reads Prometheus                         │
└─────────────────────────────────────────────────────────────┘
```

### Why Each Technology Was Chosen

| Component | Choice | Reason |
|---|---|---|
| Edge buffer | NATS JetStream w/ FileStorage | Persists messages to disk; survives process crash and power cycle |
| Edge application | Rust | Zero-cost abstractions; deterministic memory; no GC pauses during bursts |
| Wire format | Protobuf over gRPC | Compact binary; typed contract between edge and cloud; streaming support |
| Chaos injection | Toxiproxy | Network-layer failure simulation without code changes; scriptable via CLI |
| Observability | Prometheus + Grafana | Standard pull-based metrics; production-equivalent setup |
| Time-series store | TimescaleDB | PostgreSQL extension; hypertables for efficient range queries on telemetry |

---

## 3. Phase 1 — Baseline Architecture Verification

Before running chaos tests, you must verify the baseline happy-path end-to-end.

### 3.1 Start the Full Stack

```bash
git clone https://github.com/vgandhi1/edge-telemetry-plane.git
cd edge-telemetry-plane
make up
```

Verify all containers are healthy:
```bash
docker compose -f deploy/docker-compose.yml ps
```

Expected output — all services in `running` or `healthy` state:
```
NAME                    STATUS          PORTS
mosquitto               running         0.0.0.0:1883->1883/tcp
nats                    running         0.0.0.0:4222->4222/tcp
edge-gateway            running
edge-sync               running
cloud-ingress           running         0.0.0.0:50051->50051/tcp
kafka                   running         0.0.0.0:9092->9092/tcp
cloud-processor         running
timescaledb             healthy         0.0.0.0:5432->5432/tcp
toxiproxy               running         0.0.0.0:8474->8474/tcp, 0.0.0.0:8475->8475/tcp
prometheus              running         0.0.0.0:9090->9090/tcp
grafana                 running         0.0.0.0:3000->3000/tcp
```

### 3.2 Run the Robot Fleet Simulator

```bash
python3 -m venv .venv && source .venv/bin/activate
pip install -r scripts/requirements.txt

# Publish 200 messages at 0.5s intervals (100 seconds of data)
python3 scripts/simulate_robot_fleet.py --count 200 --interval 0.5
```

### 3.3 Verify End-to-End Delivery

```bash
# Count rows in TimescaleDB
docker compose -f deploy/docker-compose.yml exec timescaledb \
  psql -U detcp -d detcp -c "SELECT COUNT(*) FROM telemetry_points;"
```

Expected: `count = 200` (all messages delivered, no drops).

```bash
# Check for sequence gaps (should return 0 rows if zero data loss)
docker compose -f deploy/docker-compose.yml exec timescaledb \
  psql -U detcp -d detcp -c "
    SELECT sequence_id, lag(sequence_id) OVER (ORDER BY sequence_id) as prev_id
    FROM telemetry_points
    HAVING sequence_id - lag(sequence_id) OVER (ORDER BY sequence_id) > 1;
  "
```

### 3.4 Baseline Screenshot Checklist

Before proceeding to chaos testing, capture:
- [ ] `docker compose ps` output showing all containers healthy
- [ ] TimescaleDB `COUNT(*)` = simulator published count
- [ ] Grafana steady-state dashboard (see Phase 3 for setup)

---

## 4. Phase 2 — Toxiproxy Chaos Engineering Setup

### 4.1 What is Toxiproxy?

[Toxiproxy](https://github.com/Shopify/toxiproxy) is a network failure simulator developed by Shopify. It acts as a transparent TCP proxy that you can inject latency, packet loss, connection resets, and timeouts into — without modifying application code. Toxiproxy runs as a sidecar container and is controlled via its CLI or HTTP API.

### 4.2 Toxiproxy Placement in the Architecture

Toxiproxy sits **between the Rust edge-sync worker and the Go cloud-ingress service**. This is the WAN link — the exact point where factory Wi-Fi outages and cloud connectivity failures manifest.

```
edge-sync (Rust)  →  toxiproxy:8474  →  cloud-ingress:50051
```

The edge-sync worker is configured to connect to `toxiproxy:8474` (not directly to `cloud-ingress:50051`).

### 4.3 Toxiproxy Docker Compose Service

```yaml
# In deploy/docker-compose.yml
toxiproxy:
  image: ghcr.io/shopify/toxiproxy:2.9.0
  ports:
    - "8474:8474"   # Toxiproxy API port
    - "8475:8475"   # Proxied gRPC port (edge-sync connects here)
  networks:
    - detcp-net
  healthcheck:
    test: ["CMD", "wget", "-q", "--spider", "http://localhost:8474/proxies"]
    interval: 10s
    timeout: 5s
    retries: 5
```

### 4.4 Initialize the Proxy (Run Once After Stack Start)

```bash
# Create the proxy: edge connections on :8474 forward to cloud-ingress:50051
docker compose -f deploy/docker-compose.yml exec toxiproxy \
  toxiproxy-cli create --listen 0.0.0.0:8475 --upstream cloud-ingress:50051 cloud_ingress_grpc
```

Verify the proxy exists:
```bash
docker compose -f deploy/docker-compose.yml exec toxiproxy \
  toxiproxy-cli list
```

### 4.5 Edge-Sync Environment Variable

Update the edge-sync service environment in `docker-compose.yml` to route through Toxiproxy:

```yaml
edge-sync:
  environment:
    - INGRESS_ADDR=toxiproxy:8475   # Route through Toxiproxy
```

### 4.6 Available Toxics (Failure Types)

| Toxic | Command | Use Case |
|---|---|---|
| **Timeout** (full outage) | `toxiproxy-cli toxic add -t timeout -a timeout=0 cloud_ingress_grpc` | Simulates Wi-Fi cut or cloud unreachable |
| **Latency** | `toxiproxy-cli toxic add -t latency -a latency=500 cloud_ingress_grpc` | Simulates degraded WAN link |
| **Slow close** | `toxiproxy-cli toxic add -t slow_close -a delay=5000 cloud_ingress_grpc` | Simulates half-open TCP connections |
| **Bandwidth limit** | `toxiproxy-cli toxic add -t bandwidth -a rate=10 cloud_ingress_grpc` | Simulates throttled cellular backhaul |

**Remove all toxics (restore connectivity):**
```bash
docker compose -f deploy/docker-compose.yml exec toxiproxy \
  toxiproxy-cli toxic remove --toxicName timeout_downstream cloud_ingress_grpc
```

---

## 5. Phase 3 — Observability Stack

### 5.1 Prometheus Configuration

**File:** `deploy/prometheus/prometheus.yml`

```yaml
global:
  scrape_interval: 5s
  evaluation_interval: 5s

scrape_configs:
  - job_name: 'edge-sync'
    static_configs:
      - targets: ['edge-sync:9100']
    metrics_path: '/metrics'

  - job_name: 'cloud-ingress'
    static_configs:
      - targets: ['cloud-ingress:9101']
    metrics_path: '/metrics'

  - job_name: 'nats'
    static_configs:
      - targets: ['nats:7777']
    metrics_path: '/metrics'
```

### 5.2 Grafana Dashboard — The Three Required Panels

Access Grafana at `http://localhost:3000` (default credentials: `admin` / `admin`).

Add Prometheus as data source: `http://prometheus:9090`

#### Panel 1 — Network Status (gRPC Health)

- **Type:** Time series
- **Query:** `rate(edge_sync_grpc_send_total{status="success"}[30s])`
- **Title:** `gRPC Send Rate (msgs/sec) — Edge → Cloud`
- **Behavior:** Shows positive rate during steady state; drops to 0 during Toxiproxy outage

#### Panel 2 — NATS Edge Buffer Depth

- **Type:** Time series  
- **Query:** `nats_jetstream_stream_messages{stream="TELEMETRY"}`
- **Title:** `NATS Edge Buffer Depth (un-synced messages)`
- **Behavior:** Stays near 0 in steady state; climbs during outage; drains rapidly on recovery

#### Panel 3 — Cloud Ingress Rate

- **Type:** Time series
- **Query:** `rate(cloud_ingress_messages_received_total[30s])`
- **Title:** `Cloud Ingress Rate (msgs/sec)`
- **Behavior:** Tracks gRPC send rate in steady state; drops to 0 during outage; spikes during buffer flush

#### Bonus Panel — Sync Retry Count

- **Type:** Stat
- **Query:** `edge_sync_retries_total`
- **Title:** `Total gRPC Retry Attempts`
- **Behavior:** Increments during outage as Rust worker retries failed gRPC calls

### 5.3 Grafana Dashboard JSON Export

After building the dashboard manually, export it as JSON (`Dashboard → Share → Export`) and save it to `deploy/grafana/dashboards/detcp_fault_tolerance.json` so the Grafana container can provision it automatically.

**Grafana provisioning config (`deploy/grafana/provisioning/dashboards/default.yaml`):**
```yaml
apiVersion: 1
providers:
  - name: 'DETCP'
    folder: 'DETCP'
    type: file
    options:
      path: /etc/grafana/dashboards
```

---

## 6. Phase 4 — Fault Tolerance Demonstration Runbook

This is the canonical interview demonstration. Execute exactly in this sequence and capture screenshots at each numbered step.

### Prerequisites

- Full stack running (`make up`)
- Toxiproxy proxy initialized (Section 4.4)
- Grafana dashboard open at `http://localhost:3000`
- Robot simulator running: `python3 scripts/simulate_robot_fleet.py --count 2000 --interval 0.2` (continuous stream at 5 msg/sec)

---

### Step 1 — Steady State Baseline

**What you're showing:** The pipeline is working. Data flows end-to-end. Edge buffer is empty.

```bash
# Confirm zero buffer depth
docker compose -f deploy/docker-compose.yml exec nats \
  nats stream info TELEMETRY --server nats://localhost:4222

# Confirm messages arriving in DB
docker compose -f deploy/docker-compose.yml exec timescaledb \
  psql -U detcp -d detcp -c "SELECT COUNT(*) FROM telemetry_points;"
```

📸 **Screenshot 1:** Grafana showing:
- gRPC Send Rate: stable ~5 msgs/sec
- NATS Buffer Depth: 0 or near-zero
- Cloud Ingress Rate: stable ~5 msgs/sec

---

### Step 2 — Inject the Fault (Network Outage)

**What you're showing:** Simulating a factory Wi-Fi outage or cloud ingress failure.

```bash
docker compose -f deploy/docker-compose.yml exec toxiproxy \
  toxiproxy-cli toxic add \
    --type timeout \
    --toxicName timeout_downstream \
    --attribute timeout=0 \
    cloud_ingress_grpc
```

Wait 30 seconds.

📸 **Screenshot 2:** Grafana showing:
- gRPC Send Rate: **drops to 0**
- **NATS Buffer Depth: rising** (e.g., 50, 100, 150 messages)
- Cloud Ingress Rate: **drops to 0**
- *(The robot simulator is still publishing; data is being durably buffered)*

---

### Step 3 — Observe Resilience During Outage

```bash
# Show buffer is filling with un-synced messages
docker compose -f deploy/docker-compose.yml exec nats \
  nats stream info TELEMETRY --server nats://localhost:4222
```

Expected output excerpt:
```
State:
  Messages: 127        ← buffered messages
  Bytes:    48 KiB
  First Seq: 201
  Last Seq: 327
```

```bash
# Confirm DB count is NOT advancing (data is being held at edge)
docker compose -f deploy/docker-compose.yml exec timescaledb \
  psql -U detcp -d detcp -c "SELECT COUNT(*) FROM telemetry_points;"
```

📸 **Screenshot 3:** NATS stream info showing growing `Messages` count.

---

### Step 4 — Restore Connectivity

**What you're showing:** Network recovers; edge flushes buffered data immediately.

```bash
docker compose -f deploy/docker-compose.yml exec toxiproxy \
  toxiproxy-cli toxic remove \
    --toxicName timeout_downstream \
    cloud_ingress_grpc
```

Watch the Grafana dashboard for the next 60 seconds.

📸 **Screenshot 4:** Grafana showing:
- Cloud Ingress Rate: **spikes above baseline** (batch flush)
- NATS Buffer Depth: **draining rapidly to 0**
- gRPC Send Rate: **restored to baseline**

---

### Step 5 — Zero Data Loss Verification

```bash
# Total messages published by simulator (check simulator output)
# Expected: ~N messages published

# Verify DB count equals published count
docker compose -f deploy/docker-compose.yml exec timescaledb \
  psql -U detcp -d detcp -c "
    SELECT
      COUNT(*) as total_rows,
      MIN(sequence_id) as first_seq,
      MAX(sequence_id) as last_seq,
      MAX(sequence_id) - MIN(sequence_id) + 1 as expected_count,
      COUNT(*) = MAX(sequence_id) - MIN(sequence_id) + 1 as zero_data_loss
    FROM telemetry_points;
  "
```

Expected output:
```
 total_rows | first_seq | last_seq | expected_count | zero_data_loss
------------+-----------+----------+----------------+----------------
       2000 |         1 |     2000 |           2000 | t
```

📸 **Screenshot 5:** SQL output with `zero_data_loss = t`.

---

## 7. NATS JetStream Durability Configuration

The edge buffer only survives power cycles and process crashes if JetStream is configured with `FileStorage` (not the default `MemoryStorage`).

**File:** `deploy/nats/jetstream.conf`

```
jetstream {
  store_dir = "/data/jetstream"
}

server_name = "edge-node-001"

accounts {
  $G {
    jetstream = enabled
  }
}
```

**Stream creation on startup** (run by edge-gateway on first boot):

```rust
// In edge-gateway Rust code — stream initialization
use async_nats::jetstream;

async fn ensure_stream(js: &jetstream::Context) -> Result<()> {
    let stream_config = jetstream::stream::Config {
        name: "TELEMETRY".to_string(),
        subjects: vec!["telemetry.>".to_string()],
        storage: jetstream::stream::StorageType::File,  // ← Critical: persists to disk
        max_messages: 1_000_000,
        max_bytes: 512 * 1024 * 1024,  // 512MB max buffer
        retention: jetstream::stream::RetentionPolicy::WorkQueue,
        ..Default::default()
    };

    match js.get_or_create_stream(stream_config).await {
        Ok(_) => {
            tracing::info!("NATS JetStream TELEMETRY stream ready (FileStorage)");
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}
```

**Why `RetentionPolicy::WorkQueue`?**
Messages are deleted from the stream after the edge-sync worker acknowledges successful gRPC delivery. This prevents unbounded disk growth while ensuring messages are never deleted before they are confirmed delivered.

---

## 8. Rust Edge-Sync: Resilient gRPC with Backoff

The edge-sync worker must not crash or spin-loop during outages. It must retry with exponential backoff and resume cleanly when connectivity is restored.

**Key implementation pattern** (`edge/src/bin/edge-sync/main.rs`):

```rust
use tokio_retry::{strategy::ExponentialBackoff, Retry};
use std::time::Duration;

async fn sync_with_retry(
    js_consumer: &mut Consumer<Config>,
    grpc_addr: &str,
) -> Result<()> {
    let retry_strategy = ExponentialBackoff::from_millis(500)
        .factor(2)
        .max_delay(Duration::from_secs(30))
        .take(20);  // Max 20 retries before logging error and continuing

    Retry::spawn(retry_strategy, || async {
        match send_batch_grpc(js_consumer, grpc_addr).await {
            Ok(n) => {
                metrics::counter!("edge_sync_grpc_send_total", "status" => "success")
                    .increment(n as u64);
                Ok(())
            }
            Err(e) => {
                tracing::warn!("gRPC send failed: {}. Retrying...", e);
                metrics::counter!("edge_sync_retries_total").increment(1);
                Err(e)
            }
        }
    }).await
}
```

**Critical design note:** The NATS consumer uses `AckPolicy::Explicit`. The Rust worker only sends `ack()` to JetStream after the gRPC call returns `Ok`. If the process crashes mid-send, NATS will redeliver unacknowledged messages to the next consumer instance. This provides **at-least-once delivery** semantics.

---

## 9. Prometheus Metrics Instrumentation

### Edge-Sync Metrics (Rust — using `metrics` crate)

```toml
# Cargo.toml
[dependencies]
metrics = "0.22"
metrics-exporter-prometheus = "0.13"
```

```rust
// In edge-sync main.rs — initialize Prometheus exporter
use metrics_exporter_prometheus::PrometheusBuilder;

PrometheusBuilder::new()
    .with_http_listener(([0, 0, 0, 0], 9100))
    .install()?;

// Key counters/gauges used throughout the application:
// edge_sync_grpc_send_total{status="success"|"error"}
// edge_sync_retries_total
// edge_sync_nats_buffer_depth (gauge, polled from NATS API)
```

### Cloud Ingress Metrics (Go — using `prometheus/client_golang`)

```go
// In cloud-plane/cmd/ingress/main.go
import (
    "github.com/prometheus/client_golang/prometheus"
    "github.com/prometheus/client_golang/prometheus/promauto"
    "github.com/prometheus/client_golang/prometheus/promhttp"
)

var (
    messagesReceived = promauto.NewCounter(prometheus.CounterOpts{
        Name: "cloud_ingress_messages_received_total",
        Help: "Total gRPC messages received from edge-sync workers",
    })
    kafkaPublishErrors = promauto.NewCounter(prometheus.CounterOpts{
        Name: "cloud_ingress_kafka_publish_errors_total",
        Help: "Failed Kafka publish attempts",
    })
)

// Register HTTP metrics endpoint
http.Handle("/metrics", promhttp.Handler())
go http.ListenAndServe(":9101", nil)
```

---

## 10. Docker Compose — Complete Fault-Tolerant Stack

**File:** `deploy/docker-compose.yml` (additions/modifications highlighted)

```yaml
version: '3.9'

networks:
  detcp-net:
    driver: bridge

volumes:
  nats-data:
  timescale-data:
  kafka-data:
  grafana-data:

services:

  mosquitto:
    image: eclipse-mosquitto:2.0
    ports: ["1883:1883"]
    volumes:
      - ./mosquitto/mosquitto.conf:/mosquitto/config/mosquitto.conf
    networks: [detcp-net]

  nats:
    image: nats:2.10-alpine
    command: ["-c", "/etc/nats/jetstream.conf"]
    ports: ["4222:4222", "8222:8222", "7777:7777"]
    volumes:
      - nats-data:/data/jetstream
      - ./nats/jetstream.conf:/etc/nats/jetstream.conf
    networks: [detcp-net]

  edge-gateway:
    build: ../edge
    command: ["edge-gateway"]
    environment:
      - MQTT_URL=mqtt://mosquitto:1883
      - NATS_URL=nats://nats:4222
      - EDGE_NODE_ID=factory-floor-001
      - RUST_LOG=info
    depends_on: [mosquitto, nats]
    networks: [detcp-net]

  edge-sync:
    build: ../edge
    command: ["edge-sync"]
    environment:
      - NATS_URL=nats://nats:4222
      - INGRESS_ADDR=toxiproxy:8475   # ← Route through Toxiproxy
      - EDGE_NODE_ID=factory-floor-001
      - RUST_LOG=info
      - METRICS_PORT=9100
    ports: ["9100:9100"]
    depends_on: [nats, toxiproxy]
    networks: [detcp-net]

  toxiproxy:
    image: ghcr.io/shopify/toxiproxy:2.9.0
    ports:
      - "8474:8474"   # Toxiproxy API
      - "8475:8475"   # Proxied gRPC
    networks: [detcp-net]
    healthcheck:
      test: ["CMD", "wget", "-q", "--spider", "http://localhost:8474/proxies"]
      interval: 10s
      timeout: 5s
      retries: 5

  toxiproxy-init:
    image: ghcr.io/shopify/toxiproxy:2.9.0
    entrypoint: >
      sh -c "sleep 5 &&
      toxiproxy-cli -h toxiproxy:8474 create
        --listen 0.0.0.0:8475
        --upstream cloud-ingress:50051
        cloud_ingress_grpc"
    depends_on:
      toxiproxy:
        condition: service_healthy
    networks: [detcp-net]

  cloud-ingress:
    build: ../cloud-plane
    command: ["ingress"]
    environment:
      - KAFKA_BROKERS=kafka:9092
      - GRPC_PORT=50051
      - METRICS_PORT=9101
    ports: ["50051:50051", "9101:9101"]
    depends_on: [kafka]
    networks: [detcp-net]

  kafka:
    image: bitnami/kafka:3.6
    environment:
      - KAFKA_CFG_NODE_ID=1
      - KAFKA_CFG_PROCESS_ROLES=broker,controller
      - KAFKA_CFG_LISTENERS=PLAINTEXT://:9092,CONTROLLER://:9093
      - KAFKA_CFG_ADVERTISED_LISTENERS=PLAINTEXT://kafka:9092
      - KAFKA_CFG_CONTROLLER_QUORUM_VOTERS=1@kafka:9093
      - KAFKA_CFG_CONTROLLER_LISTENER_NAMES=CONTROLLER
      - ALLOW_PLAINTEXT_LISTENER=yes
    volumes: [kafka-data:/bitnami/kafka]
    networks: [detcp-net]

  cloud-processor:
    build: ../cloud-plane
    command: ["processor"]
    environment:
      - KAFKA_BROKERS=kafka:9092
      - DATABASE_URL=postgres://detcp:detcp_dev_change_me@timescaledb:5432/detcp?sslmode=disable
    depends_on: [kafka, timescaledb]
    networks: [detcp-net]

  timescaledb:
    image: timescale/timescaledb:latest-pg15
    environment:
      - POSTGRES_USER=detcp
      - POSTGRES_PASSWORD=detcp_dev_change_me
      - POSTGRES_DB=detcp
    volumes:
      - timescale-data:/var/lib/postgresql/data
      - ./timescaledb/init.sql:/docker-entrypoint-initdb.d/init.sql
    ports: ["5432:5432"]
    networks: [detcp-net]
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U detcp"]
      interval: 10s
      timeout: 5s
      retries: 10

  prometheus:
    image: prom/prometheus:v2.50.1
    volumes:
      - ./prometheus/prometheus.yml:/etc/prometheus/prometheus.yml
    ports: ["9090:9090"]
    networks: [detcp-net]

  grafana:
    image: grafana/grafana:10.3.3
    environment:
      - GF_SECURITY_ADMIN_PASSWORD=admin
      - GF_USERS_ALLOW_SIGN_UP=false
    volumes:
      - grafana-data:/var/lib/grafana
      - ./grafana/provisioning:/etc/grafana/provisioning
      - ./grafana/dashboards:/etc/grafana/dashboards
    ports: ["3000:3000"]
    depends_on: [prometheus]
    networks: [detcp-net]
```

---

## 11. Acceptance Criteria — Zero Data Loss Proof

### Definition of Done

The chaos engineering demonstration is complete when all of the following are documented with screenshots in `docs/chaos-runbook/`:

| # | Criterion | Evidence |
|---|---|---|
| 1 | Steady-state: all three Grafana panels show stable non-zero values | Screenshot: `01_steady_state.png` |
| 2 | Chaos injected: gRPC send rate and cloud ingress drop to 0 | Screenshot: `02_outage_triggered.png` |
| 3 | Buffer fills: NATS depth climbs during outage | Screenshot: `03_buffer_filling.png` |
| 4 | Recovery: buffer drains, cloud ingress rate spikes | Screenshot: `04_recovery_flush.png` |
| 5 | SQL verification: `zero_data_loss = t` in TimescaleDB query | Screenshot: `05_sql_zero_loss_proof.png` |
| 6 | NATS stream info: `Messages = 0` after full drain | Screenshot: `06_buffer_empty.png` |

### Repository Documentation Structure

```
docs/
├── architecture.md          # System design
├── implementation.md        # Phased build plan
├── chaos-runbook/
│   ├── README.md            # This runbook (abbreviated)
│   ├── 01_steady_state.png
│   ├── 02_outage_triggered.png
│   ├── 03_buffer_filling.png
│   ├── 04_recovery_flush.png
│   ├── 05_sql_zero_loss_proof.png
│   └── 06_buffer_empty.png
└── github-repository-metadata.md
```

### Interview Talking Points This Demonstrates

1. **"How do you ensure data durability at the edge?"** → NATS JetStream FileStorage + explicit ACK pattern
2. **"How did you test failure scenarios?"** → Toxiproxy chaos injection with quantified recovery
3. **"What happens if the cloud is unreachable for 10 minutes?"** → Edge buffer absorbs the gap; zero message loss proven via sequence ID audit
4. **"How do you observe a distributed system?"** → Prometheus scraping both edge and cloud metrics; Grafana correlating buffer depth with ingress rate
5. **"What does your runbook look like?"** → This document — reproducible, step-by-step, screenshot-backed

---

*Document version 1.0 — May 2026*
