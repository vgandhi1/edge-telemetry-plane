//! MQTT → NATS JetStream: validates JSON device payloads and stores Protobuf points.

use anyhow::{anyhow, Context, Result};
use async_nats::jetstream::{self, stream::Config};
use bytes::Bytes;
use detcp_edge::TelemetryPoint;
use prost::Message;
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use std::time::Duration;
use tracing::{error, info, warn};
use url::Url;

#[derive(serde::Deserialize)]
struct DeviceJson {
    device_id: String,
    timestamp_ms: i64,
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
    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_string());
    let edge_node_id = std::env::var("EDGE_NODE_ID").unwrap_or_else(|_| "edge-01".to_string());

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

    let _ = js
        .get_or_create_stream(Config {
            name: "TELEMETRY".into(),
            subjects: vec!["telemetry.>".into()],
            max_age: Duration::from_secs(86400 * 7),
            storage: jetstream::stream::StorageType::File,
            ..Default::default()
        })
        .await;

    info!(%edge_node_id, "gateway running; mqtt + jetstream ready");

    loop {
        match eventloop.poll().await {
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
