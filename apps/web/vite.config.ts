import { defineConfig } from 'vite'

import { tanstackStart } from '@tanstack/react-start/plugin/vite'

import viteReact from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

const BACKEND = 'http://127.0.0.1:3001'

// `@tanstack/devtools-vite` was removed from the plugin list because it
// formed a positive feedback loop with vite's own client HMR logs: the
// devtools client forwards browser console.error to the terminal, vite
// prints "[Server] <message>", the browser picks up the new HMR/client
// log line, forwards it back, repeat. 7+ MB of log spam per minute, no
// recovery. Re-enable only if there's a known fix upstream.
const config = defineConfig({
  resolve: { tsconfigPaths: true },
  plugins: [tailwindcss(), tanstackStart(), viteReact()],
  server: {
    // If 3000 is taken (e.g. a stale vite), fail loudly instead of silently
    // bumping to 3001 (which collides with the backend).
    strictPort: true,
    proxy: {
      '/api': { target: BACKEND, changeOrigin: false },
      '/health': { target: BACKEND, changeOrigin: false },
    },
  },
})

export default config
