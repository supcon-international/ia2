/**
 * The HMI canvas — renders one screen document live against the running
 * program, and renders it INCREMENTALLY: every `hmi` SSE mutation reloads
 * the document, and the node ids the mutation touched get a brief spawn
 * animation, so a watching human sees an agent assemble the screen
 * element by element (the Pencil workflow, pointed at a plant).
 *
 * Two explicit modes keep gestures unambiguous:
 *   - Operate: actions are live (tap a valve, commit a setpoint); nothing
 *     moves. The default — this is an operator surface first.
 *   - Arrange: drag-to-move with grid snap (saved on release); actions
 *     are inert so a mis-tap can't write to the plant while laying out.
 */

import { useCallback, useEffect, useRef, useState } from "react"

import { formatBinding, lookupVar, resolveBinding } from "@/lib/hmi-binding"
import { pushHistory } from "@/lib/var-history"
import { cn } from "@/lib/utils"
import { useHmiMutation } from "@/state/hmi-live"
import { useLastSnapshot } from "@/state/live-feed"
import { TrendChart } from "@/components/charts/TrendChart"
import type { HmiAction } from "@/types/generated/HmiAction"
import type { HmiDoc } from "@/types/generated/HmiDoc"
import type { HmiNode } from "@/types/generated/HmiNode"

import { useHmiHost, type HmiHost } from "./host"
import { HmiSymbol, type SymbolLive } from "./symbols"

export type CanvasMode = "operate" | "arrange"

/** Per-element delay inside one spawn batch (the "wave"). */
const SPAWN_STAGGER_MS = 80

type PendingConfirm = {
  nodeId: string
  action: HmiAction
  /** For set_value: the number the user entered. */
  value?: number
}

export function HmiCanvas({
  path,
  mode,
  selected,
  onSelect,
  onDocLoaded,
}: {
  path: string
  mode: CanvasMode
  selected: string | null
  onSelect: (id: string | null) => void
  onDocLoaded?: (doc: HmiDoc) => void
}) {
  const host = useHmiHost()
  const [doc, setDoc] = useState<HmiDoc | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const snapshot = useLastSnapshot()
  const mutation = useHmiMutation()

  // ---- document load + live reload --------------------------------
  const load = useCallback(async () => {
    try {
      const d = await host.fetchDoc(path)
      setDoc(d)
      setLoadError(null)
      onDocLoaded?.(d)
    } catch (e) {
      setLoadError(String(e))
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path, host])

  useEffect(() => {
    void load()
  }, [load])

  // Spawn animation bookkeeping: ids touched by the latest mutation get
  // the class until their timer expires. A ref map keeps timers out of
  // render; the `anim` state just triggers the re-render.
  const spawnRef = useRef<Map<string, { until: number; order: number }>>(new Map())
  const [, bumpAnim] = useState(0)
  useEffect(() => {
    if (!mutation || mutation.path !== path) return
    if (mutation.deleted) {
      setDoc(null)
      setLoadError("screen was deleted")
      return
    }
    void load()
    if (mutation.touched.length > 0) {
      // Per-batch stagger: each touched element's frame/pop/glow chain
      // starts SPAWN_STAGGER_MS after the previous one, so a batch
      // reads as a wave of drawing. The expiry covers the last
      // element's full chain.
      const life = 1400 + mutation.touched.length * SPAWN_STAGGER_MS
      const until = Date.now() + life
      mutation.touched.forEach((id, order) => {
        spawnRef.current.set(id, { until, order })
      })
      bumpAnim((n) => n + 1)
      const t = setTimeout(() => {
        const now = Date.now()
        for (const [id, s] of spawnRef.current) {
          if (s.until <= now) spawnRef.current.delete(id)
        }
        bumpAnim((n) => n + 1)
      }, life + 50)
      return () => clearTimeout(t)
    }
  }, [mutation, path, load])

  // ---- letterbox scaling ------------------------------------------
  const wrapRef = useRef<HTMLDivElement | null>(null)
  const [scale, setScale] = useState(1)
  useEffect(() => {
    const el = wrapRef.current
    if (!el || !doc) return
    const ro = new ResizeObserver(() => {
      const availW = el.clientWidth
      if (availW === 0) return
      // Width-fit with a readability floor: the screen scrolls vertically
      // rather than shrinking into an unreadable thumbnail when the pane
      // is short (Monitor keeps the bottom third). Operator tablets get
      // the full-height fit naturally because their pane IS the window.
      setScale(Math.min(Math.max(availW / doc.grid.w, 0.5), 1.25))
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [doc])

  // ---- trend history (one ring buffer per referenced variable) ----
  const historyRef = useRef<Map<string, number[]>>(new Map())
  useEffect(() => {
    if (!snapshot || !doc) return
    for (const name of trendVariables(doc)) {
      const found = lookupVar(snapshot, name)
      if (!found) continue
      const n = Number.isNaN(Number(found.raw))
        ? (/^true$/i.test(found.raw.trim()) ? 1 : 0)
        : Number(found.raw)
      let buf = historyRef.current.get(name)
      if (!buf) {
        buf = []
        historyRef.current.set(name, buf)
      }
      pushHistory(buf, n)
    }
  }, [snapshot, doc])

  // ---- drag-to-move (Arrange mode) --------------------------------
  // Gesture data lives in the ref (single source of truth — pointerup may
  // fire in the same task as the last move, before any re-render); the
  // state mirror only drives the visual position during the drag.
  const dragRef = useRef<{
    id: string
    startX: number
    startY: number
    origX: number
    origY: number
    curX: number
    curY: number
    moved: boolean
  } | null>(null)
  const [dragPos, setDragPos] = useState<{ id: string; x: number; y: number } | null>(null)

  const onNodePointerDown = (n: HmiNode, e: React.PointerEvent) => {
    // Selection is an Arrange-mode concept. In Operate a tap is an
    // ACTION — selecting here used to pop the inspector, reflow the
    // canvas mid-click, and swallow the click's tap under the moved
    // layout (real mice lost actions to it, not just automation).
    if (mode !== "arrange") return
    onSelect(n.id)
    e.preventDefault()
    try {
      ;(e.target as Element).setPointerCapture?.(e.pointerId)
    } catch {
      /* synthetic events (tests) have no active pointer — capture is
       * an optimisation, not a requirement */
    }
    dragRef.current = {
      id: n.id,
      startX: e.clientX,
      startY: e.clientY,
      origX: n.x,
      origY: n.y,
      curX: n.x,
      curY: n.y,
      moved: false,
    }
  }
  const onPointerMove = (e: React.PointerEvent) => {
    const d = dragRef.current
    if (!d || !doc) return
    const snap = Math.max(1, doc.grid.snap)
    const nx =
      Math.round((d.origX + (e.clientX - d.startX) / scale) / snap) * snap
    const ny =
      Math.round((d.origY + (e.clientY - d.startY) / scale) / snap) * snap
    if (nx !== d.origX || ny !== d.origY) d.moved = true
    d.curX = nx
    d.curY = ny
    setDragPos({ id: d.id, x: nx, y: ny })
  }
  const onPointerUp = async () => {
    const d = dragRef.current
    dragRef.current = null
    setDragPos(null)
    if (!d || !doc || !d.moved || !host.saveDoc) return
    const next = structuredClone(doc)
    const target = findNode(next.root, d.id)
    if (target) {
      target.x = d.curX
      target.y = d.curY
      setDoc(next)
      try {
        await host.saveDoc(path, next)
      } catch {
        void load() // server rejected — resync to truth
      }
    }
  }

  // ---- actions (Operate mode) -------------------------------------
  const [pending, setPending] = useState<PendingConfirm | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)

  const execute = useCallback(
    async (action: HmiAction) => {
      setActionError(null)
      try {
        if (action.kind === "nav") {
          host.nav(action.target)
          return
        }
        const variable = action.variable
        const typeName =
          (snapshot && lookupVar(snapshot, variable)?.type_name) || ""
        if (action.kind === "write") {
          await host.write(variable, action.value, typeName)
        } else if (action.kind === "toggle") {
          const cur = snapshot ? lookupVar(snapshot, variable) : null
          const on = cur ? /^(true|1)$/i.test(cur.raw.trim()) : false
          await host.write(variable, on ? 0 : 1, typeName || "BOOL")
        } else if (action.kind === "pulse") {
          await host.write(variable, 1, typeName || "BOOL")
          setTimeout(() => {
            void host.write(variable, 0, typeName || "BOOL").catch(() => {})
          }, action.ms)
        } else if (action.kind === "set_value") {
          // value arrives through the confirm flow
        }
      } catch (e) {
        setActionError(String(e))
      }
    },
    [host, snapshot],
  )

  const requestAction = useCallback(
    (nodeId: string, action: HmiAction, value?: number) => {
      if (mode !== "operate") return
      if (action.kind === "nav") {
        void execute(action)
        return
      }
      const needsConfirm =
        "confirm" in action ? action.confirm : true
      if (needsConfirm) {
        setPending({ nodeId, action, value })
      } else {
        void executeWithValue(action, value)
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [mode, execute],
  )

  const executeWithValue = useCallback(
    async (action: HmiAction, value?: number) => {
      if (action.kind === "set_value") {
        if (value == null || Number.isNaN(value)) return
        const lo = action.min ?? -Infinity
        const hi = action.max ?? Infinity
        const clamped = Math.min(hi, Math.max(lo, value))
        const typeName =
          (snapshot && lookupVar(snapshot, action.variable)?.type_name) || ""
        setActionError(null)
        try {
          await host.write(action.variable, clamped, typeName)
        } catch (e) {
          setActionError(String(e))
        }
        return
      }
      await execute(action)
    },
    [execute, host, snapshot],
  )

  // ---- render ------------------------------------------------------
  if (loadError) {
    return (
      <div className="grid h-full place-items-center p-6 text-center text-sm text-muted-foreground">
        {loadError}
      </div>
    )
  }
  if (!doc) {
    return (
      <div className="grid h-full place-items-center text-sm text-muted-foreground">
        Loading…
      </div>
    )
  }

  const rootChildren =
    doc.root.type === "group" ? doc.root.children : []

  return (
    <div
      ref={wrapRef}
      className="relative h-full w-full overflow-auto bg-muted/20"
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onClick={(e) => {
        if (e.target === e.currentTarget) onSelect(null)
      }}
    >
      <div
        className="relative origin-top-left border-b border-r border-border/60 bg-background"
        style={{
          width: doc.grid.w,
          height: doc.grid.h,
          transform: `scale(${scale})`,
        }}
        onClick={(e) => {
          if (e.target === e.currentTarget) onSelect(null)
        }}
      >
        {rootChildren.map((n) => (
          <CanvasNode
            key={n.id}
            node={n}
            doc={doc}
            snapshotTick={snapshot?.scan_count}
            spawn={spawnRef.current}
            selected={selected}
            mode={mode}
            dragPos={dragPos}
            historyRef={historyRef}
            onPointerDown={onNodePointerDown}
            onAction={requestAction}
          />
        ))}
        {/* Spawn overlays — the dashed sketch frame + glow live outside
            the element so they're visible while it is still fading in. */}
        {[...spawnRef.current.entries()].map(([id, sp]) => {
          const n = findNode(doc.root, id)
          if (!n) return null
          return (
            <div
              key={`spawn-${id}`}
              className="hmi-spawn-overlay"
              style={{
                left: n.x,
                top: n.y,
                width: n.w > 0 ? n.w : 120,
                height: n.h > 0 ? n.h : 32,
                ...({ "--spawn-delay": `${sp.order * SPAWN_STAGGER_MS}ms` } as React.CSSProperties),
              }}
            />
          )
        })}
      </div>

      {pending && (
        <ConfirmCard
          pending={pending}
          onCancel={() => setPending(null)}
          onConfirm={() => {
            const p = pending
            setPending(null)
            void executeWithValue(p.action, p.value)
          }}
        />
      )}
      {actionError && (
        <div className="absolute bottom-3 left-3 rounded border border-destructive/40 bg-destructive/10 px-2 py-1 text-[11px] text-destructive">
          {actionError}
        </div>
      )}
    </div>
  )
}

// One node. Split out so a snapshot tick re-renders the tree cheaply —
// the components are small and the tree is tens of nodes, so plain
// rendering is fine at 10 Hz; memoization can come with evidence.
function CanvasNode({
  node,
  doc,
  snapshotTick,
  spawn,
  selected,
  mode,
  dragPos,
  historyRef,
  onPointerDown,
  onAction,
}: {
  node: HmiNode
  doc: HmiDoc
  snapshotTick: unknown
  spawn: Map<string, { until: number; order: number }>
  selected: string | null
  mode: CanvasMode
  dragPos: { id: string; x: number; y: number } | null
  historyRef: React.MutableRefObject<Map<string, number[]>>
  onPointerDown: (n: HmiNode, e: React.PointerEvent) => void
  onAction: (nodeId: string, action: HmiAction, value?: number) => void
}) {
  const snapshot = useLastSnapshot()
  const host = useHmiHost()
  const pos =
    dragPos && dragPos.id === node.id
      ? { x: dragPos.x, y: dragPos.y }
      : { x: node.x, y: node.y }
  const spawning = spawn.get(node.id)
  const tapAction = node.action["tap"]

  const body = renderKind(node, snapshot, historyRef, onAction, host)

  return (
    <div
      data-hmi-id={node.id}
      className={cn(
        "absolute",
        spawning !== undefined && "hmi-spawn",
        mode === "arrange" && "cursor-grab active:cursor-grabbing",
        mode === "operate" && tapAction && "cursor-pointer",
        selected === node.id &&
          "outline outline-1 outline-offset-2 outline-ring",
      )}
      style={{
        left: pos.x,
        top: pos.y,
        width: node.w > 0 ? node.w : undefined,
        height: node.h > 0 ? node.h : undefined,
        ...(spawning !== undefined
          ? ({ "--spawn-delay": `${spawning.order * SPAWN_STAGGER_MS}ms` } as React.CSSProperties)
          : {}),
      }}
      onPointerDown={(e) => {
        e.stopPropagation()
        onPointerDown(node, e)
      }}
      onClick={(e) => {
        e.stopPropagation()
        if (mode === "operate" && tapAction) onAction(node.id, tapAction)
      }}
    >
      {body}
      {node.type === "group" &&
        node.children.map((c) => (
          <CanvasNode
            key={c.id}
            node={c}
            doc={doc}
            snapshotTick={snapshotTick}
            spawn={spawn}
            selected={selected}
            mode={mode}
            dragPos={dragPos}
            historyRef={historyRef}
            onPointerDown={onPointerDown}
            onAction={onAction}
          />
        ))}
    </div>
  )
}

function renderKind(
  node: HmiNode,
  snapshot: ReturnType<typeof useLastSnapshot>,
  historyRef: React.MutableRefObject<Map<string, number[]>>,
  onAction: (nodeId: string, action: HmiAction, value?: number) => void,
  host: HmiHost,
) {
  switch (node.type) {
    case "group":
      return null
    case "text": {
      const cls =
        node.style === "title"
          ? "text-[16px] font-semibold text-foreground"
          : node.style === "section"
            ? "text-[11px] font-medium uppercase tracking-wider text-muted-foreground"
            : node.style === "caption"
              ? "text-[10px] text-muted-foreground"
              : "text-[12px] text-foreground"
      return <div className={cn("truncate", cls)}>{node.text}</div>
    }
    case "value": {
      const b = node.bind["value"]
      const v = b !== undefined ? resolveBinding(snapshot, b) : null
      return (
        <div className="flex h-full w-full items-baseline justify-between gap-2 overflow-hidden">
          {node.label && (
            <span className="truncate font-mono text-[11px] text-muted-foreground">
              {node.label}
            </span>
          )}
          <span className="font-mono text-[13px] text-foreground">
            {v == null || b === undefined ? "—" : formatBinding(b, v)}
            {node.unit && (
              <span className="ml-0.5 text-[10px] text-muted-foreground">
                {node.unit}
              </span>
            )}
          </span>
        </div>
      )
    }
    case "symbol": {
      const live: SymbolLive = {}
      for (const [k, b] of Object.entries(node.bind)) {
        live[k] = b === undefined ? null : resolveBinding(snapshot, b)
      }
      return (
        <HmiSymbol
          symbol={node.symbol}
          w={node.w || 48}
          h={node.h || 48}
          live={live}
          props={node.props}
        />
      )
    }
    case "trend": {
      const series = node.series.map((s, i) => ({
        name: s.label ?? s.variable,
        values: historyRef.current.get(s.variable) ?? [],
        color: TREND_COLORS[i % TREND_COLORS.length],
        binary: false,
      }))
      return (
        <div className="h-full w-full rounded border border-border bg-card/60 p-2">
          <TrendChart series={series} height={Math.max(60, (node.h || 160) - 34)} />
        </div>
      )
    }
    case "alarmbar":
      return <AlarmBar host={host} />
    case "button":
      return (
        <button
          type="button"
          className="h-full w-full rounded-md border border-border bg-card px-3 font-mono text-[12px] text-foreground hover:bg-accent/50"
        >
          {node.label}
        </button>
      )
    case "input":
      return <InputNode node={node} snapshot={snapshot} onAction={onAction} />
    case "nav":
      return (
        <div className="flex h-full w-full items-center justify-center rounded-md border border-border bg-secondary px-3 font-mono text-[12px] text-foreground hover:bg-accent/50">
          {node.label} →
        </div>
      )
    case "shape": {
      if (node.shape === "rect") {
        return (
          <div className="h-full w-full rounded border border-muted-foreground/40" />
        )
      }
      // line / polyline: draw through the points within the node box.
      const pts =
        node.points.length >= 2
          ? node.points
          : ([[0, 0], [node.w || 100, node.h || 0]] as [number, number][])
      const path = pts.map(([x, y], i) => `${i === 0 ? "M" : "L"}${x} ${y}`).join(" ")
      return (
        <svg
          width={node.w || 100}
          height={Math.max(node.h, 8)}
          className="overflow-visible"
        >
          <path
            d={path}
            className="fill-none stroke-muted-foreground/40"
            strokeWidth={3}
            strokeLinecap="round"
          />
        </svg>
      )
    }
  }
}

const TREND_COLORS = ["var(--trend)", "var(--highlight)", "var(--warn)"]

function InputNode({
  node,
  snapshot,
  onAction,
}: {
  node: Extract<HmiNode, { type: "input" }>
  snapshot: ReturnType<typeof useLastSnapshot>
  onAction: (nodeId: string, action: HmiAction, value?: number) => void
}) {
  const [text, setText] = useState("")
  const commit = node.action["commit"]
  const b = node.bind["value"]
  const current = b !== undefined ? resolveBinding(snapshot, b) : null
  return (
    <div className="flex h-full w-full items-center gap-1.5 overflow-hidden">
      {node.label && (
        <span className="truncate font-mono text-[11px] text-muted-foreground">
          {node.label}
        </span>
      )}
      <input
        value={text}
        onChange={(e) => setText(e.target.value)}
        onClick={(e) => e.stopPropagation()}
        onPointerDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === "Enter" && commit) {
            const v = Number(text)
            if (!Number.isNaN(v)) onAction(node.id, commit, v)
            setText("")
          }
        }}
        placeholder={current == null ? "—" : String(current)}
        className="h-full min-w-0 flex-1 rounded border border-input bg-background px-1.5 font-mono text-[12px] text-foreground outline-none focus:border-ring"
      />
      {node.unit && (
        <span className="font-mono text-[10px] text-muted-foreground">
          {node.unit}
        </span>
      )}
    </div>
  )
}

/** Fault + run-state strip. Calm when nothing is wrong (ISA-101: color
 *  only when it means something). Polls the host — the IDE answers from
 *  its runtime status, the edge panel from the runtime's /status. */
function AlarmBar({ host }: { host: HmiHost }) {
  const [state, setState] = useState<{ running: boolean; alarm: string | null }>(
    { running: false, alarm: null },
  )
  useEffect(() => {
    let cancelled = false
    const tick = async () => {
      try {
        const s = await host.runtimeState()
        if (!cancelled) setState(s)
      } catch {
        /* offline — keep last */
      }
    }
    void tick()
    const id = setInterval(tick, 2000)
    return () => {
      cancelled = true
      clearInterval(id)
    }
  }, [host])
  if (state.alarm) {
    return (
      <div className="flex h-full w-full items-center gap-2 rounded border border-destructive/50 bg-destructive/10 px-3 font-mono text-[12px] text-destructive">
        <span className="size-2 shrink-0 rounded-full bg-destructive" />
        <span className="truncate">{state.alarm}</span>
      </div>
    )
  }
  return (
    <div className="flex h-full w-full items-center gap-2 rounded border border-border bg-card/50 px-3 font-mono text-[11px] text-muted-foreground">
      <span
        className={cn(
          "size-2 shrink-0 rounded-full",
          state.running ? "bg-highlight" : "bg-muted-foreground/40",
        )}
      />
      {state.running ? "Running — no active faults" : "Stopped"}
    </div>
  )
}

function ConfirmCard({
  pending,
  onCancel,
  onConfirm,
}: {
  pending: PendingConfirm
  onCancel: () => void
  onConfirm: () => void
}) {
  const a = pending.action
  const summary =
    a.kind === "write"
      ? `Write ${a.value} → ${a.variable}`
      : a.kind === "toggle"
        ? `Toggle ${a.variable}`
        : a.kind === "pulse"
          ? `Pulse ${a.variable} (${a.ms} ms)`
          : a.kind === "set_value"
            ? `Set ${a.variable} = ${pending.value}`
            : ""
  return (
    <div className="absolute inset-0 z-20 grid place-items-center bg-black/20">
      <div className="w-[300px] rounded-lg border border-border bg-popover p-4 shadow-2xl">
        <div className="text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
          Confirm action
        </div>
        <div className="mt-2 font-mono text-[13px] text-foreground">
          {summary}
        </div>
        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            className="rounded-md border border-border bg-card px-3 py-1 text-[12px] text-muted-foreground hover:text-foreground"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={onConfirm}
            className="rounded-md bg-primary px-3 py-1 text-[12px] font-medium text-primary-foreground hover:bg-primary/90"
          >
            Confirm
          </button>
        </div>
      </div>
    </div>
  )
}

// ---- small pure helpers -------------------------------------------

export function findNode(root: HmiNode, id: string): HmiNode | null {
  if (root.id === id) return root
  if (root.type === "group") {
    for (const c of root.children) {
      const hit = findNode(c, id)
      if (hit) return hit
    }
  }
  return null
}

function trendVariables(doc: HmiDoc): string[] {
  const out: string[] = []
  const walk = (n: HmiNode) => {
    if (n.type === "trend") for (const s of n.series) out.push(s.variable)
    if (n.type === "group") n.children.forEach(walk)
  }
  walk(doc.root)
  return out
}

