/**
 * The built-in HMI symbol library — ISA-101 high-performance style on the
 * IA2 design tokens. The discipline every symbol follows: calm neutral
 * outlines in the normal state; colour appears only when it carries state
 * (running/open = --highlight, attention = --warn, fault/alarm =
 * --destructive). No gradients, no gloss, no decoration.
 *
 * Contract per symbol (mirrored by the server's /api/hmi-symbols catalog):
 * a `live` map with the resolved numeric value per bind key (null when
 * unresolved), and the node's static `props`.
 */

import { cn } from "@/lib/utils"

export type SymbolLive = Record<string, number | null>

type Props = {
  symbol: string
  w: number
  h: number
  live: SymbolLive
  props: Record<string, unknown>
}

const on = (v: number | null | undefined) => (v ?? 0) !== 0

function label(props: Record<string, unknown>): string | null {
  const l = props["label"]
  return typeof l === "string" && l.length > 0 ? l : null
}

export function HmiSymbol({ symbol, w, h, live, props }: Props) {
  switch (symbol) {
    case "tank":
      return <Tank w={w} h={h} live={live} props={props} />
    case "valve":
      return <Valve w={w} h={h} live={live} props={props} />
    case "pump":
      return <RoundMachine w={w} h={h} live={live} props={props} kind="pump" />
    case "motor":
      return <RoundMachine w={w} h={h} live={live} props={props} kind="motor" />
    case "pipe_h":
      return <div className="h-[6px] w-full rounded-full bg-muted-foreground/30" />
    case "pipe_v":
      return <div className="h-full w-[6px] rounded-full bg-muted-foreground/30" />
    case "gauge":
      return <Gauge w={w} h={h} live={live} props={props} />
    case "indicator":
      return <Indicator live={live} props={props} />
    case "setpoint":
      return <SetpointChip live={live} props={props} />
    default:
      // Unknown symbol: visible placeholder, never a silent hole —
      // validate warns about it too.
      return (
        <div className="flex h-full w-full items-center justify-center rounded border border-dashed border-warn/60 font-mono text-[10px] text-warn">
          {symbol}?
        </div>
      )
  }
}

/** Vessel with a live fill (0-100) and an alarm ring. */
function Tank({
  w,
  h,
  live,
  props,
}: {
  w: number
  h: number
  live: SymbolLive
  props: Record<string, unknown>
}) {
  const pct = Math.max(0, Math.min(100, live["value"] ?? 0))
  const alarm = on(live["alarm"])
  const unit = typeof props["unit"] === "string" ? (props["unit"] as string) : "%"
  const bodyH = h - 18
  const fillH = (bodyH - 4) * (pct / 100)
  return (
    <div className="flex h-full w-full flex-col items-center">
      <svg width={w} height={bodyH} viewBox={`0 0 ${w} ${bodyH}`}>
        <rect
          x={1}
          y={1}
          width={w - 2}
          height={bodyH - 2}
          rx={8}
          className={cn(
            "fill-card stroke-muted-foreground/60",
            alarm && "stroke-destructive",
          )}
          strokeWidth={alarm ? 2 : 1.5}
        />
        <rect
          x={3}
          y={bodyH - 3 - fillH}
          width={w - 6}
          height={fillH}
          rx={6}
          className={cn("fill-trend/25", alarm && "fill-destructive/20")}
        />
        <text
          x={w / 2}
          y={bodyH / 2 + 4}
          textAnchor="middle"
          className="fill-foreground font-mono text-[13px]"
        >
          {live["value"] == null ? "—" : `${pct.toFixed(0)}${unit}`}
        </text>
      </svg>
      {label(props) && (
        <div className="mt-0.5 font-mono text-[10px] text-muted-foreground">
          {label(props)}
        </div>
      )}
    </div>
  )
}

/** Bowtie block valve — filled when open, destructive ring on fault. */
function Valve({
  w,
  h,
  live,
  props,
}: {
  w: number
  h: number
  live: SymbolLive
  props: Record<string, unknown>
}) {
  const openState = on(live["open"])
  const fault = on(live["fault"])
  const s = Math.min(w, h - (label(props) ? 14 : 0))
  const half = s / 2
  return (
    <div className="flex h-full w-full flex-col items-center justify-center">
      <svg width={s} height={s} viewBox={`0 0 ${s} ${s}`}>
        <path
          d={`M2 4 L${half} ${half} L2 ${s - 4} Z M${s - 2} 4 L${half} ${half} L${s - 2} ${s - 4} Z`}
          className={cn(
            openState
              ? "fill-highlight/80 stroke-highlight"
              : "fill-card stroke-muted-foreground/70",
            fault && "stroke-destructive",
          )}
          strokeWidth={fault ? 2 : 1.5}
          strokeLinejoin="round"
        />
      </svg>
      {label(props) && (
        <div className="font-mono text-[10px] text-muted-foreground">
          {label(props)}
        </div>
      )}
    </div>
  )
}

/** Pump (impeller tick) / motor (M) — filled when running. */
function RoundMachine({
  w,
  h,
  live,
  props,
  kind,
}: {
  w: number
  h: number
  live: SymbolLive
  props: Record<string, unknown>
  kind: "pump" | "motor"
}) {
  const running = on(live["running"])
  const fault = on(live["fault"])
  const s = Math.min(w, h - (label(props) ? 14 : 0))
  const r = s / 2 - 2
  return (
    <div className="flex h-full w-full flex-col items-center justify-center">
      <svg width={s} height={s} viewBox={`0 0 ${s} ${s}`}>
        <circle
          cx={s / 2}
          cy={s / 2}
          r={r}
          className={cn(
            running
              ? "fill-highlight/80 stroke-highlight"
              : "fill-card stroke-muted-foreground/70",
            fault && "stroke-destructive",
          )}
          strokeWidth={fault ? 2 : 1.5}
        />
        {kind === "motor" ? (
          <text
            x={s / 2}
            y={s / 2 + 4}
            textAnchor="middle"
            className={cn(
              "font-mono text-[12px] font-bold",
              running ? "fill-highlight-foreground" : "fill-muted-foreground",
            )}
          >
            M
          </text>
        ) : (
          <path
            d={`M${s / 2} ${s / 2} L${s / 2 + r * 0.8} ${s / 2 - r * 0.45} M${s / 2} ${s / 2} L${s / 2 - r * 0.2} ${s / 2 - r * 0.85}`}
            className={cn(
              "stroke-[1.5]",
              running ? "stroke-highlight-foreground" : "stroke-muted-foreground",
            )}
            strokeLinecap="round"
          />
        )}
      </svg>
      {label(props) && (
        <div className="font-mono text-[10px] text-muted-foreground">
          {label(props)}
        </div>
      )}
    </div>
  )
}

/** Radial 0-100 gauge — ISA says use sparingly; we oblige with restraint. */
function Gauge({
  w,
  h,
  live,
  props,
}: {
  w: number
  h: number
  live: SymbolLive
  props: Record<string, unknown>
}) {
  const pct = Math.max(0, Math.min(100, live["value"] ?? 0))
  const s = Math.min(w, h - (label(props) ? 14 : 0))
  const r = s / 2 - 6
  const a0 = Math.PI * 0.75
  const a1 = Math.PI * 0.75 + Math.PI * 1.5 * (pct / 100)
  const arc = (a: number) => ({
    x: s / 2 + r * Math.cos(a),
    y: s / 2 + r * Math.sin(a),
  })
  const p0 = arc(a0)
  const p1 = arc(a1)
  const large = a1 - a0 > Math.PI ? 1 : 0
  return (
    <div className="flex h-full w-full flex-col items-center justify-center">
      <svg width={s} height={s} viewBox={`0 0 ${s} ${s}`}>
        <circle
          cx={s / 2}
          cy={s / 2}
          r={r}
          className="fill-none stroke-muted-foreground/25"
          strokeWidth={5}
        />
        {pct > 0 && (
          <path
            d={`M${p0.x} ${p0.y} A${r} ${r} 0 ${large} 1 ${p1.x} ${p1.y}`}
            className="fill-none stroke-trend"
            strokeWidth={5}
            strokeLinecap="round"
          />
        )}
        <text
          x={s / 2}
          y={s / 2 + 5}
          textAnchor="middle"
          className="fill-foreground font-mono text-[14px]"
        >
          {live["value"] == null ? "—" : pct.toFixed(0)}
        </text>
      </svg>
      {label(props) && (
        <div className="font-mono text-[10px] text-muted-foreground">
          {label(props)}
        </div>
      )}
    </div>
  )
}

/** Status dot + label row. Calm gray off; highlight on; destructive alarm. */
function Indicator({
  live,
  props,
}: {
  live: SymbolLive
  props: Record<string, unknown>
}) {
  const isOn = on(live["on"])
  const alarm = on(live["alarm"])
  return (
    <div className="flex h-full w-full items-center gap-2 overflow-hidden">
      <span
        className={cn(
          "size-[10px] shrink-0 rounded-full border",
          alarm
            ? "border-destructive bg-destructive"
            : isOn
              ? "border-highlight bg-highlight"
              : "border-muted-foreground/50 bg-transparent",
        )}
      />
      <span
        className={cn(
          "truncate font-mono text-[12px]",
          alarm ? "text-destructive" : "text-foreground",
        )}
      >
        {label(props) ?? "—"}
      </span>
    </div>
  )
}

/** Read-only setpoint chip. */
function SetpointChip({
  live,
  props,
}: {
  live: SymbolLive
  props: Record<string, unknown>
}) {
  const unit = typeof props["unit"] === "string" ? (props["unit"] as string) : ""
  const v = live["value"]
  return (
    <div className="flex h-full w-full items-center justify-between gap-2 rounded border border-border bg-card px-2">
      <span className="truncate font-mono text-[11px] text-muted-foreground">
        {label(props) ?? "SP"}
      </span>
      <span className="font-mono text-[12px] text-foreground">
        {v == null ? "—" : v}
        {unit && <span className="ml-0.5 text-muted-foreground">{unit}</span>}
      </span>
    </div>
  )
}
