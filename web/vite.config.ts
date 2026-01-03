import { defineConfig } from 'vite';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  plugins: [tailwindcss()],
  appType: 'spa',
  server: {
    port: 5173,
    proxy: {
      '/api': {
        target: 'http://localhost:3000',
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/api/, ''),
        // WebSocket support for publish progress
        ws: true,
      },
      // Proxy local media serving (only used in dev with LOCAL_STORAGE_PATH)
      '/media': {
        target: 'http://localhost:3000',
        changeOrigin: true,
      },
    },
  },
});
