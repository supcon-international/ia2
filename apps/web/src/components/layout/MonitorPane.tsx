import { Lock, Pause, Pin, Play, StepForward, Unlock } from "lucide-react"
import { useCallback, useEffect, useMemo, useRef, useState } from "react"

import { Sparkline } from "@/components/charts/Sparkline"
import { TrendChart } from "@/components/charts/TrendChart"
import { cn } from "@/lib/utils"
import {
  forceVariable,
  pauseRuntime,
  resumeRuntime,
  stepRuntime,
  unforceVariable,
  writeVariable,
} from "@/lib/api"
import {
  classifyType,
  colorFor,
  isBoolType,
  parseVarValue,
  prettyTime,
  pushHistory,
  stripHexPrefix,
  type VarCategory,
} from "@/lib/var-history"
import { useRuntime, type RunningInfo } from "@/state/runtime"
import type { VarValue } from "@/types/generated/VarValue"

export function MonitorPane() {
  const { lastSnapshot, isRunning, currentPou, running, attached } = useRuntime()
  // Variable writes go through the local bridge. When attached to a
  // remote edge runtime we don't (yet) proxy writes — disable the
  // controls in that case to avoid silently losing the user's input.
  const canWrite = isRunning && !attached

  // Debug control state. We poll /api/runtime/status on a 1s timer
  // when the program is running so the toolbar reflects external
  // changes (e.g. `cs runtime pause` from a shell). Optimistic local
  // updates keep clicks feeling instant.
  const [mode, setMode] = useState<"running" | "paused" | "step">("running")
  const [forces, setForces] = useState<Set<string>>(new Set())
  useEffect(() => {
    if (!isRunning) return
    let cancelled = false
    const tick = async () => {
      try {
        const r = await fetch("/api/runtime/status")
        if (!r.ok) return
        const data = await r.json()
        if (cancelled) return
        const m = data?.mode?.kind as string | undefined
        if (m === "running" || m === "paused" || m === "step") setMode(m)
        const fs: Array<{ name: string }> = data?.forces ?? []
        setForces(new Set(fs.map((f) => f.name)))
      } catch {
        /* ignore */
      }
    }
    void tick()
    const id = setInterval(tick, 1000)
    return () => {
      cancelled = true
      clearInterval(id)
    }
  }, [isRunning])

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
  }, [currentPou?.path])

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

  // Optimistic-update wrappers: flip local state immediately for
  // responsiveness, then issue the API call. The 1s status poll
  // reconciles any drift from external changes.
  const onPause = useCallback(async () => {
    setMode("paused")
    try { await pauseRuntime() } catch { /* status poll will re-sync */ }
  }, [])
  const onResume = useCallback(async () => {
    setMode("running")
    try { await resumeRuntime() } catch { /* */ }
  }, [])
  const onStep = useCallback(async () => {
    setMode("step")
    try { await stepRuntime(1) } catch { /* */ }
  }, [])

  const onToggleForce = useCallback(
    async (v: VarValue) => {
      if (forces.has(v.name)) {
        setForces((p) => {
          const n = new Set(p)
          n.delete(v.name)
          return n
        })
        try { await unforceVariable(v.name) } catch { /* */ }
      } else {
        // Pin to the *current* value — operator's intent is usually
        // "lock this where it is now" rather than picking a fresh
        // value. They can change it via the inline numeric editor /
        // BOOL toggle once forced.
        setForces((p) => new Set(p).add(v.name))
        try {
          const cat = classifyType(v.type_name)
          let i32 = 0
          if (cat === "bool") {
            i32 = v.value === "TRUE" ? 1 : 0
          } else if (cat === "numeric") {
            i32 = parseInt(v.value, 10) || 0
          }
          await forceVariable(v.name, i32, v.type_name)
        } catch { /* */ }
      }
    },
    [forces],
  )

  return (
    <section className="flex h-full min-h-0 min-w-0 flex-col border-t border-border bg-muted/20">
      <div className="flex h-7 items-center justify-between gap-3 border-b border-border px-3 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        <span className="flex shrink-0 items-center gap-2">
          <span>Monitor</span>
          <RunningPill running={running} isRunning={isRunning} />
          {isRunning && !attached && (
            <DebugToolbar
              mode={mode}
              onPause={onPause}
              onResume={onResume}
              onStep={onStep}
            />
          )}
        </span>
        {lastSnapshot && (
          <span
            className={cn(
              "shrink-0 font-mono normal-case tracking-normal",
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
          <div className="flex h-full items-center justify-center p-4 text-xs text-muted-foreground">
            <span>
              Click{" "}
              <span className="font-mono text-highlight">Run</span>{" "}
              to start the program.
            </span>
          </div>
        ) : vars.length === 0 ? (
          <div className="flex h-full items-center justify-center p-4 text-xs text-muted-foreground">
            Waiting for first snapshot…
          </div>
        ) : (
          <ul className="divide-y divide-border/60">
            {vars.map((v, i) => (
              <VarRow
                key={`${i}:${v.name}`}
                v={v}
                history={historyRef.current.get(v.name) ?? []}
                isPinned={pinned.has(v.name)}
                sparkColor={colorByName[v.name]}
                onPin={togglePin}
                stale={stale}
                canWrite={canWrite}
                forced={forces.has(v.name)}
                onToggleForce={onToggleForce}
              />
            ))}
          </ul>
        )}
      </div>
    </section>
  )
}

// ============================================================
//   RunningPill — header chip that labels WHICH program(s) the
//   variables below belong to. Three variants:
//
//     - isolated  (ProgramPane Run): one PROGRAM name in FX Green
//     - scheduled (TasksPane Run):   list of PROGRAM names
//     - remote    (attached to edge): edge alias + "remote" tag
//
//   When `running` is null but `isRunning` is true (race window
//   between SSE `started` and our local state catching up), fall
//   back to a neutral "running" tag so the header doesn't lie.
// ============================================================

function RunningPill({
  running,
  isRunning,
}: {
  running: RunningInfo
  isRunning: boolean
}) {
  if (!running) {
    if (isRunning) {
      return <Tag color="highlight">running</Tag>
    }
    return null
  }
  if (running.kind === "isolated") {
    return (
      <Tag color="highlight" title={`Running ad-hoc from ${running.filePath}.st`}>
        <span className="font-mono">{running.program}</span>
        <span className="opacity-60">isolated</span>
      </Tag>
    )
  }
  if (running.kind === "scheduled") {
    const names = running.programs
    const label =
      names.length === 0
        ? "(empty schedule)"
        : names.length <= 3
          ? names.join(", ")
          : `${names.slice(0, 2).join(", ")} +${names.length - 2}`
    return (
      <Tag
        // Empty schedule = the scan loop is up but nothing executes; show it
        // muted, not highlighted, so it doesn't read as active program work.
        color={names.length === 0 ? "muted" : "highlight"}
        title={
          names.length > 0
            ? `Running ${names.length} PROGRAM instance${names.length > 1 ? "s" : ""}: ${names.join(", ")}`
            : "tasks.toml has no PROGRAM bindings — nothing is executing"
        }
      >
        <span className="font-mono">{label}</span>
        <span className="opacity-60">scheduled</span>
      </Tag>
    )
  }
  // remote
  return (
    <Tag color="muted" title={`Attached to edge ${running.edge}`}>
      <span className="font-mono">{running.edge}</span>
      <span className="opacity-60">remote</span>
    </Tag>
  )
}

/**
 * Inline pause / step / resume control + mode badge for the Monitor
 * header. Three icon buttons total — operators recognise the play /
 * pause / step pattern from every media UI they've ever used. The
 * mode badge ("PAUSED" / "STEP") shows up only when off the default
 * Running state, so the toolbar stays quiet during normal operation.
 */
function DebugToolbar({
  mode,
  onPause,
  onResume,
  onStep,
}: {
  mode: "running" | "paused" | "step"
  onPause: () => void
  onResume: () => void
  onStep: () => void
}) {
  return (
    <span className="ml-2 flex items-center gap-0.5">
      {mode === "running" ? (
        <button
          type="button"
          onClick={onPause}
          title="Pause scan loop (freeze IO + program)"
          className="rounded p-0.5 text-muted-foreground hover:bg-accent/40 hover:text-foreground"
        >
          <Pause className="size-3" />
        </button>
      ) : (
        <button
          type="button"
          onClick={onResume}
          title="Resume continuous scanning"
          className="rounded p-0.5 text-highlight hover:bg-highlight/15"
        >
          <Play className="size-3" />
        </button>
      )}
      <button
        type="button"
        onClick={onStep}
        title="Step one scan cycle (auto-pause after)"
        className="rounded p-0.5 text-muted-foreground hover:bg-accent/40 hover:text-foreground"
      >
        <StepForward className="size-3" />
      </button>
      {mode !== "running" && (
        <Tag color="muted" title={`scan mode: ${mode}`}>
          {mode}
        </Tag>
      )}
    </span>
  )
}

function Tag({
  color,
  title,
  children,
}: {
  color: "highlight" | "muted"
  title?: string
  children: React.ReactNode
}) {
  return (
    <span
      title={title}
      className={cn(
        "inline-flex items-center gap-1.5 rounded px-1.5 py-0.5 font-medium normal-case tracking-normal",
        color === "highlight"
          ? "bg-highlight/15 text-highlight"
          : "border border-border bg-muted/50 text-muted-foreground",
      )}
    >
      {children}
    </span>
  )
}

// ============================================================
//   Per-variable row — branches on the type category so each
//   IEC 61131-3 family gets a renderer that fits how an operator
//   actually reads it. Numerics trend, booleans flip, time scales
//   to seconds, bit masks render hex. FB instances (PID, etc.)
//   are scratch storage so we collapse them to a quiet "instance"
//   label rather than showing meaningless byte offsets.
// ============================================================

interface VarRowProps {
  v: VarValue
  history: number[]
  isPinned: boolean
  sparkColor: string | undefined
  onPin: (name: string) => void
  stale: boolean
  canWrite: boolean
  /** Whether this variable is currently forced (pinned across scans). */
  forced: boolean
  /** Toggle the force state for this variable. */
  onToggleForce: (v: VarValue) => void
}

function VarRow({
  v,
  history,
  isPinned,
  sparkColor,
  onPin,
  stale,
  canWrite,
  forced,
  onToggleForce,
}: VarRowProps) {
  const cat: VarCategory = classifyType(v.type_name)
  const trendable = cat === "numeric" || cat === "bool" || cat === "bits"

  return (
    <li
      className={cn(
        "flex items-center gap-2 px-2 py-0.5",
        stale && "opacity-60",
      )}
    >
      {trendable ? (
        <button
          type="button"
          onClick={() => onPin(v.name)}
          className={cn(
            "shrink-0 rounded p-0.5 transition-colors",
            isPinned
              ? "text-foreground"
              : "text-muted-foreground/30 hover:text-muted-foreground",
          )}
          title={isPinned ? "Unpin from trend" : "Pin to trend"}
        >
          <Pin
            className={cn("size-3", isPinned && "fill-current rotate-45")}
          />
        </button>
      ) : (
        // Reserve the same width so name columns line up across categories.
        <span className="size-4 shrink-0" />
      )}

      <span className="w-24 shrink-0 truncate font-mono text-xs">{v.name}</span>

      <span className="block h-4 flex-1 min-w-0">
        <CategoryVisual cat={cat} v={v} history={history} sparkColor={sparkColor} />
      </span>

      {v.type_name && (
        <span className="hidden font-mono text-[9px] text-muted-foreground sm:inline">
          {v.type_name}
        </span>
      )}

      {canWrite && (cat === "bool" || cat === "numeric" || cat === "bits") && (
        <button
          type="button"
          onClick={() => onToggleForce(v)}
          className={cn(
            "shrink-0 rounded p-0.5 transition-colors",
            forced
              ? "text-destructive hover:text-destructive/80"
              : "text-muted-foreground/30 hover:text-muted-foreground",
          )}
          title={
            forced
              ? `Unforce ${v.name} (resume normal program-driven behaviour)`
              : `Force ${v.name} = current value (pin across scans)`
          }
        >
          {forced ? <Lock className="size-3" /> : <Unlock className="size-3" />}
        </button>
      )}
      {/* Reserve width so rows align even when the force button is
          hidden (read-only categories or remote attach mode). */}
      {!(canWrite && (cat === "bool" || cat === "numeric" || cat === "bits")) && (
        <span className="size-4 shrink-0" />
      )}

      <ValueCell v={v} cat={cat} canWrite={canWrite} />
    </li>
  )
}

/** Right-hand value cell. Interactive when the program is running
 *  (clickable BOOL toggle / inline numeric input that writes via
 *  `/api/runtime/variables`). Falls back to plain text otherwise.
 *  This is what turns the Monitor into a no-code HMI for driving an
 *  LD POU during dev — without a real plant simulator, the operator
 *  IS the simulator. */
function ValueCell({
  v,
  cat,
  canWrite,
}: {
  v: VarValue
  cat: VarCategory
  canWrite: boolean
}) {
  if (canWrite && cat === "bool") {
    const on = v.value === "TRUE"
    return (
      <button
        type="button"
        onClick={() => {
          void writeVariable(v.name, on ? 0 : 1, "BOOL").catch(() => {
            /* swallow — UI will refresh from next SSE snapshot */
          })
        }}
        className={cn(
          "w-20 shrink-0 rounded px-1 text-right font-mono text-xs tabular-nums transition-colors",
          on
            ? "bg-highlight/15 text-highlight hover:bg-highlight/25"
            : "text-muted-foreground hover:bg-accent/40 hover:text-foreground",
        )}
        title="Click to toggle"
      >
        {on ? "on" : "off"}
      </button>
    )
  }
  if (canWrite && cat === "numeric") {
    return <NumericEditor name={v.name} typeName={v.type_name} value={v.value} />
  }
  return (
    <span
      className={cn(
        "w-20 shrink-0 text-right font-mono text-xs tabular-nums",
        cat === "fb" && "text-muted-foreground/50",
      )}
    >
      {renderValue(cat, v)}
    </span>
  )
}

/** Numeric editor — committed on blur or Enter, reverts on Escape.
 *  Holds a local draft so the SSE snapshots arriving between
 *  keystrokes don't blow away mid-typed values. The `typeName` is
 *  threaded so writeVariable() can do the right encoding (REAL → f32
 *  bit pattern, integers → truncated i32). */
function NumericEditor({
  name,
  typeName,
  value,
}: {
  name: string
  typeName: string
  value: string
}) {
  const [draft, setDraft] = useState(value)
  const [editing, setEditing] = useState(false)
  useEffect(() => {
    if (!editing) setDraft(value)
  }, [value, editing])
  const commit = () => {
    setEditing(false)
    const parsed = parseFloat(draft)
    if (!Number.isFinite(parsed)) {
      setDraft(value)
      return
    }
    void writeVariable(name, parsed, typeName).catch(() => {
      setDraft(value)
    })
  }
  return (
    <input
      type="text"
      value={draft}
      onChange={(e) => {
        setDraft(e.target.value)
        setEditing(true)
      }}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit()
        else if (e.key === "Escape") {
          setDraft(value)
          setEditing(false)
          ;(e.target as HTMLInputElement).blur()
        }
      }}
      className={cn(
        "w-20 shrink-0 rounded bg-transparent px-1 text-right font-mono text-xs tabular-nums",
        editing
          ? "ring-1 ring-highlight bg-highlight/5"
          : "hover:bg-accent/40",
      )}
      title="Click to edit; Enter to commit"
    />
  )
}

/** The middle column — the visual that conveys "what's happening
 *  with this variable over time, at a glance". Different per category. */
function CategoryVisual({
  cat,
  v,
  history,
  sparkColor,
}: {
  cat: VarCategory
  v: VarValue
  history: number[]
  sparkColor: string | undefined
}) {
  switch (cat) {
    case "numeric": {
      const defaultColor = "text-sky-600 dark:text-sky-400"
      return (
        <span
          className={cn("block h-4 w-full", !sparkColor && defaultColor)}
          style={sparkColor ? { color: sparkColor } : undefined}
        >
          <Sparkline values={history} width={120} height={18} filled />
        </span>
      )
    }
    case "bool":
      return <BoolStrip history={history} sparkColor={sparkColor} />
    case "bits":
      return <BitsVisual hex={v.value} />
    case "time":
    case "text":
    case "fb":
    case "other":
      // Nothing to chart — leave the middle column empty so the value
      // column on the right does the talking.
      return <span className="block h-4 w-full" />
  }
}

/** Compact strip of segments showing the last ~80 BOOL transitions —
 *  green when true, muted when false. Faster to read at a glance than
 *  a 0/1 step-trace sparkline. */
function BoolStrip({
  history,
  sparkColor,
}: {
  history: number[]
  sparkColor: string | undefined
}) {
  // Take the last 80 ticks (the strip is 120 px wide → 1.5 px per cell).
  const last = history.slice(-80)
  return (
    <span className="flex h-4 w-full items-center gap-px overflow-hidden">
      {last.length === 0 ? (
        <span className="text-[10px] text-muted-foreground/60">—</span>
      ) : (
        last.map((v, i) => (
          <span
            key={i}
            className={cn(
              "h-2.5 flex-1 rounded-[1px]",
              v > 0.5
                ? sparkColor
                  ? ""
                  : "bg-highlight/80"
                : "bg-muted-foreground/20",
            )}
            style={v > 0.5 && sparkColor ? { backgroundColor: sparkColor } : undefined}
          />
        ))
      )}
    </span>
  )
}

/** Bits — render the value as monospace hex pill so each nibble lines
 *  up; useful for visually spotting which bits are set in an alarm
 *  register (`16#0013` jumps out as different from `16#0010`). */
function BitsVisual({ hex }: { hex: string }) {
  const digits = stripHexPrefix(hex)
  return (
    <span className="flex h-4 items-center">
      <span className="rounded border border-border bg-muted/40 px-1.5 font-mono text-[10px] tracking-wider text-foreground">
        {digits}
      </span>
    </span>
  )
}

/** Right-hand value column: tweak per category so each looks "right"
 *  rather than uniformly using the raw bridge string. */
function renderValue(cat: VarCategory, v: VarValue): string {
  switch (cat) {
    case "time":
      return prettyTime(v.value)
    case "bool":
      return v.value === "TRUE" ? "on" : "off"
    case "fb":
      return "instance"
    case "text":
      // ironplc currently puts a numeric slot value here (the body lives
      // in a separate layout table). Surface that we know it's a string
      // but can't show contents — beats lying.
      return "(string)"
    default:
      return v.value
  }
}
