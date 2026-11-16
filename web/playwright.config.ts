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
    { name: "dev", testIgnore: /(app-login|room|sessions|shots|motion|perf|settings)\.spec\.ts/, use: { browserName: "chromium" } },
    {
      name: "app",
      // settings.spec.ts: its single-workspace test runs here, its other test in "settings".
      testMatch: /(app-login|settings)\.spec\.ts/,
      use: { browserName: "chromium", baseURL: process.env["RUBATO_E2E_APP_URL"] ?? "http://127.0.0.1:18091" },
    },
    // Issue #122: two people in a plan's room, with a companion that launches #916. Its launch
    // reorders the issue list under other tests' pointers, so it runs after them, alone.
    // After dev: these pin and launch issues the map tests must not see (916, 919).
    { name: "room", testMatch: /(room|sessions)\.spec\.ts/, dependencies: ["dev", "app"], use: { browserName: "chromium" } },
    // Issue #81: these change or watch shared state (they pin #901, which other tests expect
    // unpinned; they compare frames), so each runs alone, after every other test, in this order.
    { name: "shots", testMatch: /shots\.spec\.ts/, dependencies: ["room"], use: { browserName: "chromium" } },
    { name: "motion", testMatch: /motion\.spec\.ts/, dependencies: ["shots"], use: { browserName: "chromium" } },
    { name: "perf", testMatch: /perf\.spec\.ts/, dependencies: ["motion"], use: { browserName: "chromium" } },
    // Issue #189: it renames a workspace and moves default's map ref (and puts both back), so it
    // runs last, alone, against the dev server.
    { name: "settings", testMatch: /settings\.spec\.ts/, dependencies: ["perf"], use: { browserName: "chromium" } },
  ],
  // The JSON report feeds the scenario coverage report (pnpm scenarios report, issue #198).
  reporter: [["list"], ["json", { outputFile: "../e2e/results/playwright.json" }]],
});
