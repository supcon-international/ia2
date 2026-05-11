import { Play, RotateCcw, Save, Square } from "lucide-react"

import { STEditor } from "@/components/editor/STEditor"
import { useRuntime } from "@/state/runtime"

export function ProgramPane() {
  const {
    currentApp,
    source,
    setSource,
    isDirty,
    saveCurrentApp,
    isRunning,
    run,
    stop,
    diagnostics,
    error,
  } = useRuntime()

  if (!currentApp) {
    return (
      <main className="flex min-h-0 min-w-0 flex-col">
        <div className="flex h-9 items-center border-b border-border px-3 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
          Program
        </div>
        <div className="grid flex-1 place-items-center p-6 text-center text-sm text-muted-foreground">
          Select a POU from the project tree, or create one with the&nbsp;
          <span className="font-mono">+</span> button next to "Applications".
        </div>
      </main>
    )
  }

  return (
    <main className="flex min-h-0 min-w-0 flex-col">
      <div className="flex h-9 items-center justify-between border-b border-border pl-3 pr-2 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        <span className="flex items-center gap-2 truncate normal-case tracking-normal text-foreground">
          <span className="truncate font-mono">{currentApp.name}</span>
          <span className="font-mono text-[9px] uppercase tracking-wider text-muted-foreground">
            {currentApp.kind === "function_block" ? "fb" : "prg"}
          </span>
          {isDirty && (
            <span className="rounded bg-amber-500/15 px-1.5 py-0.5 font-mono text-[9px] uppercase tracking-wider text-amber-700 dark:text-amber-400">
              modified
            </span>
          )}
          {diagnostics.length > 0 && (
            <span className="rounded bg-red-500/15 px-1.5 py-0.5 font-mono text-[9px] uppercase tracking-wider text-red-700 dark:text-red-400">
              {diagnostics.length}{" "}
              {diagnostics.length === 1 ? "issue" : "issues"}
            </span>
          )}
        </span>
        <div className="flex items-center gap-1">
          {isDirty && (
            <>
              <button
                type="button"
                onClick={() => setSource(currentApp.source)}
                title="Revert"
                className="flex items-center gap-1 rounded-md px-2 py-1 text-xs font-medium tracking-normal text-muted-foreground normal-case hover:bg-accent/40"
              >
                <RotateCcw className="size-3" />
                Revert
              </button>
              <button
                type="button"
                onClick={() => void saveCurrentApp()}
                title="Save (Cmd/Ctrl+S)"
                className="flex items-center gap-1 rounded-md px-2 py-1 text-xs font-medium tracking-normal text-foreground normal-case hover:bg-accent/40"
              >
                <Save className="size-3" />
                Save
              </button>
            </>
          )}
          {isRunning ? (
            <button
              type="button"
              onClick={stop}
              className="flex items-center gap-1 rounded-md px-2 py-1 text-xs font-medium tracking-normal text-red-700 normal-case hover:bg-red-100 dark:text-red-400 dark:hover:bg-red-950/50"
            >
              <Square className="size-3 fill-current" />
              Stop
            </button>
          ) : (
            <button
              type="button"
              onClick={run}
              className="flex items-center gap-1 rounded-md px-2 py-1 text-xs font-medium tracking-normal text-emerald-700 normal-case hover:bg-emerald-100 dark:text-emerald-400 dark:hover:bg-emerald-950/50"
            >
              <Play className="size-3 fill-current" />
              Run
            </button>
          )}
        </div>
      </div>
      {error && (
        <div className="border-b border-red-200 bg-red-50 px-3 py-2 text-xs text-red-700 dark:border-red-900 dark:bg-red-950/40 dark:text-red-400">
          {error}
        </div>
      )}
      <div className="flex-1 min-h-0">
        <STEditor
          value={source}
          onChange={setSource}
          diagnostics={diagnostics}
        />
      </div>
    </main>
  )
}
