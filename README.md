<div align="center">

# edge-telemetry-plane

### Distributed Edge Telemetry & Control Plane (DETCP)

**MQTT → Rust edge (NATS JetStream) → gRPC → Go cloud (Kafka → TimescaleDB)**  
*A runnable reference stack for factory and fleet telemetry.*

[![Repo](https://img.shields.io/badge/GitHub-vgandhi1%2Fedge--telemetry--plane-181717?style=flat-square&logo=github)](https://github.com/vgandhi1/edge-telemetry-plane)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow?style=flat-square)](LICENSE)

[![Rust](https://img.shields.io/badge/edge-Rust-orange?style=flat-square&logo=rust)](https://www.rust-lang.org/)
[![Go](https://img.shields.io/badge/cloud-Go-00ADD8?style=flat-square&logo=go)](https://go.dev/)
[![Kafka](https://img.shields.io/badge/stream-Kafka-231F20?style=flat-square&logo=apachekafka)](https://kafka.apache.org/)
[![NATS](https://img.shields.io/badge/buffer-NATS_JetStream-27AAEE?style=flat-square)](https://nats.io/)
[![TimescaleDB](https://img.shields.io/badge/DB-TimescaleDB-FDB515?style=flat-square)](https://www.timescale.com/)

[Architecture](docs/architecture.md) · [Implementation plan](docs/implementation.md) · [Docs index](docs/readme.md) · [GitHub profile hints](docs/github-repository-metadata.md)

</div>

---

## Why this repo

**edge-telemetry-plane** implements **DETCP**: a **clear boundary** between noisy factory floors and cloud analytics—**durable edge buffering**, **typed batches (Protobuf)**, and a **simple path to scale** (Kafka + time-series DB).

| If you need… | This repo shows… |
| :--- | :--- |
| Survive WAN blips | JetStream file-backed queue before gRPC |
| One contract edge ↔ cloud | [`proto/detcp/v1/telemetry.proto`](proto/detcp/v1/telemetry.proto) |
| Run it locally | `make up` + Python simulator → rows in TimescaleDB |

> **OpenTelemetry** (Collector, Jaeger, Grafana) and **mTLS** on gRPC are **planned**; the ingestion hot path works in Docker Compose today. See [docs/implementation.md](docs/implementation.md).

---

## Quick start

**Stack:** Docker Compose v2 · Python 3 (simulator).

```bash
git clone https://github.com/vgandhi1/edge-telemetry-plane.git
cd edge-telemetry-plane

make up

python3 -m venv .venv && . .venv/bin/activate
pip install -r scripts/requirements.txt
python3 scripts/simulate_robot_fleet.py --count 10 --interval 0.5

docker compose -f deploy/docker-compose.yml exec timescaledb \
  psql -U detcp -d detcp -c "SELECT COUNT(*) FROM telemetry_points;"
```

**Tear down (removes volumes):** `make down`

**Ports:** `1883` MQTT · `4222` NATS · `50051` gRPC · `5432` Postgres/Timescale · `9092` Kafka  

**Dev DB password** (change for anything non-local): `detcp_dev_change_me`

---

## Architecture

```mermaid
flowchart LR
    subgraph Edge["Factory edge"]
        D[Devices] -->|MQTT JSON| GW[edge-gateway · Rust]
        GW --> NATS[(NATS JetStream)]
        NATS --> SYNC[edge-sync · Rust]
    end

    subgraph Cloud["Cloud plane"]
        SYNC -->|gRPC| ING[ingress · Go]
        ING --> K[(Kafka)]
        K --> PROC[processor · Go]
        PROC --> TS[(TimescaleDB)]
    end
```

Full design: [docs/architecture.md](docs/architecture.md)

---

## Repository layout

| Path | Purpose |
| :--- | :--- |
| [`proto/`](proto/) | `detcp.v1` Protobuf + gRPC `EdgeSyncService` |
| [`edge/`](edge/) | `edge-gateway`, `edge-sync` (Rust) |
| [`cloud-plane/`](cloud-plane/) | gRPC ingress + Kafka consumer → DB (Go) |
| [`deploy/`](deploy/) | Compose, Mosquitto config, Timescale init SQL |
| [`scripts/`](scripts/) | Fleet simulator, dev layout check |
| [`docs/`](docs/) | Architecture, phased plan, **GitHub About/topics copy-paste** |

---

## Makefile

| Command | Action |
| :--- | :--- |
| `make up` / `make down` | Start or stop the full stack |
| `make proto` | Regenerate Go code from `.proto` (needs `protoc` + plugins) |
| `make logs` | Tail edge + cloud service logs |
| `make smoke` | `scripts/run_dev_check.py` + hints |

---

## Local development (apps outside Docker)

1. Run infra via Compose or your own brokers/DB.
2. `make proto` after editing `.proto`.
3. `cd cloud-plane && KAFKA_BROKERS=localhost:9092 go run ./cmd/ingress`
4. `cd cloud-plane && DATABASE_URL=postgres://detcp:...@localhost:5432/detcp?sslmode=disable go run ./cmd/processor`
5. `cd edge` — run `edge-gateway` and `edge-sync` (see [Quick start](#quick-start) env vars in [README](README.md) history or `deploy/docker-compose.yml`).

---

## Contributing

Issues and PRs welcome. For **description, topics, and GitHub UI** suggestions, see [docs/github-repository-metadata.md](docs/github-repository-metadata.md).

---

## License

[MIT](LICENSE)
