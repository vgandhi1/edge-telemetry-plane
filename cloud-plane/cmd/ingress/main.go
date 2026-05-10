package main

import (
	"context"
	"log/slog"
	"net"
	"os"

	detcpv1 "detcp/cloud-plane/gen/detcp/v1"

	"github.com/segmentio/kafka-go"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"
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
		return nil, status.Error(codes.Unavailable, "kafka unavailable")
	}

	return &detcpv1.SyncResponse{
		Success:          true,
		ProcessedCount: int32(len(batch.Points)),
	}, nil
}

func main() {
	slog.SetDefault(slog.New(slog.NewTextHandler(os.Stderr, &slog.HandlerOptions{Level: slog.LevelInfo})))

	addr := getenv("GRPC_LISTEN", ":50051")
	brokers := getenv("KAFKA_BROKERS", "localhost:9092")

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

	s := grpc.NewServer()
	detcpv1.RegisterEdgeSyncServiceServer(s, &ingressServer{w: w})

	slog.Info("ingress listening", "addr", addr, "kafka", brokers)
	if err := s.Serve(lis); err != nil {
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
	var out []string
	for _, p := range splitComma(csv) {
		if p != "" {
			out = append(out, p)
		}
	}
	if len(out) == 0 {
		return []string{"localhost:9092"}
	}
	return out
}

func splitComma(s string) []string {
	var cur string
	var all []string
	for _, r := range s {
		if r == ',' {
			all = append(all, cur)
			cur = ""
			continue
		}
		cur += string(r)
	}
	all = append(all, cur)
	return all
}
