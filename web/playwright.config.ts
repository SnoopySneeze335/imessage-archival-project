import { defineConfig, devices } from "@playwright/test";

// Browser tests against the real server binary and the built app (web/dist),
// on fixture data only. Each test starts its own server (e2e/server.ts).
//
//   cargo build -p imessage-ap && npm run build
//   scripts/demo.sh --prepare          # demo data for demo.spec.ts
//   npm run e2e -- --project=chromium  # webkit too where it is installed
export default defineConfig({
  testDir: "e2e",
  timeout: 60_000,
  fullyParallel: false,
  workers: 1,
  forbidOnly: !!process.env.CI,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  use: {
    trace: "off",
    screenshot: "off",
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    { name: "webkit", use: { ...devices["Desktop Safari"] } },
    { name: "iphone", use: { ...devices["iPhone 15"] } },
  ],
});
