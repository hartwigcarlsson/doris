VERSION = $(shell cargo pkgid -p doris-server | sed 's/.*@//')
DIST := target/dist
# Upper bound for the release wasm, gzipped: that is what crosses the wire
# (the server sends brotli or gzip; gzip is the larger of the two).
WASM_BUDGET := 500000

.PHONY: test web e2e e2e-dist dev dist

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

# The same browser tests against the release binary from `make dist`.
e2e-dist: dist
	cd e2e && npm ci && npx playwright install chromium && DORIS_BIN=../$(DIST)/doris npx playwright test

# Server on :3000 and `trunk serve` on :8080 (proxying the API). Open
# http://localhost:8080 — WebAuthn needs the page's origin as RP origin.
dev:
	@trap 'kill 0' EXIT; \
	DORIS_RP_ORIGIN=http://localhost:8080 cargo run -p doris-server & \
	cd crates/web && trunk serve --port 8080

# Release binary with the frontend embedded, plus the same frontend as a
# tarball for serving from a CDN or nginx.
dist:
	cd crates/web && trunk build --release --cargo-profile wasm-release
	@wasm=$$(ls crates/web/dist/*_bg.wasm); size=$$(gzip -c $$wasm | wc -c); \
		echo "dist: $$wasm is $$size bytes gzipped (budget $(WASM_BUDGET))"; \
		test $$size -le $(WASM_BUDGET) || \
		{ echo "dist: over the $(WASM_BUDGET) byte budget"; exit 1; }
	cargo build --release -p doris-server
	mkdir -p $(DIST)
	@# rm first: on macOS, overwriting a binary that has run keeps its old
	@# code signature cached, and the new one is killed on start (SIGKILL).
	rm -f $(DIST)/doris
	cp target/release/doris $(DIST)/doris
	tar -czf $(DIST)/doris-web-$(VERSION).tar.gz -C crates/web/dist .
	@tar -tzf $(DIST)/doris-web-$(VERSION).tar.gz | grep -qx './index.html' || \
		{ echo "dist: tarball missing ./index.html"; exit 1; }
	@tar -tzf $(DIST)/doris-web-$(VERSION).tar.gz | grep -q '_bg\.wasm$$' || \
		{ echo "dist: tarball missing *_bg.wasm"; exit 1; }
	@echo "built $(DIST)/doris and $(DIST)/doris-web-$(VERSION).tar.gz"
