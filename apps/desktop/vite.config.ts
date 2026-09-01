import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  build: {
    outDir: "dist/client",
    rollupOptions: {
      input: ["index.html", "agent.html"],
    },
  },
  optimizeDeps: {
    include: ["react", "react-dom/client"],
  },
  plugins: [react()],
  server: {
    host: "0.0.0.0",
    allowedHosts: ["terminal.local"],
    warmup: {
      clientFiles: ["./src/main.tsx", "./src/agent-worker.ts"],
    },
  },
  test: {
    environment: "jsdom",
    setupFiles: "./src/test/setup.ts",
    css: true,
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
