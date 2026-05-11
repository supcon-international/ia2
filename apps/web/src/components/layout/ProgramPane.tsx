import { Play, RotateCcw, Square } from "lucide-react"
import { STEditor } from "@/components/editor/STEditor"
import { useRuntime } from "@/state/runtime"

export function ProgramPane() {
  const {
    programInfo,
    source,
    setSource,
    isDirty,
    isRunning,
    run,
    stop,
    error,
  } = useRuntime()

  return (
    <main className="flex min-h-0 min-w-0 flex-col">
      <div className="flex h-9 items-center justify-between border-b border-border pl-3 pr-2 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        <span className="flex items-center gap-2 truncate">
          <span className="truncate">{programInfo?.name ?? "Program"}</span>
          {isDirty && (
            <span className="rounded bg-amber-500/15 px-1.5 py-0.5 font-mono text-[9px] normal-case tracking-normal text-amber-700 dark:text-amber-400">
              modified
            </span>
          )}
        </span>
        <div className="flex items-center gap-1">
          {isDirty && programInfo && (
            <button
              type="button"
              onClick={() => setSource(programInfo.source)}
              title="Revert to original"
              className="flex items-center gap-1 rounded-md px-2 py-1 text-xs font-medium tracking-normal text-muted-foreground normal-case hover:bg-accent/40"
            >
              <RotateCcw className="size-3" />
              Revert
            </button>
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
        <STEditor value={source} onChange={setSource} />
      </div>
    </main>
  )
}
