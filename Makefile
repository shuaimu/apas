.PHONY: all build test lint fmt clean dev-server dev-web

# Default target
all: build test

# Build everything
build:
	cargo build --release
	cd packages/web && pnpm build

# Run all tests
test:
	cargo test
	cd packages/web && pnpm test

# Run Rust tests only
test-rust:
	cargo test

# Run web tests only
test-web:
	cd packages/web && pnpm test

# Run linters
lint:
	cargo fmt --all -- --check
	cargo clippy --all-targets --all-features -- -D warnings
	cd packages/web && pnpm lint

# Format code
fmt:
	cargo fmt --all
	cd packages/web && pnpm exec prettier --write "src/**/*.{ts,tsx}"

# Clean build artifacts
clean:
	cargo clean
	cd packages/web && rm -rf .next node_modules

# Development commands
dev-server:
	cargo run --bin apas-server

dev-web:
	cd packages/web && pnpm dev

# Install dependencies
install:
	cd packages/web && pnpm install

# Watch mode for web tests
test-watch:
	cd packages/web && pnpm test:watch

# Help
help:
	@echo "Available targets:"
	@echo "  all        - Build and test everything"
	@echo "  build      - Build release binaries and web app"
	@echo "  test       - Run all tests"
	@echo "  test-rust  - Run Rust tests only"
	@echo "  test-web   - Run web tests only"
	@echo "  lint       - Run all linters"
	@echo "  fmt        - Format all code"
	@echo "  clean      - Clean build artifacts"
	@echo "  dev-server - Run development server"
	@echo "  dev-web    - Run web development server"
	@echo "  install    - Install dependencies"
	@echo "  test-watch - Run web tests in watch mode"
