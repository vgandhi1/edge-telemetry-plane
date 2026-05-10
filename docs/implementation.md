# DETCP — Implementation plan

This document outlines the **phased execution strategy** for the Distributed Edge Telemetry & Control Plane (DETCP). The approach is structured to mitigate risk early, establish strong contracts, and iteratively build toward a resilient, observable system.

**Related:** [Repository README](../README.md) · [Architecture](architecture.md)

---

## Executive summary

| Phase | Theme | Horizon (indicative) | Repository status |
| :--- | :--- | :--- | :--- |
| **1** | Foundation & contracts | Weeks 1–2 | **Largely done:** proto, Compose stack (minus OTel bundle), no CI yet |
| **2** | Resilient edge | Weeks 3–5 | **Largely done:** gateway + sync + JetStream; **gap:** formal offline unit tests |
| **3** | Cloud control plane | Weeks 6–8 | **Largely done:** Go ingress, Kafka, processor, hypertable; **gap:** mTLS, auth tokens |
| **4** | Observability | Weeks 9–10 | **Planned:** OTel Collector, Prometheus, Jaeger, Grafana; trace in payload today, not full propagation |
| **5** | Chaos & hardening | Weeks 11–12 | **Planned:** toxiproxy/tc, load tests, runbooks/ADRs |

---

## Phase 1: Foundation & contracts (weeks 1–2)

**Goal:** Establish infrastructure, CI/CD, and data contracts before (or in parallel with) application hardening.

### 1.1 Data contracts

- Define **Protocol Buffer** (`.proto`) schemas for telemetry points, batches, and the **gRPC** `EdgeSyncService`.
- Version package as `detcp.v1`; keep fields backward-compatible for rolling upgrades.
- **Canonical file:** [`proto/detcp/v1/telemetry.proto`](../proto/detcp/v1/telemetry.proto).

### 1.2 Local infrastructure

- **`docker-compose.yml`** for local development: Mosquitto, NATS JetStream, Kafka, TimescaleDB, and application services.
- **Reference:** [`deploy/docker-compose.yml`](../deploy/docker-compose.yml) (dev uses Kafka **replication factor 1**; production target is RF ≥ 3).

### 1.3 Observability skeleton

- Deploy **OpenTelemetry Collector**, **Prometheus**, **Jaeger**, and **Grafana** alongside the data plane (compose profile or second compose file).
- **Status:** Not yet in repo; see Phase 4.

### 1.4 CI/CD setup

- **GitHub Actions:** Rust (`fmt`, `clippy`, `test`, `cargo build`), Go (`fmt`, `vet`, `test`), Protobuf generation check, Docker image build/push.
- **Status:** Not yet initialized; recommended before declaring Phase 1 “complete” for production readiness.

**Phase 1 exit criteria:** `docker compose up` healthy; generated Rust/Go compiles from a single proto source; CI green on main.

---

## Phase 2: The resilient edge layer (weeks 3–5)

**Goal:** Rust edge components that survive **network partitions** and restarts without losing in-flight telemetry.

### 2.1 MQTT ingestion

- **Edge gateway** (`rumqttc`, `tokio`): subscribe to device topics; validate JSON; map to Protobuf.
- **As-built:** [`edge`](../edge/) — binary `edge-gateway`.

### 2.2 NATS integration

- **`async-nats`** + JetStream: durable stream (file-backed) as local buffer; retention policies to cap disk during long outages.
- **As-built:** stream `TELEMETRY`, subjects `telemetry.>`.

### 2.3 Edge sync worker

- Pull consumer; batch points; **tonic** gRPC client to cloud ingress; exponential backoff on transient failures.
- **As-built:** binary `edge-sync`.

### 2.4 Offline testing

- **Unit / integration tests:** with sync worker stopped or failing, prove messages accumulate in JetStream and are **acked only after** successful gRPC delivery.
- **Status:** recommended next step for Phase 2 sign-off.

---

## Phase 3: Cloud control plane (weeks 6–8)

**Goal:** Concurrent cloud path: ingest → Kafka → durable time-series store.

### 3.1 gRPC ingress

- Terminate **mTLS** (production); validate edge identity (JWT or mTLS SAN); `SyncTelemetry` handler.
- **As-built:** Go ingress in [`cloud-plane/cmd/ingress`](../cloud-plane/cmd/ingress/main.go); **dev:** plaintext gRPC on Docker network.

### 3.2 Kafka producer

- Partition by `edge_node_id` (or hash of key) for per-edge ordering.
- **As-built:** `segmentio/kafka-go` writer, topic `detcp.telemetry.batches`.

### 3.3 Stream processor

- Consumer group; deserialize `TelemetryBatch`; write to TimescaleDB.
- **As-built:** [`cloud-plane/cmd/processor`](../cloud-plane/cmd/processor/main.go).

### 3.4 Database schema

- Hypertable on `time`; indexes for edge + device query patterns; optional enrichment joins (factory metadata) later.
- **As-built:** [`deploy/sql/init.sql`](../deploy/sql/init.sql).

**Phase 3 exit criteria:** Sustained ingest with bounded consumer lag; documented connection strings and secrets policy (no secrets in git).

---

## Phase 4: Pervasive observability (weeks 9–10)

**Goal:** End-to-end **distributed tracing** and **metrics**.

### 4.1 Tracing context propagation

- Trace ID in **MQTT** user properties or payload; **NATS** optional metadata; **gRPC** metadata; **Kafka** record headers; processor emits spans.
- **As-built:** `TelemetryPoint.trace_id` in payload; propagation through gRPC headers and Kafka headers **not yet** implemented in services.

### 4.2 Metric instrumentation

- Prometheus: NATS depth, gRPC latency, Kafka lag, edge memory/CPU.
- **Status:** planned.

### 4.3 Dashboards

- Grafana: “Fleet health”, “system latency”, Kafka/NATS panels.
- **Status:** planned.

---

## Phase 5: Chaos engineering & hardening (weeks 11–12)

**Goal:** Prove behavior under adverse factory and WAN conditions.

### 5.1 Network partition simulation

- **Toxiproxy** or **Linux tc**: latency, loss, partition between edge and cloud; verify NATS buffering and gRPC recovery.

### 5.2 Load testing

- Order-of-magnitude target (e.g. **10k** logical devices at **10 Hz**): measure ingress, Kafka, and DB; define backpressure signals (gRPC `UNAVAILABLE`, consumer lag).

### 5.3 Documentation

- Runbooks, deployment guides, **ADRs** for major choices (mTLS, retention, partitioning).

---

## Repository layout (current)

```
deploy/              # docker-compose.yml, Mosquitto config, SQL init
proto/               # detcp.v1 contracts
edge/                # edge-gateway, edge-sync (Rust)
cloud-plane/         # ingress, processor (Go)
scripts/             # simulate_robot_fleet.py, run_dev_check.py
```

See the root [README.md](../README.md) for the **end-to-end quick start**.
