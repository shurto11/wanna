import { defineConfig } from "vite";

// 開発時は wannad (127.0.0.1:8787) に API を転送する
export default defineConfig({
  server: {
    proxy: { "/api": "http://127.0.0.1:8787" },
  },
});
