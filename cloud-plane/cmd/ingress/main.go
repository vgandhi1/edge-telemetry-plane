package main

import (
	"context"
	"log/slog"
	"net"
	"net/http"
	"os"
	"os/signal"
	"strings"
	"syscall"

	detcpv1 "detcp/cloud-plane/gen/detcp/v1"

	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/client_golang/prometheus/promauto"
	"github.com/prometheus/client_golang/prometheus/promhttp"
	"github.com/segmentio/kafka-go"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"
)

var (
	messagesReceived = promauto.NewCounter(prometheus.CounterOpts{
		Name: "cloud_ingress_messages_received_total",
		Help: "Total individual telemetry points received from edge-sync workers",
	})
	batchesReceived = promauto.NewCounter(prometheus.CounterOpts{
		Name: "cloud_ingress_batches_received_total",
		Help: "Total gRPC batches received from edge-sync workers",
	})
	kafkaPublishErrors = promauto.NewCounter(prometheus.CounterOpts{
		Name: "cloud_ingress_kafka_publish_errors_total",
		Help: "Failed Kafka publish attempts",
	})
)

type ingressServer struct {
	detcpv1.UnimplementedEdgeSyncServiceServer
	w *kafka.Writer
}

func (s *ingressServer) SyncTelemetry(ctx context.Context, batch *detcpv1.TelemetryBatch) (*detcpv1.SyncResponse, error) {
	if batch == nil || batch.EdgeNodeId == "" {
		return nil, status.Error(codes.InvalidArgument, "invalid batch")
	}
	if len(batch.Points) == 0 {
		return &detcpv1.SyncResponse{Success: true, ProcessedCount: 0}, nil
	}

	payload, err := proto.Marshal(batch)
	if err != nil {
		return nil, status.Error(codes.Internal, "encode failed")
	}

	err = s.w.WriteMessages(ctx, kafka.Message{
		Key:   []byte(batch.EdgeNodeId),
		Value: payload,
	})
	if err != nil {
		slog.Error("kafka write", "error", err)
		kafkaPublishErrors.Inc()
		return nil, status.Error(codes.Unavailable, "kafka unavailable")
	}

	batchesReceived.Inc()
	messagesReceived.Add(float64(len(batch.Points)))

	return &detcpv1.SyncResponse{
		Success:        true,
		ProcessedCount: int32(len(batch.Points)),
	}, nil
}

func main() {
	slog.SetDefault(slog.New(slog.NewTextHandler(os.Stderr, &slog.HandlerOptions{Level: slog.LevelInfo})))

	addr := getenv("GRPC_LISTEN", ":50051")
	metricsAddr := getenv("METRICS_ADDR", ":9101")
	brokers := getenv("KAFKA_BROKERS", "localhost:9092")

	ctx, cancel := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer cancel()

	lis, err := net.Listen("tcp", addr)
	if err != nil {
		slog.Error("listen", "error", err)
		os.Exit(1)
	}

	w := &kafka.Writer{
		Addr:         kafka.TCP(splitBrokers(brokers)...),
		Topic:        "detcp.telemetry.batches",
		Balancer:     &kafka.Hash{},
		RequiredAcks: kafka.RequireOne,
		Async:        false,
	}
	defer w.Close()

	// Prometheus metrics endpoint — scraped by Prometheus at METRICS_ADDR/metrics.
	mux := http.NewServeMux()
	mux.Handle("/metrics", promhttp.Handler())
	metricsSrv := &http.Server{Addr: metricsAddr, Handler: mux}
	go func() {
		slog.Info("metrics listening", "addr", metricsAddr)
		if err := metricsSrv.ListenAndServe(); err != nil && err != http.ErrServerClosed {
			slog.Error("metrics server", "error", err)
		}
	}()

	grpcSrv := grpc.NewServer()
	detcpv1.RegisterEdgeSyncServiceServer(grpcSrv, &ingressServer{w: w})

	go func() {
		<-ctx.Done()
		slog.Info("shutdown signal received")
		grpcSrv.GracefulStop()
		_ = metricsSrv.Shutdown(context.Background())
	}()

	slog.Info("ingress listening", "addr", addr, "kafka", brokers, "metrics", metricsAddr)
	if err := grpcSrv.Serve(lis); err != nil {
		slog.Error("serve", "error", err)
		os.Exit(1)
	}
}

func getenv(k, def string) string {
	if v := os.Getenv(k); v != "" {
		return v
	}
	return def
}

func splitBrokers(csv string) []string {
	parts := strings.Split(csv, ",")
	var out []string
	for _, p := range parts {
		p = strings.TrimSpace(p)
		if p != "" {
			out = append(out, p)
		}
	}
	if len(out) == 0 {
		return []string{"localhost:9092"}
	}
	return out
}
