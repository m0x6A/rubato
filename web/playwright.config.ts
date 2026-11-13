import { defineConfig } from "@playwright/test";

// `web/e2e/run.sh` starts two servers and runs both projects:
//   dev  127.0.0.1:18090, RUBATO_DEV_USER, a seeded local git repository in place of GitHub.
//   app  127.0.0.1:18091, no dev user, configured as a GitHub App against `e2e/github-standin.ts`,
//        so the login and the team gate (ADR 0022) run in a browser.
export default defineConfig({
  testDir: "e2e",
  timeout: 60_000,
  retries: 0,
  use: {
    baseURL: process.env["RUBATO_E2E_URL"] ?? "http://127.0.0.1:18090",
    trace: "retain-on-failure",
  },
  projects: [
    { name: "dev", testIgnore: /(app-login|shots|motion|perf)\.spec\.ts/, use: { browserName: "chromium" } },
    {
      name: "app",
      testMatch: /app-login\.spec\.ts/,
      use: { browserName: "chromium", baseURL: process.env["RUBATO_E2E_APP_URL"] ?? "http://127.0.0.1:18091" },
    },
    // Issue #81: these change or watch shared state (they pin #901, which other tests expect
    // unpinned; they compare frames), so each runs alone, after every other test, in this order.
    { name: "shots", testMatch: /shots\.spec\.ts/, dependencies: ["dev", "app"], use: { browserName: "chromium" } },
    { name: "motion", testMatch: /motion\.spec\.ts/, dependencies: ["shots"], use: { browserName: "chromium" } },
    { name: "perf", testMatch: /perf\.spec\.ts/, dependencies: ["motion"], use: { browserName: "chromium" } },
  ],
  reporter: [["list"]],
});
