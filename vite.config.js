import { defineConfig } from "vite";

export default defineConfig({
  build: {
    emptyOutDir: true,
    lib: {
      entry: "web/memory-graph.ts",
      formats: ["es"],
      fileName: () => "memory-graph.js",
    },
    outDir: "web/generated",
    target: "es2022",
    minify: true,
    sourcemap: false,
    rollupOptions: {
      output: {
        banner: "/*! vgpu 0.3.1 | MIT License | /assets/vgpu-LICENSE.txt */",
      },
    },
  },
});
