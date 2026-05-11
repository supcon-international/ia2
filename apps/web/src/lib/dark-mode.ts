import { useEffect, useState } from "react"

/**
 * Tracks the user's preferred color scheme and applies/removes the `.dark`
 * class on `<html>` so shadcn's CSS variables flip in sync. Returns the
 * current resolved theme. Safe to call from multiple components — they all
 * see the same matchMedia event source.
 */
export function useDarkMode(): "dark" | "light" {
  const [theme, setTheme] = useState<"dark" | "light">("light")

  useEffect(() => {
    if (typeof window === "undefined") return
    const mq = window.matchMedia("(prefers-color-scheme: dark)")
    const apply = (dark: boolean) => {
      setTheme(dark ? "dark" : "light")
      document.documentElement.classList.toggle("dark", dark)
    }
    apply(mq.matches)
    const handler = (e: MediaQueryListEvent) => apply(e.matches)
    mq.addEventListener("change", handler)
    return () => mq.removeEventListener("change", handler)
  }, [])

  return theme
}
