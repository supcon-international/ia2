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

import {
  clampNotice,
  confirmSummary,
  parseCommitText,
  resolveActionWrite,
  type ResolvedWrite,
} from "@/lib/hmi-action"
import { canHostAction } from "@/lib/hmi-actions"
import {
  bindingVariable,
  colorBinding,
  cssColor,
  displayBinding,
  lookupVar,
  resolveBinding,
  resolveOn,
} from "@/lib/hmi-binding"
import {
  pushTimedHistory,
  windowSlice,
  type TimedSample,
} from "@/lib/var-history"
import { cn } from "@/lib/utils"
import { useHmiMutation } from "@/state/hmi-live"
import { useConnected, useLastSnapshot } from "@/state/live-feed"
import { TrendChart } from "@/components/charts/TrendChart"
import type { HmiAction } from "@/types/generated/HmiAction"
import type { HmiDoc } from "@/types/generated/HmiDoc"
import type { HmiNode } from "@/types/generated/HmiNode"

import { useHmiHost, type HmiHost, type HmiRuntimeState } from "./host"
import { derivePanelHealth, type PanelTone } from "./panel-health"
import { HmiSymbol, type SymbolLive } from "./symbols"

export type CanvasMode = "operate" | "arrange"

/** Per-element delay inside one spawn batch (the "wave"). */
const SPAWN_STAGGER_MS = 80

type PendingConfirm = {
  nodeId: string
  action: HmiAction
  /** Resolved at request time — Confirm sends exactly this. */
  write: ResolvedWrite
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
  const connected = useConnected()
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

  // ---- trend history (one timed ring buffer per referenced variable,
  // retained for the widest window_s among the nodes referencing it;
  // each node slices its own window at render) ----
  const historyRef = useRef<Map<string, TimedSample[]>>(new Map())
  useEffect(() => {
    if (!snapshot || !doc) return
    const t = Date.now() / 1000
    for (const [name, windowS] of trendWindows(doc)) {
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
      pushTimedHistory(buf, t, n, windowS)
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

  // The write itself. `write` was resolved at request time, so the
  // confirm path sends exactly what the dialog showed. A pulse's reset
  // rides the SAME request (`pulseMs`) — the runtime writes the 0, so a
  // closed tab or suspended tablet can't leave the coil latched.
  const performWrite = useCallback(
    async (action: HmiAction, write: ResolvedWrite) => {
      try {
        await host.write(
          write.variable,
          write.value,
          write.typeName,
          action.kind === "pulse" ? action.ms : undefined,
        )
      } catch (e) {
        setActionError(String(e))
      }
    },
    [host],
  )

  const requestAction = useCallback(
    (nodeId: string, action: HmiAction, value?: number) => {
      if (mode !== "operate") return
      setActionError(null)
      if (action.kind === "nav") {
        try {
          host.nav(action.target)
        } catch (e) {
          setActionError(String(e))
        }
        return
      }
      const res = resolveActionWrite(snapshot, action, value)
      if (!res.ok) {
        setActionError(res.reason)
        return
      }
      const needsConfirm =
        "confirm" in action ? action.confirm : true
      if (needsConfirm) {
        setPending({ nodeId, action, write: res.write })
      } else {
        // A clamped no-confirm entry still writes — but never silently.
        setActionError(clampNotice(res.write))
        void performWrite(action, res.write)
      }
    },
    [mode, host, snapshot, performWrite],
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

  // SSE gone while a snapshot is still on screen = every readout is
  // frozen at its last value. Dim the surface (the alarmbar carries the
  // words) so stale numbers can't pass for live ones.
  const stale = !connected && snapshot != null

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
        className={cn(
          "relative origin-top-left border-b border-r border-border/60 bg-background",
          stale && "opacity-60",
        )}
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
            the element so they're visible while it is still fading in.
            Coordinates are accumulated to canvas space: a node inside a
            nested group carries group-relative x/y. */}
        {[...spawnRef.current.entries()].map(([id, sp]) => {
          const hit = findNodeAbs(doc.root, id, 0, 0)
          if (!hit) return null
          const { node: n, x, y } = hit
          return (
            <div
              key={`spawn-${id}`}
              className="hmi-spawn-overlay"
              style={{
                left: x,
                top: y,
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
            void performWrite(p.action, p.write)
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
  historyRef: React.MutableRefObject<Map<string, TimedSample[]>>
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
  // Only control-surface node types fire gestures (mirrors validate_hmi's
  // action-host rule) — a tap on a text label must never reach the plant.
  const tapAction = canHostAction(node.type) ? node.action["tap"] : undefined

  // `visible` bind: 0 hides the element in Operate; Arrange keeps it
  // ghosted so it can still be selected and edited.
  const visBind = node.bind["visible"]
  const hidden =
    visBind !== undefined && (resolveBinding(snapshot, visBind) ?? 1) === 0
  if (hidden && mode === "operate") return null

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
        opacity: hidden ? 0.35 : undefined,
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
  historyRef: React.MutableRefObject<Map<string, TimedSample[]>>,
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
      // Live text (a mapped state label) wins over the static string;
      // a live color (map output) wins over the prop color.
      const textBind = node.bind["text"]
      const liveText =
        textBind !== undefined ? displayBinding(snapshot, textBind) : null
      const colorB = node.bind["color"]
      const liveColor =
        colorB !== undefined ? colorBinding(snapshot, colorB) : null
      const p = node.props
      const color = liveColor ?? (typeof p["color"] === "string" ? (p["color"] as string) : null)
      const style: React.CSSProperties = {}
      if (color) style.color = cssColor(color)
      if (typeof p["size"] === "number") style.fontSize = p["size"] as number
      if (typeof p["align"] === "string")
        style.textAlign = p["align"] as React.CSSProperties["textAlign"]
      if (typeof p["weight"] === "number" || typeof p["weight"] === "string")
        style.fontWeight = p["weight"] as React.CSSProperties["fontWeight"]
      return (
        <div className={cn("truncate", cls)} style={style}>
          {liveText ?? node.text}
        </div>
      )
    }
    case "value": {
      const b = node.bind["value"]
      const display = b !== undefined ? displayBinding(snapshot, b) : null
      const colorB = node.bind["color"]
      const liveColor =
        colorB !== undefined ? colorBinding(snapshot, colorB) : null
      return (
        <div className="flex h-full w-full items-baseline justify-between gap-2 overflow-hidden">
          {node.label && (
            <span className="truncate font-mono text-[11px] text-muted-foreground">
              {node.label}
            </span>
          )}
          <span
            className="font-mono text-[13px] text-foreground"
            style={liveColor ? { color: cssColor(liveColor) } : undefined}
          >
            {display ?? "—"}
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
      const colorB = node.bind["color"]
      const liveColor =
        colorB !== undefined ? colorBinding(snapshot, colorB) : null
      const valueBind = node.bind["value"]
      const history =
        node.symbol === "sparkline" && valueBind !== undefined
          ? windowSlice(
              historyRef.current.get(bindingVariable(valueBind)) ?? [],
              SPARKLINE_WINDOW_S,
            ).map((p) => p.v)
          : undefined
      return (
        <HmiSymbol
          symbol={node.symbol}
          w={node.w || 48}
          h={node.h || 48}
          live={live}
          props={node.props}
          liveColor={liveColor}
          history={history}
        />
      )
    }
    case "trend": {
      const series = node.series.map((s, i) => {
        const buf = windowSlice(
          historyRef.current.get(s.variable) ?? [],
          node.window_s,
        )
        return {
          name: s.label ?? s.variable,
          values: buf.map((p) => p.v),
          times: buf.map((p) => p.t),
          color: TREND_COLORS[i % TREND_COLORS.length],
          binary: false,
        }
      })
      return (
        <div className="h-full w-full rounded border border-border bg-card/60 p-2">
          <TrendChart
            series={series}
            height={Math.max(60, (node.h || 160) - 34)}
            windowS={node.window_s}
          />
        </div>
      )
    }
    case "alarmbar":
      return <AlarmBar host={host} />
    case "button": {
      // Optional state feedback: with `bind.on` the button lights while
      // the bound value is truthy (the indicator's lit treatment), so a
      // toggle shows the state it controls. No binding = plain button.
      const onBind = node.bind["on"]
      const lit = onBind !== undefined && resolveOn(snapshot, onBind)
      return (
        <button
          type="button"
          className={cn(
            "h-full w-full rounded-md border px-3 font-mono text-[12px]",
            lit
              ? "border-highlight bg-highlight/80 text-highlight-foreground hover:bg-highlight/70"
              : "border-border bg-card text-foreground hover:bg-accent/50",
          )}
        >
          {node.label}
        </button>
      )
    }
    case "input":
      return <InputNode node={node} snapshot={snapshot} onAction={onAction} />
    case "nav":
      return (
        <div className="flex h-full w-full items-center justify-center rounded-md border border-border bg-secondary px-3 font-mono text-[12px] text-foreground hover:bg-accent/50">
          {node.label} →
        </div>
      )
    case "shape": {
      // Style: props give the static look, `fill`/`stroke` binds (map
      // outputs) override it live — free-form P&ID pieces that carry
      // state color without a dedicated symbol.
      const p = node.props
      const fillB = node.bind["fill"]
      const strokeB = node.bind["stroke"]
      const liveFill = fillB !== undefined ? colorBinding(snapshot, fillB) : null
      const liveStroke =
        strokeB !== undefined ? colorBinding(snapshot, strokeB) : null
      const fill =
        liveFill ?? (typeof p["fill"] === "string" ? (p["fill"] as string) : null)
      const stroke =
        liveStroke ??
        (typeof p["stroke"] === "string" ? (p["stroke"] as string) : null)
      const strokeW =
        typeof p["stroke_width"] === "number" ? (p["stroke_width"] as number) : null
      const dash = typeof p["dash"] === "string" ? (p["dash"] as string) : null

      if (node.shape === "rect" || node.shape === "ellipse") {
        const rx = typeof p["rx"] === "number" ? (p["rx"] as number) : 4
        return (
          <div
            className={cn(
              "h-full w-full border",
              !stroke && "border-muted-foreground/40",
            )}
            style={{
              borderRadius: node.shape === "ellipse" ? "50%" : rx,
              background: fill ? cssColor(fill) : undefined,
              borderColor: stroke ? cssColor(stroke) : undefined,
              borderWidth: strokeW ?? undefined,
              borderStyle: dash ? "dashed" : undefined,
            }}
          />
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
            className={cn("fill-none", !stroke && "stroke-muted-foreground/40")}
            style={stroke ? { stroke: cssColor(stroke) } : undefined}
            strokeWidth={strokeW ?? 3}
            strokeDasharray={dash ?? undefined}
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
  // Display resolution (not resolveBinding): the placeholder echoes the
  // current value, which may legitimately be text (STRING var).
  const current = b !== undefined ? displayBinding(snapshot, b) : null
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
            // Number("") is 0, not NaN — an empty field must not commit.
            const v = parseCommitText(text)
            if (v !== null) onAction(node.id, commit, v)
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
 *  its runtime status, the edge panel from the runtime's /status. A
 *  failing poll counts toward an explicit COMMS LOST state instead of
 *  keeping the last green state over frozen values. */
const ALARM_TONES: Record<PanelTone, { bar: string; dot: string }> = {
  ok: {
    bar: "border-border bg-card/50 text-[11px] text-muted-foreground",
    dot: "bg-highlight",
  },
  idle: {
    bar: "border-border bg-card/50 text-[11px] text-muted-foreground",
    dot: "bg-muted-foreground/40",
  },
  warn: {
    bar: "border-warn/50 bg-warn/10 text-[12px] text-warn",
    dot: "bg-warn",
  },
  alert: {
    bar: "border-destructive/50 bg-destructive/10 text-[12px] text-destructive",
    dot: "bg-destructive",
  },
}

function AlarmBar({ host }: { host: HmiHost }) {
  const [state, setState] = useState<HmiRuntimeState | null>(null)
  const [failedPolls, setFailedPolls] = useState(0)
  useEffect(() => {
    let cancelled = false
    const tick = async () => {
      try {
        const s = await host.runtimeState()
        if (!cancelled) {
          setState(s)
          setFailedPolls(0)
        }
      } catch {
        if (!cancelled) setFailedPolls((n) => n + 1)
      }
    }
    void tick()
    const id = setInterval(tick, 2000)
    return () => {
      cancelled = true
      clearInterval(id)
    }
  }, [host])
  const health = derivePanelHealth(state, failedPolls)
  const tone = ALARM_TONES[health.tone]
  return (
    <div
      className={cn(
        "flex h-full w-full items-center gap-2 rounded border px-3 font-mono",
        tone.bar,
      )}
    >
      <span className={cn("size-2 shrink-0 rounded-full", tone.dot)} />
      <span className="truncate">{health.text}</span>
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
  const summary = confirmSummary(pending.action, pending.write)
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

/** Same lookup as findNode, but accumulating group offsets into canvas
 *  coordinates (spawn overlays render at the root layer). */
function findNodeAbs(
  n: HmiNode,
  id: string,
  baseX: number,
  baseY: number,
): { node: HmiNode; x: number; y: number } | null {
  const ax = baseX + n.x
  const ay = baseY + n.y
  if (n.id === id) return { node: n, x: ax, y: ay }
  if (n.type === "group") {
    for (const c of n.children) {
      const hit = findNodeAbs(c, id, ax, ay)
      if (hit) return hit
    }
  }
  return null
}

/** Retention window a sparkline keeps for its bound variable. */
const SPARKLINE_WINDOW_S = 120

/** Per-variable retention window: the max `window_s` among the trend
 *  nodes referencing it, plus SPARKLINE_WINDOW_S for every sparkline's
 *  `value` bind (one shared buffer serves them all). */
function trendWindows(doc: HmiDoc): Map<string, number> {
  const out = new Map<string, number>()
  const bump = (v: string, w: number) =>
    out.set(v, Math.max(out.get(v) ?? 0, w))
  const walk = (n: HmiNode) => {
    if (n.type === "trend") {
      for (const s of n.series) bump(s.variable, n.window_s)
    }
    if (n.type === "symbol" && n.symbol === "sparkline") {
      const b = n.bind["value"]
      if (b !== undefined) bump(bindingVariable(b), SPARKLINE_WINDOW_S)
    }
    if (n.type === "group") n.children.forEach(walk)
  }
  walk(doc.root)
  return out
}

