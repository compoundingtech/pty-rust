import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    // Each test starts real sessions in its own registry; running them at
    // once makes the timings fight.
    fileParallelism: false,
    testTimeout: 30_000,
    hookTimeout: 30_000,
    setupFiles: ["./setup/isolate.ts"],
  },
});
