import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  // 本机 1420 落在 Windows 保留端口段（1420-1519，Hyper-V winnat）且无管理员权限重置，
  // 退而使用紧邻的可用端口 1520。
  server: {
    port: 1520,
    strictPort: true,
  },
  build: {
    target: "es2021",
  },
});
