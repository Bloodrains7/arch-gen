import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/e2e",
  workers: 1,
  use: { baseURL: "http://127.0.0.1:5187", viewport: { width: 1400, height: 950 } },
  webServer: {
    command: "node node_modules/vite/bin/vite.js --host 127.0.0.1 --port 5187",
    url: "http://127.0.0.1:5187", reuseExistingServer: false,
  },
});
