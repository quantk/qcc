// A production build of the benchmark page: React's development build is several
// times slower and would distort the numbers.
import { fileURLToPath } from "node:url";

import { defineConfig, mergeConfig } from "vite";

import base from "../vite.config";

export default mergeConfig(
  base,
  defineConfig({
    root: fileURLToPath(new URL("..", import.meta.url)),
    build: {
      outDir: "/tmp/qcc-bench/dist",
      emptyOutDir: true,
      target: "es2022",
      rollupOptions: { input: fileURLToPath(new URL("./changes.html", import.meta.url)) },
    },
  }),
);
