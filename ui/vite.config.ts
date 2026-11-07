import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { '@': decodeURIComponent(new URL('./src', import.meta.url).pathname) } },
  server: { port: 4520, strictPort: true, proxy: { '/api': { target: 'http://127.0.0.1:4510', changeOrigin: true } } },
  test: { environment: 'node', maxWorkers: 1 },
});
