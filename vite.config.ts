import { defineConfig } from 'vite';
import { resolve } from 'node:path';

// DictatingMe 前端构建配置：MainWindow / HudWindow 两个独立入口
// 对应 runtime/tauri.conf.json 中的 build.frontendDist = "../dist"
export default defineConfig({
  root: 'ui',
  build: {
    outDir: '../dist',
    emptyOutDir: true,
    rollupOptions: {
      input: {
        main: resolve(__dirname, 'ui/main-window/index.html'),
        hud: resolve(__dirname, 'ui/hud-window/index.html'),
      },
    },
  },
  server: {
    port: 1420,
    strictPort: true,
  },
});
