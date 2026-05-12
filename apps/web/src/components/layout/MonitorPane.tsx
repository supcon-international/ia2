import { Pin } from "lucide-react"
import { useEffect, useMemo, useRef, useState } from "react"

import { Sparkline } from "@/components/charts/Sparkline"
import { TrendChart } from "@/components/charts/TrendChart"
import { cn } from "@/lib/utils"
import {
  colorFor,
  isBoolType,
  parseVarValue,
  pushHistory,
} from "@/lib/var-history"
import { useRuntime } from "@/state/runtime"

export function MonitorPane() {
  const { lastSnapshot, isRunning, currentApp } = useRuntime()

  // History buffers (mutated in place; re-rendered via a tick counter).
  const historyRef = useRef<Map<string, number[]>>(new Map())
  const typeRef = useRef<Map<string, string>>(new Map())
  const [, setTick] = useState(0)
  const [pinned, setPinned] = useState<Set<string>>(new Set())

  // Drop history + pins when the user switches POU — old vars aren't
  // relevant to the new one.
  useEffect(() => {
    historyRef.current.clear()
    typeRef.current.clear()
    setPinned(new Set())
    setTick((t) => t + 1)
  }, [currentApp?.name])

  // Ingest every snapshot into the per-variable history.
  useEffect(() => {
    if (!lastSnapshot) return
    for (const v of lastSnapshot.vars) {
      typeRef.current.set(v.name, v.type_name)
      let arr = historyRef.current.get(v.name)
      if (!arr) {
        arr = []
        historyRef.current.set(v.name, arr)
      }
      pushHistory(arr, parseVarValue(v))
    }
    setTick((t) => t + 1)
  }, [lastSnapshot])

  const togglePin = (name: string) => {
    setPinned((prev) => {
      const next = new Set(prev)
      if (next.has(name)) next.delete(name)
      else next.add(name)
      return next
    })
  }

  // Build series for the pinned trend chart.
  const pinnedList = useMemo(() => Array.from(pinned), [pinned])
  const pinnedSeries = pinnedList.map((name, idx) => ({
    name,
    values: historyRef.current.get(name) ?? [],
    color: colorFor(idx),
    binary: isBoolType(typeRef.current.get(name) ?? ""),
  }))
  const colorByName: Record<string, string> = Object.fromEntries(
    pinnedList.map((name, idx) => [name, colorFor(idx)]),
  )

  const vars = lastSnapshot?.vars ?? []
  const stale = !!lastSnapshot && !isRunning

  return (
    <section className="flex h-full min-h-0 min-w-0 flex-col border-t border-border bg-muted/20">
      <div className="flex h-7 items-center justify-between border-b border-border px-3 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        <span>Monitor</span>
        {lastSnapshot && (
          <span
            className={cn(
              "font-mono normal-case tracking-normal",
              stale ? "text-muted-foreground" : "text-foreground",
            )}
          >
            {stale && "(last) "}scan #{Number(lastSnapshot.scan_count)}
          </span>
        )}
      </div>

      {pinnedSeries.length > 0 && (
        <div className="border-b border-border bg-background/40 px-3 py-2">
          <TrendChart series={pinnedSeries} />
        </div>
      )}

      <div className="flex-1 overflow-auto">
        {!lastSnapshot ? (
          <div className="grid h-full place-items-center p-4 text-center text-xs text-muted-foreground">
            Click&nbsp;
            <span className="font-mono text-emerald-700 dark:text-emerald-400">
              Run
            </span>
            &nbsp;to start the program.
          </div>
        ) : vars.length === 0 ? (
          <div className="grid h-full place-items-center p-4 text-xs text-muted-foreground">
            Waiting for first snapshot…
          </div>
        ) : (
          <ul className="divide-y divide-border/60">
            {vars.map((v, i) => {
              const history = historyRef.current.get(v.name) ?? []
              const binary = isBoolType(v.type_name)
              const isPinned = pinned.has(v.name)
              const sparkColor = colorByName[v.name]
              const defaultColor = binary
                ? "text-emerald-600 dark:text-emerald-400"
                : "text-sky-600 dark:text-sky-400"
              return (
                <li
                  key={`${i}:${v.name}`}
                  className={cn(
                    "flex items-center gap-2 px-2 py-1",
                    stale && "opacity-60",
                  )}
                >
                  <button
                    type="button"
                    onClick={() => togglePin(v.name)}
                    className={cn(
                      "shrink-0 rounded p-0.5 transition-colors",
                      isPinned
                        ? "text-foreground"
                        : "text-muted-foreground/30 hover:text-muted-foreground",
                    )}
                    title={isPinned ? "Unpin from trend" : "Pin to trend"}
                  >
                    <Pin
                      className={cn(
                        "size-3",
                        isPinned && "fill-current rotate-45",
                      )}
                    />
                  </button>
                  <span className="w-24 shrink-0 truncate font-mono text-xs">
                    {v.name}
                  </span>
                  <span
                    className={cn("flex-1 min-w-0", !sparkColor && defaultColor)}
                    style={sparkColor ? { color: sparkColor } : undefined}
                  >
                    <Sparkline
                      values={history}
                      binary={binary}
                      width={120}
                      height={18}
                      filled={!binary}
                    />
                  </span>
                  {v.type_name && (
                    <span className="hidden font-mono text-[9px] text-muted-foreground sm:inline">
                      {v.type_name}
                    </span>
                  )}
                  <span className="w-20 shrink-0 text-right font-mono text-xs tabular-nums">
                    {v.value}
                  </span>
                </li>
              )
            })}
          </ul>
        )}
      </div>
    </section>
  )
}
