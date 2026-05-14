#!/usr/bin/env python3
"""Publish synthetic factory MQTT telemetry (DETCP JSON schema).

Each message carries a monotonically increasing `sequence_id` so the
zero-data-loss SQL proof (MAX(sequence_id) - MIN(sequence_id) + 1 = COUNT(*))
can be verified after a chaos cycle.
"""

from __future__ import annotations

import argparse
import itertools
import json
import random
import sys
import time
import uuid

try:
    import paho.mqtt.client as mqtt
except ImportError:
    print("error: install deps: pip install -r scripts/requirements.txt", file=sys.stderr)
    sys.exit(1)

_seq_counter = itertools.count(1)


def build_payload(device_id: str, trace: str | None, sequence_id: int) -> bytes:
    body = {
        "device_id": device_id,
        "timestamp_ms": int(time.time() * 1000),
        "sequence_id": sequence_id,
        "sensors": {
            "temp_c": round(18.0 + random.random() * 8.0, 2),
            "vibration_rms": round(random.random() * 0.05, 4),
        },
        "trace_id": trace or "",
    }
    return json.dumps(body, separators=(",", ":")).encode("utf-8")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--mqtt-host", default="127.0.0.1")
    ap.add_argument("--mqtt-port", type=int, default=1883)
    ap.add_argument("--factory", default="factory01", help="MQTT topic segment: factory/<factory>/telemetry")
    ap.add_argument("--count", type=int, default=5, help="Number of logical devices to rotate per interval")
    ap.add_argument("--interval", type=float, default=1.0, help="Seconds between publish rounds")
    ap.add_argument("--trace", action="store_true", help="Attach a random trace_id per message")
    args = ap.parse_args()

    topic = f"factory/{args.factory}/telemetry"
    client = mqtt.Client(client_id=f"detcp-sim-{uuid.uuid4().hex[:8]}")
    client.connect(args.mqtt_host, args.mqtt_port, keepalive=30)
    client.loop_start()

    n = 0
    try:
        while True:
            for i in range(args.count):
                did = f"robot-{i:03d}"
                tid = uuid.uuid4().hex if args.trace else ""
                seq = next(_seq_counter)
                payload = build_payload(did, tid, seq)
                client.publish(topic, payload, qos=1)
                n += 1
            time.sleep(args.interval)
    except KeyboardInterrupt:
        pass
    finally:
        client.loop_stop()
        client.disconnect()

    print(f"published {n} messages (last sequence_id={n})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
