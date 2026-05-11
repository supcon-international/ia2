import { defineConfig } from 'vite'
import { devtools } from '@tanstack/devtools-vite'

import { tanstackStart } from '@tanstack/react-start/plugin/vite'

import viteReact from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

const BACKEND = 'http://127.0.0.1:3001'

const config = defineConfig({
  resolve: { tsconfigPaths: true },
  plugins: [devtools(), tailwindcss(), tanstackStart(), viteReact()],
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
