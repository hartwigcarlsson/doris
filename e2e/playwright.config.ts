import { defineConfig, devices } from "@playwright/test";

// Each test starts its own `doris` server on a fresh database (see fixtures.ts),
// so there is no global webServer here. Build first: `make e2e`.
export default defineConfig({
  testDir: "./tests",
  fullyParallel: true,
  reporter: "list",
  use: { ...devices["Desktop Chrome"], trace: "retain-on-failure" },
});
