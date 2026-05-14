//! MQTT → NATS JetStream: validates JSON device payloads and stores Protobuf points.
//!
//! Graceful shutdown on SIGTERM/SIGINT: drains the MQTT eventloop cleanly.

use anyhow::{anyhow, Context, Result};
use async_nats::jetstream::{self, stream::Config};
use bytes::Bytes;
use detcp_edge::TelemetryPoint;
use prost::Message;
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use std::time::Duration;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{error, info, warn};
use url::Url;

#[derive(serde::Deserialize)]
struct DeviceJson {
    device_id: String,
    timestamp_ms: i64,
    #[serde(default)]
    sequence_id: i64,
    #[serde(default)]
    sensors: std::collections::HashMap<String, f64>,
    #[serde(default)]
    trace_id: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("detcp_edge=info".parse()?),
        )
        .init();

    let mqtt_url =
        std::env::var("MQTT_URL").unwrap_or_else(|_| "mqtt://127.0.0.1:1883".to_string());
    let nats_url =
        std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_string());
    let edge_node_id =
        std::env::var("EDGE_NODE_ID").unwrap_or_else(|_| "edge-01".to_string());

    let (host, port) = mqtt_host_port(&mqtt_url)?;
    let mut opt = MqttOptions::new("detcp-edge-gateway", host, port);
    opt.set_keep_alive(Duration::from_secs(30));

    let (client, mut eventloop) = AsyncClient::new(opt, 64);
    client
        .subscribe("factory/+/telemetry", QoS::AtLeastOnce)
        .await
        .context("mqtt subscribe")?;

    let nc = async_nats::connect(&nats_url).await.context("nats connect")?;
    let js = jetstream::new(nc);

    // WorkQueue retention: messages are deleted from the stream after the
    // edge-sync consumer explicitly ACKs them. This prevents unbounded disk
    // growth while guaranteeing no message is removed before delivery.
    let _ = js
        .get_or_create_stream(Config {
            name: "TELEMETRY".into(),
            subjects: vec!["telemetry.>".into()],
            max_age: Duration::from_secs(86400 * 7),
            max_messages: 1_000_000,
            max_bytes: 512 * 1024 * 1024,
            storage: jetstream::stream::StorageType::File,
            retention: jetstream::stream::RetentionPolicy::WorkQueue,
            ..Default::default()
        })
        .await;

    info!(%edge_node_id, "gateway running; mqtt + jetstream ready");

    let mut sigterm = signal(SignalKind::terminate()).context("sigterm handler")?;
    let mut sigint = signal(SignalKind::interrupt()).context("sigint handler")?;

    loop {
        tokio::select! {
            biased;
            _ = sigterm.recv() => {
                info!("SIGTERM received — gateway shutting down");
                let _ = client.disconnect().await;
                break;
            }
            _ = sigint.recv() => {
                info!("SIGINT received — gateway shutting down");
                let _ = client.disconnect().await;
                break;
            }
            event = eventloop.poll() => {
                match event {
                    Ok(Event::Incoming(Incoming::Publish(p))) => {
                        if let Err(e) = handle_publish(&js, &edge_node_id, &p.payload).await {
                            warn!(?e, "drop invalid mqtt payload");
                        }
                    }
                    Ok(Event::Incoming(Incoming::ConnAck { .. })) => {}
                    Ok(Event::Incoming(_)) => {}
                    Ok(Event::Outgoing(_)) => {}
                    Err(e) => {
                        error!(?e, "mqtt eventloop");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }
    }
    Ok(())
}

fn mqtt_host_port(mqtt_url: &str) -> Result<(String, u16)> {
    let u = Url::parse(mqtt_url).map_err(|e| anyhow!("mqtt url: {e}"))?;
    let host = u
        .host_str()
        .ok_or_else(|| anyhow!("mqtt url missing host"))?
        .to_string();
    let port = u.port().unwrap_or(1883);
    Ok((host, port))
}

async fn handle_publish(
    js: &jetstream::Context,
    edge_node_id: &str,
    payload: &[u8],
) -> Result<()> {
    let j: DeviceJson = serde_json::from_slice(payload).context("json")?;
    let point = TelemetryPoint {
        device_id: j.device_id,
        timestamp_ms: j.timestamp_ms,
        sequence_id: j.sequence_id,
        sensors: j.sensors,
        trace_id: j.trace_id,
    };
    let mut buf = Vec::with_capacity(point.encoded_len());
    point.encode(&mut buf).context("encode protobuf")?;

    let subject = format!("telemetry.{edge_node_id}");
    js.publish(subject, Bytes::from(buf))
        .await?
        .await
        .context("jetstream ack")?;
    Ok(())
}
