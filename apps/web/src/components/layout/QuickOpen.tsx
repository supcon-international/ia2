import { useEffect, useMemo, useRef, useState } from "react"
import { Cpu, FileCode2, Radio, Search } from "lucide-react"

import { useRuntime } from "@/state/runtime"
import { cn } from "@/lib/utils"

/**
 * Quick-open palette — the real behaviour behind the icon rail's Search
 * icon (and ⌘P). A self-contained overlay that flattens the project into
 * a searchable list (POUs, devices, edges) and opens whatever the user
 * picks, routing through the same `selectPou` / `selectDevice` /
 * `selectEdge` the tree uses. No new backend, no ProjectTree surgery —
 * it reads the tree already in context.
 *
 * Kept deliberately small: substring match, keyboard-driven, Escape to
 * dismiss. It exists so the Search affordance in the rail is genuine,
 * not decorative chrome.
 */

type Entry = {
  kind: "pou" | "device" | "edge"
  id: string // path (pou) or name (device/edge)
  label: string
  hint: string
  open: () => void
}

export function QuickOpen({
  open,
  onClose,
}: {
  open: boolean
  onClose: () => void
}) {
  const { project, selectPou, selectDevice, selectEdge } = useRuntime()
  const [query, setQuery] = useState("")
  const [active, setActive] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)

  const entries = useMemo<Entry[]>(() => {
    if (!project) return []
    const out: Entry[] = []
    for (const f of project.pous) {
      out.push({
        kind: "pou",
        id: f.path,
        label: f.path,
        hint: f.declarations.map((d) => d.type.replace("_", " ")).join(", ") || "empty",
        open: () => void selectPou(f.path),
      })
    }
    for (const d of project.devices) {
      out.push({
        kind: "device",
        id: d.name,
        label: d.name,
        hint: d.protocol,
        open: () => void selectDevice(d.name),
      })
    }
    for (const e of project.edges) {
      out.push({
        kind: "edge",
        id: e.name,
        label: e.name,
        hint: "edge",
        open: () => void selectEdge(e.name),
      })
    }
    return out
  }, [project, selectPou, selectDevice, selectEdge])

  const results = useMemo(() => {
    const q = query.trim().toLowerCase()
    if (!q) return entries.slice(0, 50)
    return entries
      .filter((e) => e.label.toLowerCase().includes(q) || e.hint.toLowerCase().includes(q))
      .slice(0, 50)
  }, [entries, query])

  // Reset + focus each time it opens; clamp the active row as results shrink.
  useEffect(() => {
    if (open) {
      setQuery("")
      setActive(0)
      // Focus after the element paints.
      requestAnimationFrame(() => inputRef.current?.focus())
    }
  }, [open])
  useEffect(() => {
    setActive((a) => Math.min(a, Math.max(0, results.length - 1)))
  }, [results.length])

  if (!open) return null

  const choose = (e: Entry | undefined) => {
    if (!e) return
    e.open()
    onClose()
  }

  return (
    <div
      className="fixed inset-0 z-[720] flex items-start justify-center bg-black/30 pt-[12vh]"
      onClick={onClose}
    >
      <div
        className="w-[520px] max-w-[calc(100vw-96px)] overflow-hidden rounded-lg border border-border bg-popover shadow-2xl"
        onClick={(ev) => ev.stopPropagation()}
      >
        <div className="flex items-center gap-2 border-b border-border px-3">
          <Search className="size-4 shrink-0 text-muted-foreground" />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown") {
                e.preventDefault()
                setActive((a) => Math.min(a + 1, results.length - 1))
              } else if (e.key === "ArrowUp") {
                e.preventDefault()
                setActive((a) => Math.max(a - 1, 0))
              } else if (e.key === "Enter") {
                e.preventDefault()
                choose(results[active])
              } else if (e.key === "Escape") {
                e.preventDefault()
                onClose()
              }
            }}
            placeholder="Search POUs, devices, edges…"
            className="cs-selectable h-11 flex-1 bg-transparent text-[13px] text-foreground outline-none placeholder:text-muted-foreground"
          />
          <kbd className="shrink-0 rounded border border-border bg-muted/60 px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
            esc
          </kbd>
        </div>
        <ul className="max-h-[46vh] overflow-auto py-1">
          {results.length === 0 ? (
            <li className="px-4 py-6 text-center text-[12px] text-muted-foreground">
              No matches
            </li>
          ) : (
            results.map((e, i) => (
              <li key={`${e.kind}:${e.id}`}>
                <button
                  type="button"
                  onMouseEnter={() => setActive(i)}
                  onClick={() => choose(e)}
                  className={cn(
                    "flex w-full items-center gap-2.5 px-3 py-1.5 text-left",
                    i === active ? "bg-accent text-foreground" : "text-muted-foreground",
                  )}
                >
                  <EntryIcon kind={e.kind} />
                  <span className="flex-1 truncate font-mono text-[12px] text-foreground">
                    {e.label}
                  </span>
                  <span className="shrink-0 font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
                    {e.hint}
                  </span>
                </button>
              </li>
            ))
          )}
        </ul>
      </div>
    </div>
  )
}

function EntryIcon({ kind }: { kind: Entry["kind"] }) {
  const cls = "size-3.5 shrink-0 text-muted-foreground"
  if (kind === "device") return <Cpu className={cls} />
  if (kind === "edge") return <Radio className={cls} />
  return <FileCode2 className={cls} />
}
