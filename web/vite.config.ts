import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const server = process.env["RUBATO_SERVER"] ?? "http://127.0.0.1:8080";

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      "/api": server,
      "/auth": server,
      "/ws": { target: server.replace(/^http/, "ws"), ws: true },
    },
  },
  build: { outDir: "dist", sourcemap: true },
});
