# e6paint — uniform targets (AGENTS.md). `make check` is the gate.

.PHONY: default fmt fmt-check lint test build-native build-web run-native run-web check clean install-tools

default: check

install-tools:
	rustup target add wasm32-unknown-unknown
	cargo install --locked trunk

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

build-native:
	cargo build --workspace

build-web:
	cd crates/app && trunk build --release

run-native:
	cargo run -p e6-app

run-web:
	cd crates/app && trunk serve

check: fmt-check lint test

clean:
	cargo clean
	rm -rf crates/app/dist
