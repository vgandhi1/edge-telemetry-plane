//! NATS JetStream → gRPC cloud ingress (batched, with exponential backoff and Prometheus metrics).
//!
//! Fault tolerance contract:
//!   - Messages are NAK'd (not ACK'd) on gRPC failure so NATS redelivers them.
//!   - Failed gRPC calls back off exponentially (500ms → 30s) to avoid tight-
//!     looping against a dead connection during a network partition.
//!   - Prometheus counters let Grafana surface the outage and recovery in real time.
//!   - SIGTERM/SIGINT flush any pending batch before exiting.

use anyhow::{Context, Result};
use async_nats::jetstream::{self, consumer};
use detcp_edge::{EdgeSyncServiceClient, TelemetryBatch, TelemetryPoint};
use futures::StreamExt;
use metrics::{counter, gauge};
use metrics_exporter_prometheus::PrometheusBuilder;
use prost::Message as _;
use std::time::Duration;
use tokio::signal::unix::{signal, SignalKind};
use tokio::time::interval;
use tonic::transport::Channel;
use tracing::{error, info, warn};

const BATCH_MAX: usize = 100;
const FLUSH_MS: u64 = 1000;
const BACKOFF_INIT: Duration = Duration::from_millis(500);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Tracks exponential backoff state for gRPC failures. Resets on success.
struct GrpcBackoff {
    current: Duration,
}

impl GrpcBackoff {
    fn new() -> Self {
        Self { current: Duration::ZERO }
    }

    fn reset(&mut self) {
        self.current = Duration::ZERO;
    }

    /// Sleeps for the current delay then doubles it (capped at BACKOFF_MAX).
    async fn wait_and_advance(&mut self) {
        if self.current.is_zero() {
            self.current = BACKOFF_INIT;
        } else {
            tokio::time::sleep(self.current).await;
            self.current = (self.current * 2).min(BACKOFF_MAX);
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("detcp_edge=info".parse()?),
        )
        .init();

    let metrics_port: u16 = std::env::var("METRICS_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(9100);

    PrometheusBuilder::new()
        .with_http_listener(([0, 0, 0, 0], metrics_port))
        .install()
        .context("prometheus exporter")?;

    info!(port = metrics_port, "Prometheus metrics endpoint started");

    let nats_url =
        std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_string());
    let ingress =
        std::env::var("INGRESS_ADDR").unwrap_or_else(|_| "http://127.0.0.1:50051".to_string());
    let edge_node_id =
        std::env::var("EDGE_NODE_ID").unwrap_or_else(|_| "edge-01".to_string());

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

    let channel = connect_with_retry(&ingress).await?;
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
    let mut backoff = GrpcBackoff::new();

    let mut sigterm = signal(SignalKind::terminate()).context("sigterm handler")?;
    let mut sigint = signal(SignalKind::interrupt()).context("sigint handler")?;

    info!(%edge_node_id, %ingress, "edge-sync running");

    loop {
        tokio::select! {
            biased;
            _ = sigterm.recv() => {
                info!("SIGTERM — flushing pending batch before exit");
                if !pending.is_empty() {
                    flush_batch(&mut client, &edge_node_id, &mut pending, &mut backoff).await;
                }
                break;
            }
            _ = sigint.recv() => {
                info!("SIGINT — flushing pending batch before exit");
                if !pending.is_empty() {
                    flush_batch(&mut client, &edge_node_id, &mut pending, &mut backoff).await;
                }
                break;
            }
            _ = tick.tick(), if !pending.is_empty() => {
                flush_batch(&mut client, &edge_node_id, &mut pending, &mut backoff).await;
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
                            flush_batch(&mut client, &edge_node_id, &mut pending, &mut backoff).await;
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
    Ok(())
}

/// Retries gRPC channel creation with backoff; needed when Toxiproxy isn't up yet.
async fn connect_with_retry(ingress: &str) -> Result<Channel> {
    let mut delay = BACKOFF_INIT;
    loop {
        match Channel::from_shared(ingress.to_string())
            .context("ingress channel")?
            .connect_timeout(Duration::from_secs(5))
            .connect()
            .await
        {
            Ok(ch) => return Ok(ch),
            Err(e) => {
                warn!(?e, "initial gRPC connect failed; retrying in {:?}", delay);
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(BACKOFF_MAX);
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
    backoff: &mut GrpcBackoff,
) {
    if pending.is_empty() {
        return;
    }

    // Honour any active backoff delay before attempting a send.
    backoff.wait_and_advance().await;

    let taken: Vec<_> = pending.drain(..).collect();
    let points: Vec<TelemetryPoint> = taken.iter().map(|(_, p)| p.clone()).collect();
    let n = points.len() as u64;
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
                counter!("edge_sync_grpc_send_total", "status" => "success").increment(n);
                gauge!("edge_sync_grpc_backoff_seconds").set(0.0);
                backoff.reset();
                info!(processed = inner.processed_count, "sync ok");
            } else {
                warn!("ingress returned success=false; nak batch");
                for (m, _) in taken {
                    let _ = m
                        .ack_with(async_nats::jetstream::AckKind::Nak(None))
                        .await;
                }
                counter!("edge_sync_grpc_send_total", "status" => "error").increment(n);
                counter!("edge_sync_retries_total").increment(1);
                gauge!("edge_sync_grpc_backoff_seconds")
                    .set(backoff.current.as_secs_f64());
            }
        }
        Err(e) => {
            error!(?e, "grpc sync failed; nak batch (backoff={:?})", backoff.current);
            for (m, _) in taken {
                let _ = m
                    .ack_with(async_nats::jetstream::AckKind::Nak(None))
                    .await;
            }
            counter!("edge_sync_grpc_send_total", "status" => "error").increment(n);
            counter!("edge_sync_retries_total").increment(1);
            gauge!("edge_sync_grpc_backoff_seconds")
                .set(backoff.current.as_secs_f64());
        }
    }
}
