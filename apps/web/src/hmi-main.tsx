// Standalone HMI panel entry (hmi.html). Mounts the operator panel
// against the edge runtime's own surface — /api/hmi for documents,
// /events for live values, /write for actions. No router, no IDE state.

import { StrictMode } from "react"
import { createRoot } from "react-dom/client"

import { HmiStandalone } from "./components/hmi/HmiStandalone"
import "./lib/dark-mode" // applies the persisted theme class pre-paint
import "./styles.css"

const container = document.getElementById("root")
if (!container) {
  throw new Error("#root not found in hmi.html")
}
createRoot(container).render(
  <StrictMode>
    <HmiStandalone />
  </StrictMode>,
)
