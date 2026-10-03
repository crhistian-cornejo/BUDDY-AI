import { cloudflareTest } from "@cloudflare/vitest-pool-workers";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [
    cloudflareTest({
      wrangler: { configPath: "./wrangler.jsonc" },
      // The owner key used by the tests. APNs secrets are set per test.
      miniflare: { bindings: { OWNER_KEY: "test-owner-key" } },
    }),
  ],
  // The relay logs room ids and event kinds; show them only for failing tests.
  test: { include: ["test/**/*.test.ts"], silent: "passed-only" },
});
