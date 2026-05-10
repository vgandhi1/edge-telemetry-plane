# DETCP — Detailed architecture

This document describes the system design of the **Distributed Edge Telemetry & Control Plane (DETCP)**: component responsibilities, data contracts, failure mitigations, and observability. It matches the [implementation plan](implementation.md).

**Related:** [Repository README](../README.md) · [Implementation plan](implementation.md)

**Legend:** *Target* = production-grade intent; *As-built* = what the reference repo runs in dev Compose today.

---

## 1. System components & technologies

### 1.1 Edge tier (factory K3s / hardware)

| Component | Technology | Responsibility |
| :--- | :--- | :--- |
| **Mosquitto** | MQTT broker | Lightweight local broker for PLCs, robots, and simulators. |
| **Edge gateway** | **Rust:** `tokio`, `rumqttc`, `async-nats`, `prost` | Subscribe to device telemetry topics; **validate** payload shape; attach or preserve **trace** context; encode **Protobuf**; publish to JetStream. |
| **NATS JetStream** | File-backed stream | **Shock absorber** during WAN outages; retention limits disk growth; consumers resume from durable positions after reboot. |
| **Edge sync worker** | **Rust:** `tonic`, `async-nats` | Pull from JetStream; **batch** points into `TelemetryBatch`; **gRPC** to cloud ingress; **exponential backoff** on failure. *Target:* **mTLS** streams. *As-built:* plaintext gRPC on the dev Docker network. |

### 1.2 Cloud tier (Kubernetes)

| Component | Technology | Responsibility |
| :--- | :--- | :--- |
| **Cloud ingress API** | **Go** (`grpc-go`), Kafka client | Terminate **mTLS** (*target*); validate edge identity (*target:* JWT or mTLS); unpack batches; **produce** to Kafka. *As-built:* Go ingress, no mTLS in dev. |
| **Apache Kafka** | Distributed log | Ingest buffer; *target* **RF = 3** for HA; partition for ordering per edge or device class. *As-built:* single broker, RF = 1 in Compose. |
| **Telemetry processor** | **Go**, `pgx` | Consume batches; optional **enrichment** (e.g. join device → factory metadata); **batch insert** into TimescaleDB. *As-built:* direct insert per point from batch. |
| **TimescaleDB** | PostgreSQL + Timescale | Time-series store; **hypertables** partitioned by **time**; indexes for edge and device slices. |

---

## 2. Data contracts (Protocol Buffers)

Strict contracts allow schema evolution without breaking the pipeline (unknown fields ignored per Protobuf rules).

**Source of truth:** [`proto/detcp/v1/telemetry.proto`](../proto/detcp/v1/telemetry.proto)

```protobuf
syntax = "proto3";
package detcp.v1;

message TelemetryPoint {
  string device_id = 1;
  int64 timestamp_ms = 2;
  map<string, double> sensors = 3;
  string trace_id = 4;  // distributed tracing correlation
}

message TelemetryBatch {
  string edge_node_id = 1;
  repeated TelemetryPoint points = 2;
}

message SyncResponse {
  bool success = 1;
  int32 processed_count = 2;
}

service EdgeSyncService {
  rpc SyncTelemetry(TelemetryBatch) returns (SyncResponse);
}
```

| Concern | Approach |
| :--- | :--- |
| **Schema mismatch** | Protobuf backward compatibility: new fields ignored by old consumers until upgraded; avoid reusing field numbers. |
| **Ordering** | Kafka key = `edge_node_id` (or derived key) so one edge’s batches stay ordered on a single partition. |

---

## 3. Failure modes & mitigations

Design assumes **every component will eventually fail**.

| Failure mode | Detection | Mitigation |
| :--- | :--- | :--- |
| **Edge–cloud network drop** | gRPC timeout / `UNAVAILABLE` / `DEADLINE_EXCEEDED` | Sync worker backs off; gateway keeps accepting MQTT and appending to JetStream; replay when cloud is reachable. |
| **Edge node power loss** | Missing heartbeat in Prometheus (*target*) | JetStream file storage + consumer positions; after reboot, gateway and sync resume without re-sending acknowledged work. |
| **Kafka degraded** | Ingress produce timeout / errors | *Target:* return **UNAVAILABLE** to edge so NATS retains messages; sync retries with backoff (**backpressure**). *As-built:* align ingress errors with retriable gRPC status codes. |
| **Schema mismatch** | Deserialize errors at processor | Dead-letter or metric + skip; Protobuf ignores unknown fields on compatible readers. |

---

## 4. Observability & tracing context

**Target** end-to-end path for one telemetry point:

1. **MQTT:** Device publishes (payload and/or MQTT 5 user properties may carry correlation IDs).
2. **Edge gateway:** Generate or accept OpenTelemetry trace context; set **`TelemetryPoint.trace_id`** (and optionally duplicate in NATS headers *target*).
3. **gRPC:** Sync worker maps trace context into **gRPC metadata** (*target*).
4. **Kafka:** Ingress copies trace into **Kafka record headers** (*target*).
5. **Analysis:** All services export spans to **OTel Collector**; **Jaeger** rebuilds the waterfall from shared trace IDs.

**As-built:** `trace_id` is carried in the Protobuf payload from simulator/gateway; **OTel Collector, Jaeger, Grafana, and Kafka header propagation are not yet wired** in the compose stack.

**Metrics to watch (*target*):** JetStream depth, gRPC latency (histogram), Kafka consumer lag, ingress error rate, edge resource usage.
