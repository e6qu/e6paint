// Playwright e2e for the wasm app. The browser downloads stay inside the repo
// via PLAYWRIGHT_BROWSERS_PATH (set it for both `playwright install` and
// `playwright test`; the npm script does this for you).
//
// The webServer boots `trunk serve` (debug profile) on a fixed port; tests run
// against the live wasm app.

module.exports = {
  testDir: "e2e",
  timeout: 60_000,
  use: {
    baseURL: "http://127.0.0.1:4173",
    // Headless Chromium needs explicit opt-in for WebGPU, which the app
    // requires (it refuses to start without a GPU surface).
    launchOptions: {
      args: ["--enable-unsafe-webgpu"],
    },
  },
  webServer: [
    {
      command: "trunk serve --port 4173",
      cwd: "crates/app",
      url: "http://127.0.0.1:4173",
      reuseExistingServer: true,
      timeout: 300_000,
    },
    {
      // The client's RELAY_HTTP_BASE is 127.0.0.1:8480; log and DB land under
      // target/ where the specs poll them. One shell string so the redirect and
      // inline env actually apply.
      command:
        "RUST_LOG=info E6PAINT_RELAY_DB=../../target/e2e-relay.db exec ../../target/debug/e6-relay > ../../target/e2e-relay.log 2>&1",
      cwd: "crates/app",
      port: 8480,
      reuseExistingServer: true,
      timeout: 60_000,
    },
  ],
};
