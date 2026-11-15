import { fileURLToPath } from "node:url";

import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  // `@/` is the import alias shadcn components are written against.
  resolve: { alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) } },
  // Tauri expects a fixed port and prints its own logs.
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
  build: { target: "es2022", outDir: "dist" },
  test: { environment: "node", include: ["src/**/*.test.ts"] },
});
