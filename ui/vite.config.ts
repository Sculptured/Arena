import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  server: { port: 4520, strictPort: true, proxy: { '/api': { target: 'http://127.0.0.1:4510', changeOrigin: true } } },
  test: { environment: 'node' },
});
