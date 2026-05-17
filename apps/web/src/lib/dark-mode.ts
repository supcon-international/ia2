import { useSyncExternalStore } from "react"

/**
 * Theme controller.
 *
 * The previous version followed `prefers-color-scheme` so a user on a
 * dark-mode macOS would land in the IDE's dark theme by default. The
 * design language explicitly prescribes **white / light gray / near-black
 * as the canonical visual direction** (DESIGN.md §2), so we default to
 * light regardless of the OS, and let the user override via the toggle
 * in the workbench header.
 *
 * Choice is persisted under `ia2.theme` so the next reload
 * stays in the user's mode of choice. SSR-safe (no window touch during
 * `useState` initial value). One-time migration: if the legacy
 * `controlsoftware.theme` key exists, copy it across so users don't
 * lose their dark-mode preference across the rename.
 */

const STORAGE_KEY = "ia2.theme"
const LEGACY_STORAGE_KEY = "controlsoftware.theme"
type Theme = "light" | "dark"

/** Read once at module load so the very first paint matches the user's
 * persisted choice (no light → dark flash on dark-mode users). Runs
 * synchronously in the bundle's top-level. */
if (typeof window !== "undefined") {
  try {
    let persisted = window.localStorage.getItem(STORAGE_KEY) as Theme | null
    if (persisted === null) {
      const legacy = window.localStorage.getItem(LEGACY_STORAGE_KEY) as Theme | null
      if (legacy === "dark" || legacy === "light") {
        window.localStorage.setItem(STORAGE_KEY, legacy)
        window.localStorage.removeItem(LEGACY_STORAGE_KEY)
        persisted = legacy
      }
    }
    if (persisted === "dark") {
      document.documentElement.classList.add("dark")
    } else {
      document.documentElement.classList.remove("dark")
    }
  } catch {
    /* localStorage unavailable in some sandboxes — fall through to light. */
  }
}

// ---- Subscribers ----------------------------------------------------------
// We deliberately don't use Context: the toggle is a single switch shared
// by every component that needs to know the theme (Workbench, STEditor).
// A tiny pub-sub via `useSyncExternalStore` is enough and avoids prop
// drilling.
const listeners = new Set<() => void>()
function subscribe(cb: () => void) {
  listeners.add(cb)
  return () => listeners.delete(cb)
}
function snapshot(): Theme {
  if (typeof document === "undefined") return "light"
  return document.documentElement.classList.contains("dark") ? "dark" : "light"
}

/** Apply a theme + persist + notify subscribers. Centralised so we can
 *  tweak persistence later (e.g. swap to cookies for SSR) in one place. */
export function setTheme(theme: Theme) {
  if (typeof document === "undefined") return
  document.documentElement.classList.toggle("dark", theme === "dark")
  try {
    window.localStorage.setItem(STORAGE_KEY, theme)
  } catch {
    /* persistence is best-effort */
  }
  listeners.forEach((cb) => cb())
}

/** Returns the currently applied theme and re-renders the caller when it
 *  changes. Pure read — to mutate, call `setTheme`. */
export function useDarkMode(): Theme {
  return useSyncExternalStore(subscribe, snapshot, () => "light")
}

/** Bind a setter + current value for callers that want both in one go
 *  (e.g. the toggle button). */
export function useThemeToggle(): {
  theme: Theme
  setTheme: (t: Theme) => void
  toggle: () => void
} {
  const theme = useDarkMode()
  return {
    theme,
    setTheme,
    toggle: () => setTheme(theme === "dark" ? "light" : "dark"),
  }
}

