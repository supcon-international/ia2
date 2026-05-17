// SPA entry point. Replaces the TanStack Start `shellComponent` flow
// because we ship as a static dist served by either Vite (dev) or our
// own axum binary (production + desktop). SSR adds zero value to an
// offline desktop app loaded over localhost and costs us a Node
// runtime in the supervision tree — not worth it.

import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { RouterProvider } from '@tanstack/react-router'

import { getRouter } from './router'
import './styles.css'

const router = getRouter()

// Font-fallback prewarm: WebKit/Chromium pay a one-time stutter the
// first time they substitute a fallback font (CJK, emoji). For a PLC
// IDE shipping demos with Chinese POU names and emoji in some demo
// logs, that stutter is visible on first paint. Cf. references/03-
// webview-survival.md § A.9 — render once into an off-screen span so
// Core Text / DirectWrite caches the glyph tables before the first
// real frame.
function prewarmFontFallbacks() {
  const span = document.createElement('span')
  span.setAttribute('aria-hidden', 'true')
  span.style.cssText =
    'position:absolute;left:-9999px;top:0;opacity:0;pointer-events:none'
  span.textContent = '😀✨📦 中文 日本語 한국어 ∑∫√ ✓✗'
  document.body.appendChild(span)
  // Force layout, then remove after two frames so the font is fully
  // registered in the cache. Single rAF can race the paint pass.
  void span.getBoundingClientRect()
  requestAnimationFrame(() =>
    requestAnimationFrame(() => span.remove()),
  )
}

const container = document.getElementById('root')
if (!container) {
  throw new Error('#root not found in index.html')
}

createRoot(container).render(
  <StrictMode>
    <RouterProvider router={router} />
  </StrictMode>,
)

prewarmFontFallbacks()
