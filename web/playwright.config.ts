import { defineConfig } from "@playwright/test";

// The e2e suite expects a running rubato-server with RUBATO_DEV_USER and the web dist served.
// `web/e2e/run.sh` starts one on 127.0.0.1:18090 with the map loaded from the local file.
export default defineConfig({
  testDir: "e2e",
  timeout: 60_000,
  retries: 0,
  use: {
    baseURL: process.env["RUBATO_E2E_URL"] ?? "http://127.0.0.1:18090",
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { browserName: "chromium" } }],
  reporter: [["list"]],
});
