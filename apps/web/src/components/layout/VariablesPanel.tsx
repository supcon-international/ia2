import { ArrowLeftRight, ArrowRight, Link2Off } from "lucide-react"
import { useEffect, useState } from "react"

import { fetchPouVariables } from "@/lib/api"
import { useRuntime } from "@/state/runtime"
import type { Mapping } from "@/types/generated/Mapping"
import type { VariableInfo } from "@/types/generated/VariableInfo"

/**
 * Right-side panel inside ProgramPane. Lists every variable declared in the
 * current POU, with its iomap bindings rendered as clickable pills. Clicking
 * a pill jumps to the device editor and selects the bound device so the
 * user can see/edit the channel without re-navigating.
 *
 * Read-only — actual binding edits happen in the IO Mapping pane or in the
 * Device editor's "Linked to" column.
 */
export function VariablesPanel() {
  const { currentPou, iomap, project, selectDevice } = useRuntime()
  const [vars, setVars] = useState<VariableInfo[]>([])
  const [loading, setLoading] = useState(false)

  // Refetch whenever the POU changes — variable lists are cheap to recompute
  // and source-typed (so an edit-in-progress changes the list).
  useEffect(() => {
    if (!currentPou) {
      setVars([])
      return
    }
    let cancelled = false
    setLoading(true)
    fetchPouVariables(currentPou.path)
      .then((vs) => {
        if (!cancelled) setVars(vs)
      })
      .catch(() => {
        if (!cancelled) setVars([])
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [currentPou])

  // Build a map: variable name → mappings within the current POU.
  const bindingsByVar = new Map<string, Mapping[]>()
  if (currentPou) {
    for (const m of iomap.mappings) {
      if (m.application !== currentPou.path) continue
      const arr = bindingsByVar.get(m.variable) ?? []
      arr.push(m)
      bindingsByVar.set(m.variable, arr)
    }
  }

  // Devices known to the project — we shade unknown-device bindings (stale
  // mappings) so the user can spot misalignment with the device tree.
  const knownDevices = new Set(project?.devices.map((d) => d.name) ?? [])

  return (
    <aside className="flex h-full min-h-0 w-full flex-col border-l border-border bg-background/40">
      <div className="flex h-9 items-center border-b border-border px-3 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        Variables
        <span className="ml-2 font-mono text-[10px] tracking-normal opacity-60">
          {vars.length}
        </span>
      </div>
      <div className="min-h-0 flex-1 overflow-auto">
        {!currentPou ? (
          <Empty>Select a POU.</Empty>
        ) : loading && vars.length === 0 ? (
          <Empty>Loading…</Empty>
        ) : vars.length === 0 ? (
          <Empty>No variables declared.</Empty>
        ) : (
          <ul className="divide-y divide-border">
            {vars.map((v) => {
              const bindings = bindingsByVar.get(v.name) ?? []
              return (
                <li key={v.name} className="px-3 py-2">
                  <div className="flex items-baseline justify-between gap-2">
                    <span className="truncate font-mono text-[13px] text-foreground">
                      {v.name}
                    </span>
                    <span className="shrink-0 font-mono text-[10px] uppercase text-muted-foreground">
                      {v.direction}
                    </span>
                  </div>
                  <div className="font-mono text-[10px] text-muted-foreground">
                    {v.type_name}
                  </div>
                  {bindings.length === 0 ? (
                    <div className="mt-1 inline-flex items-center gap-1 text-[10px] text-muted-foreground/60">
                      <Link2Off className="size-3" />
                      unbound
                    </div>
                  ) : (
                    <ul className="mt-1 space-y-1">
                      {bindings.map((m, i) => {
                        const unknown = !knownDevices.has(m.device)
                        return (
                          <li key={i}>
                            <button
                              type="button"
                              onClick={() => void selectDevice(m.device)}
                              disabled={unknown}
                              title={
                                unknown
                                  ? `Device "${m.device}" not in project`
                                  : `Jump to ${m.device}`
                              }
                              className={
                                "inline-flex max-w-full items-center gap-1 rounded-md border border-border bg-muted/40 px-1.5 py-0.5 text-left font-mono text-[11px] " +
                                (unknown
                                  ? "cursor-not-allowed opacity-50"
                                  : "hover:bg-accent/40")
                              }
                            >
                              {m.direction === "input" ? (
                                <ArrowLeftRight className="size-3 shrink-0 text-sky-700 dark:text-sky-400" />
                              ) : (
                                <ArrowRight className="size-3 shrink-0 text-emerald-700 dark:text-emerald-400" />
                              )}
                              <span className="truncate">
                                {m.device}.{m.channel}
                              </span>
                            </button>
                          </li>
                        )
                      })}
                    </ul>
                  )}
                </li>
              )
            })}
          </ul>
        )}
      </div>
    </aside>
  )
}

function Empty({ children }: { children: React.ReactNode }) {
  return (
    <div className="grid h-full place-items-center px-4 text-center text-[11px] italic text-muted-foreground">
      {children}
    </div>
  )
}
