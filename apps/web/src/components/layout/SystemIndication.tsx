import { Moon, Sun } from "lucide-react"

import { useThemeToggle } from "@/lib/dark-mode"
import { useRuntime } from "@/state/runtime"
import { useConnected } from "@/state/live-feed"
import { cn } from "@/lib/utils"

export function SystemIndication() {
  const { isRunning } = useRuntime()
  const connected = useConnected()
  const { theme, toggle } = useThemeToggle()

  const { label, dot, text } = connected
    ? isRunning
      ? {
          label: "Running",
          // The one and only "actively running" green — uses the FX
          // Green token rather than a hardcoded emerald so the brand
          // accent stays in one place.
          dot: "bg-highlight",
          text: "text-foreground",
        }
      : { label: "Idle", dot: "bg-muted-foreground/50", text: "text-muted-foreground" }
    : {
        label: "Server unreachable",
        dot: "bg-destructive/60",
        text: "text-muted-foreground",
      }

  return (
    <div className="flex items-end justify-between gap-2 border-t border-border px-3 py-2">
      <div className="min-w-0 flex-1">
        <div className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
          System indication
        </div>
        <div className="mt-1 flex items-center gap-2 text-xs">
          <span className={cn("size-2 rounded-full", dot)} />
          <span className={cn("truncate", text)}>{label}</span>
        </div>
      </div>
      <button
        type="button"
        onClick={toggle}
        title={theme === "dark" ? "Switch to light mode" : "Switch to dark mode"}
        aria-label="Toggle colour theme"
        className="flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent/40 hover:text-foreground"
      >
        {theme === "dark" ? <Sun className="size-3.5" /> : <Moon className="size-3.5" />}
      </button>
    </div>
  )
}
