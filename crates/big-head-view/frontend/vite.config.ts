import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Vite builds the R3F frontend into crates/big-head-view/frontend/dist.
// big-head-view's axum server serves those files at the root URL.
export default defineConfig({
  plugins: [react()],
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
  server: {
    port: 5173,
    proxy: {
      '/ws': { target: 'ws://localhost:3030', ws: true },
    },
  },
});
