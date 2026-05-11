import { Play, Square } from "lucide-react"
import { useRuntime } from "@/state/runtime"

export function ProgramPane() {
  const { programInfo, isRunning, run, stop, error } = useRuntime()

  return (
    <main className="flex min-w-0 flex-col border-r border-border">
      <div className="flex h-9 items-center justify-between border-b border-border pl-3 pr-2 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        <span className="truncate">{programInfo?.name ?? "Program"}</span>
        <div className="flex items-center gap-1">
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
      <pre className="flex-1 overflow-auto whitespace-pre p-4 font-mono text-xs leading-relaxed">
        {programInfo?.source ?? "Loading…"}
      </pre>
    </main>
  )
}
