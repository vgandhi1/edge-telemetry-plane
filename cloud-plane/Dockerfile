FROM golang:1.22-bookworm AS builder
RUN apt-get update && apt-get install -y --no-install-recommends protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*
ENV PATH="/go/bin:${PATH}"
RUN go install google.golang.org/protobuf/cmd/protoc-gen-go@v1.35.2 \
    && go install google.golang.org/grpc/cmd/protoc-gen-go-grpc@v1.5.1

WORKDIR /build
COPY proto /build/proto
COPY cloud-plane /build/cloud-plane
WORKDIR /build/cloud-plane
RUN mkdir -p /out \
    && rm -rf gen && mkdir -p gen/detcp/v1 \
    && protoc -I /build/proto /build/proto/detcp/v1/telemetry.proto \
        --go_out=./gen --go_opt=paths=source_relative \
        --go-grpc_out=./gen --go-grpc_opt=paths=source_relative \
    && go mod download \
    && go build -ldflags="-s -w" -o /out/ingress ./cmd/ingress \
    && go build -ldflags="-s -w" -o /out/processor ./cmd/processor

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /out/ingress /out/processor /usr/local/bin/
