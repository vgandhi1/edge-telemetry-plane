# DETCP — local factory stack
.PHONY: proto up down logs smoke

proto:
	@test -n "$$(command -v protoc)" || { echo "Install protoc (https://grpc.io/docs/protoc-installation/) or use Docker."; exit 1; }
	@test -n "$$(command -v protoc-gen-go)" || go install google.golang.org/protobuf/cmd/protoc-gen-go@v1.35.2
	@test -n "$$(command -v protoc-gen-go-grpc)" || go install google.golang.org/grpc/cmd/protoc-gen-go-grpc@v1.5.1
	mkdir -p cloud-plane/gen/detcp/v1
	protoc -I proto proto/detcp/v1/telemetry.proto \
		--go_out=cloud-plane/gen --go_opt=paths=source_relative \
		--go-grpc_out=cloud-plane/gen --go-grpc_opt=paths=source_relative
	cd cloud-plane && go mod tidy

up:
	cd deploy && docker compose up -d --build

down:
	cd deploy && docker compose down -v

logs:
	cd deploy && docker compose logs -f --tail=100 edge-gateway edge-sync ingress processor

smoke:
	python3 scripts/run_dev_check.py
	@echo "With stack up: python3 -m venv .venv && . .venv/bin/activate && pip install -r scripts/requirements.txt && python3 scripts/simulate_robot_fleet.py"
