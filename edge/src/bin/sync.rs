//! NATS JetStream → gRPC cloud ingress (batched).

use anyhow::{Context, Result};
use async_nats::jetstream::{self, consumer};
use detcp_edge::{EdgeSyncServiceClient, TelemetryBatch, TelemetryPoint};
use futures::StreamExt;
use prost::Message as _;
use std::time::Duration;
use tokio::time::interval;
use tonic::transport::Channel;
use tracing::{error, info, warn};

const BATCH_MAX: usize = 100;
const FLUSH_MS: u64 = 1000;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("detcp_edge=info".parse()?),
        )
        .init();

    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_string());
    let ingress = std::env::var("INGRESS_ADDR").unwrap_or_else(|_| "http://127.0.0.1:50051".to_string());
    let edge_node_id = std::env::var("EDGE_NODE_ID").unwrap_or_else(|_| "edge-01".to_string());

    let nc = async_nats::connect(&nats_url).await.context("nats connect")?;
    let js = jetstream::new(nc);
    let stream = wait_for_stream(&js).await.context("get TELEMETRY stream")?;

    let durable = format!("edge-sync-{edge_node_id}");
    let filter = format!("telemetry.{edge_node_id}");
    let pull_consumer = stream
        .get_or_create_consumer(
            &durable,
            consumer::pull::Config {
                durable_name: Some(durable.clone()),
                filter_subject: filter,
                ack_policy: consumer::AckPolicy::Explicit,
                ..Default::default()
            },
        )
        .await
        .context("get_or_create_consumer")?;

    let channel = Channel::from_shared(ingress.clone())
        .context("ingress channel")?
        .connect_timeout(Duration::from_secs(5))
        .connect()
        .await
        .with_context(|| format!("connect ingress {ingress}"))?;

    let mut client = EdgeSyncServiceClient::new(channel);
    let mut tick = interval(Duration::from_millis(FLUSH_MS));
    tick.tick().await;
    let mut messages = pull_consumer
        .stream()
        .max_messages_per_batch(50)
        .messages()
        .await
        .context("consumer messages")?;

    let mut pending: Vec<(async_nats::jetstream::Message, TelemetryPoint)> = Vec::new();

    info!(%edge_node_id, %ingress, "edge-sync running");

    loop {
        tokio::select! {
            biased;
            _ = tick.tick(), if !pending.is_empty() => {
                flush_batch(&mut client, &edge_node_id, &mut pending).await;
            }
            msg = messages.next() => {
                match msg {
                    Some(Ok(m)) => {
                        match TelemetryPoint::decode(m.payload.as_ref()) {
                            Ok(pt) => pending.push((m, pt)),
                            Err(e) => {
                                warn!(?e, "bad protobuf; nak");
                                let _ = m
                                    .ack_with(async_nats::jetstream::AckKind::Nak(None))
                                    .await;
                            }
                        }
                        if pending.len() >= BATCH_MAX {
                            flush_batch(&mut client, &edge_node_id, &mut pending).await;
                        }
                    }
                    Some(Err(e)) => warn!(?e, "jetstream recv"),
                    None => {
                        error!("jetstream message stream ended");
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                }
            }
        }
    }
}

async fn wait_for_stream(js: &jetstream::Context) -> Result<jetstream::stream::Stream> {
    for attempt in 1..=90 {
        match js.get_stream("TELEMETRY").await {
            Ok(s) => return Ok(s),
            Err(_) => {
                if attempt == 1 || attempt % 15 == 0 {
                    info!(attempt, "waiting for TELEMETRY stream (start edge-gateway first)");
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
    anyhow::bail!("TELEMETRY stream not available after 90s");
}

async fn flush_batch(
    client: &mut EdgeSyncServiceClient<Channel>,
    edge_node_id: &str,
    pending: &mut Vec<(async_nats::jetstream::Message, TelemetryPoint)>,
) {
    if pending.is_empty() {
        return;
    }
    let taken: Vec<_> = pending.drain(..).collect();
    let points: Vec<TelemetryPoint> = taken.iter().map(|(_, p)| p.clone()).collect();
    let batch = TelemetryBatch {
        edge_node_id: edge_node_id.to_string(),
        points,
    };
    match client.sync_telemetry(tonic::Request::new(batch)).await {
        Ok(resp) => {
            let inner = resp.into_inner();
            if inner.success {
                for (m, _) in taken {
                    let _ = m.ack().await;
                }
                info!(processed = inner.processed_count, "sync ok");
            } else {
                warn!("ingress returned success=false; nak batch");
                for (m, _) in taken {
                    let _ = m
                        .ack_with(async_nats::jetstream::AckKind::Nak(None))
                        .await;
                }
            }
        }
        Err(e) => {
            error!(?e, "grpc sync failed; nak batch");
            for (m, _) in taken {
                let _ = m
                    .ack_with(async_nats::jetstream::AckKind::Nak(None))
                    .await;
            }
        }
    }
}
