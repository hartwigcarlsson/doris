.PHONY: test web e2e

# Unit and integration tests (Rust, all crates).
test:
	cargo test --workspace

# Debug build of the frontend into crates/web/dist.
web:
	cd crates/web && trunk build

# Browser tests against the debug server (frontend embedded from crates/web/dist).
e2e: web
	cargo build -p doris-server
	cd e2e && npm ci && npx playwright install chromium && npx playwright test
