SHELL := /bin/bash

.PHONY: test lint build up down npm-test

test:
	cargo test --workspace --locked --all-targets
	cd npm/cli && npm test

lint:
	cargo clippy --workspace --all-targets -- -D warnings

build:
	cargo build --release --locked -p rystemdb-cli

npm-test:
	cd npm/cli && npm test

up:
	docker compose -f docker/compose.yml up --build -d

down:
	docker compose -f docker/compose.yml down
