.PHONY: fmt fmt-check test clippy gui-check check reuse build

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all --check

test:
	cargo test --workspace

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

gui-check:
	cargo check -p nfc-foundry

reuse:
	reuse lint

check: fmt-check clippy test gui-check reuse

build:
	cargo build --release --workspace
