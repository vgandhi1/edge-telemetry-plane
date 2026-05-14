package main

import (
	"context"
	"encoding/json"
	"log/slog"
	"math/rand"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"

	detcpv1 "detcp/cloud-plane/gen/detcp/v1"

	"github.com/jackc/pgx/v5/pgxpool"
	"github.com/segmentio/kafka-go"
	"google.golang.org/protobuf/proto"
)

const (
	persistMaxRetries = 8
	persistBaseDelay  = 100 * time.Millisecond
	persistMaxDelay   = 30 * time.Second
)

func main() {
	slog.SetDefault(slog.New(slog.NewTextHandler(os.Stderr, &slog.HandlerOptions{Level: slog.LevelInfo})))

	brokers := getenv("KAFKA_BROKERS", "localhost:9092")
	dsn := getenv("DATABASE_URL", "postgres://detcp:detcp_dev_change_me@localhost:5432/detcp?sslmode=disable")

	ctx, cancel := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer cancel()

	pool, err := pgxpool.New(ctx, dsn)
	if err != nil {
		slog.Error("db connect", "error", err)
		os.Exit(1)
	}
	defer pool.Close()

	if err := pool.Ping(ctx); err != nil {
		slog.Error("db ping", "error", err)
		os.Exit(1)
	}

	r := kafka.NewReader(kafka.ReaderConfig{
		Brokers: splitBrokers(brokers),
		GroupID: "detcp-processor",
		Topic:   "detcp.telemetry.batches",
	})
	defer r.Close()

	slog.Info("processor running", "kafka", brokers)

	for {
		select {
		case <-ctx.Done():
			slog.Info("shutdown")
			return
		default:
		}

		m, err := r.FetchMessage(ctx)
		if err != nil {
			if ctx.Err() != nil {
				return
			}
			slog.Error("fetch", "error", err)
			time.Sleep(time.Second)
			continue
		}

		var batch detcpv1.TelemetryBatch
		if err := proto.Unmarshal(m.Value, &batch); err != nil {
			slog.Error("unmarshal batch", "error", err)
			if err := r.CommitMessages(ctx, m); err != nil {
				slog.Error("commit bad msg", "error", err)
			}
			continue
		}

		if err := persistWithBackoff(ctx, pool, &batch); err != nil {
			slog.Error("persist exhausted retries", "error", err)
			// Do not commit: leave message for the next consumer restart.
			continue
		}

		if err := r.CommitMessages(ctx, m); err != nil {
			slog.Error("commit", "error", err)
		}
	}
}

// persistWithBackoff retries persistBatch with exponential backoff + jitter.
// It gives up after persistMaxRetries and returns the last error.
func persistWithBackoff(ctx context.Context, pool *pgxpool.Pool, batch *detcpv1.TelemetryBatch) error {
	delay := persistBaseDelay
	var lastErr error
	for attempt := 0; attempt < persistMaxRetries; attempt++ {
		if err := persistBatch(ctx, pool, batch); err == nil {
			return nil
		} else {
			lastErr = err
			if attempt+1 == persistMaxRetries {
				break
			}
			// Full jitter: sleep between [delay/2, delay) to spread thundering-herd retries.
			jitter := time.Duration(rand.Int63n(int64(delay/2))) + delay/2
			slog.Warn("persist failed; retrying", "attempt", attempt+1, "delay", jitter, "error", err)
			select {
			case <-ctx.Done():
				return ctx.Err()
			case <-time.After(jitter):
			}
			if delay < persistMaxDelay {
				delay = min(delay*2, persistMaxDelay)
			}
		}
	}
	return lastErr
}

func persistBatch(ctx context.Context, pool *pgxpool.Pool, batch *detcpv1.TelemetryBatch) error {
	for _, p := range batch.Points {
		ts := time.UnixMilli(p.TimestampMs).UTC()
		sensors, err := json.Marshal(p.Sensors)
		if err != nil {
			return err
		}
		_, err = pool.Exec(ctx, `
			INSERT INTO telemetry_points (time, edge_node_id, device_id, sensors, trace_id, sequence_id)
			VALUES ($1, $2, $3, $4::jsonb, $5, $6)
		`, ts, batch.EdgeNodeId, p.DeviceId, string(sensors), p.TraceId, p.SequenceId)
		if err != nil {
			return err
		}
	}
	return nil
}

func min(a, b time.Duration) time.Duration {
	if a < b {
		return a
	}
	return b
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
