.PHONY: fmt fmt-check test clippy gui-check check reuse build dist aur-check

VERSION ?= $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)

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
	cargo build --release --locked --workspace

dist: build
	packaging/release/package.sh "$(VERSION)" dist
	packaging/release/source-archive.sh "$(VERSION)" dist
	cd dist && sha256sum "nfc-foundry-$(VERSION)-x86_64.tar.gz" \
		"nfc-foundry-$(VERSION)-source.tar.gz" >SHA256SUMS

aur-check:
	namcap packaging/aur/PKGBUILD
	cd packaging/aur && makepkg --printsrcinfo | diff -u .SRCINFO -
