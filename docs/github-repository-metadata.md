# GitHub repository profile (copy-paste)

Use these on [github.com/vgandhi1/edge-telemetry-plane](https://github.com/vgandhi1/edge-telemetry-plane): **Settings → General** (description, website) and **About** (topics, website).

## Short description (≤350 characters)

**Option A (recommended):**

> Reference pipeline: MQTT factory telemetry → Rust edge (NATS JetStream buffer) → gRPC → Go cloud (Kafka → TimescaleDB). Protobuf contracts, Docker Compose, industrial IoT.

**Option B (shorter):**

> Edge-to-cloud industrial telemetry: MQTT, Rust, NATS, gRPC, Kafka, TimescaleDB — runnable with Docker Compose.

## Website / homepage

If you publish docs elsewhere, set **Website** to that URL. Otherwise use the repo itself:

`https://github.com/vgandhi1/edge-telemetry-plane`

## Topics (for search & discovery)

Add under **About → Topics** (hyphenated topics work well):

`mqtt` `nats` `jetstream` `kafka` `timescaledb` `timeseries` `grpc` `protobuf` `rust` `go` `golang` `docker-compose` `industrial-iot` `iiot` `manufacturing` `edge-computing` `telemetry` `observability` `opentelemetry` `factory` `robotics`

## Social preview (optional)

**Settings → General → Social preview** — upload a 1280×640 image (e.g. architecture diagram or logo) so links to the repo look professional on Twitter/Slack.

## From GitHub CLI

After `gh auth login`:

```bash
gh repo edit vgandhi1/edge-telemetry-plane \
  --description "Reference pipeline: MQTT factory telemetry → Rust edge (NATS JetStream) → gRPC → Go cloud (Kafka → TimescaleDB). Docker Compose." \
  --homepage "https://github.com/vgandhi1/edge-telemetry-plane" \
  --add-topic mqtt --add-topic nats --add-topic jetstream --add-topic kafka \
  --add-topic timescaledb --add-topic grpc --add-topic protobuf --add-topic rust \
  --add-topic go --add-topic docker-compose --add-topic industrial-iot --add-topic iiot \
  --add-topic edge-computing --add-topic telemetry --add-topic manufacturing
```

(You can run `gh repo edit --help` for limits on topic count.)
