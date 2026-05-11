import { useRuntime } from "@/state/runtime"

export function MonitorPane() {
  const { lastSnapshot, isRunning } = useRuntime()
  const vars = lastSnapshot?.vars ?? []

  return (
    <section className="flex min-h-0 min-w-0 flex-col border-t border-border bg-muted/20">
      <div className="flex h-7 items-center justify-between border-b border-border px-3 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        <span>Monitor</span>
        {lastSnapshot && (
          <span className="font-mono normal-case tracking-normal">
            scan #{Number(lastSnapshot.scan_count)}
          </span>
        )}
      </div>
      <div className="flex-1 overflow-auto">
        {!isRunning ? (
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
          <ul className="divide-y divide-border">
            {vars.map((v) => (
              <li
                key={v.name}
                className="flex items-baseline gap-2 px-3 py-2"
              >
                <span className="flex-1 truncate font-mono text-sm">
                  {v.name}
                </span>
                {v.type_name && (
                  <span className="font-mono text-[10px] text-muted-foreground">
                    {v.type_name}
                  </span>
                )}
                <span className="font-mono text-sm tabular-nums">
                  {v.value}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  )
}
