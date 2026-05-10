# DETCP — Documentation index

Welcome to the documentation for the **Distributed Edge Telemetry & Control Plane (DETCP)**.

The **project overview**, badges, and primary entry for visitors are in the repository root:

**[../README.md](../README.md)** · **Upstream:** [github.com/vgandhi1/edge-telemetry-plane](https://github.com/vgandhi1/edge-telemetry-plane)

---

## GitHub discoverability

Suggested **description**, **topics**, and `gh repo edit` commands: [github-repository-metadata.md](github-repository-metadata.md).

---

## In this folder

| File | What you will find |
| :--- | :--- |
| [architecture.md](architecture.md) | Edge vs cloud components, protobuf contract outline, failure modes, OpenTelemetry propagation |
| [implementation.md](implementation.md) | Phased build plan (contracts → edge → cloud → tracing → chaos) |

---

## Quick start (reference)

When runtime directories are present in the repo, local simulation is intended to work like this:

1. Start infra and observability stacks from `deploy/`.
2. Run cloud ingress from `cloud-plane/`.
3. Run the Rust edge binaries from `edge/` (`edge-gateway`, `edge-sync`).
4. Optional: run `scripts/simulate_robot_fleet.py` and chaos tools (e.g. Toxiproxy) as described in the root README.

**End-to-end stack:** see the root [README.md](../README.md) (`make up`, simulator, SQL verification).

**Layout check:**

```bash
python3 scripts/run_dev_check.py
```

For phased roadmap and future work (OTel, mTLS, chaos), see [implementation.md](implementation.md).

---

## Conventions

- Cross-boundary payloads: **Protobuf** (versioned packages, e.g. `detcp.v1`).
- Ordering: Kafka keyed by **`edge_node_id`** where per-node ordering matters.
- Tracing: **`trace_id`** carried from edge payloads through gRPC metadata and Kafka headers into the observability backend.

For diagrams and tables in Git-friendly form, prefer the root [README.md](../README.md) and [architecture.md](architecture.md).
