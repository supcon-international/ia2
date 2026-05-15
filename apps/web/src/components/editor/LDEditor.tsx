import { useMemo } from "react"

import { cn } from "@/lib/utils"
import type { LdCoil } from "@/types/generated/LdCoil"
import type { LdNode } from "@/types/generated/LdNode"
import type { LdProgram } from "@/types/generated/LdProgram"
import type { LdRung } from "@/types/generated/LdRung"

/**
 * Read-only SVG renderer for a Ladder Diagram POU.
 *
 * Design notes:
 *
 *  - Source of truth is the JSON literal (the `.ld.json` file on disk).
 *    We parse it once on every render. If the JSON is malformed (e.g.
 *    mid-edit save) we render an inline error rather than blanking the
 *    pane, so the user can recover by editing the JSON.
 *
 *  - Layout is deterministic from the boolean tree:
 *      AND  → children laid out left-to-right ("series")
 *      OR   → children laid out top-to-bottom ("parallel"), with extra
 *             vertical rungs auto-drawn to merge the branches back
 *             onto the main horizontal rail.
 *      NOT  → inline slash through the contact (`|/|` notation).
 *      Contacts and coils are fixed-width "tiles" so columns line up
 *      across rungs vertically.
 *
 *  - No drag, no editing. Phase 1 ships read-only — authoring goes
 *    through the JSON view (see `LDJsonFallback` below for the
 *    affordance). Drag/drop authoring lands in a follow-up phase.
 */

export function LDEditor({
  source,
  className,
}: {
  source: string
  className?: string
}) {
  const parsed = useMemo(() => parse(source), [source])

  if (parsed.kind === "error") {
    return (
      <div className={cn("flex h-full min-h-0 flex-col", className)}>
        <div className="border-b border-destructive/40 bg-destructive/5 px-3 py-2 text-xs text-destructive">
          LD JSON parse error: {parsed.message}
        </div>
        <LDJsonFallback source={source} />
      </div>
    )
  }
  const prog = parsed.program

  return (
    <div className={cn("flex h-full min-h-0 flex-col", className)}>
      <div className="border-b border-border bg-muted/30 px-3 py-1.5 text-[11px] uppercase tracking-wider text-muted-foreground">
        <span className="font-mono normal-case tracking-normal text-foreground">
          {prog.name}
        </span>
        <span className="ml-2 rounded border border-border bg-muted/50 px-1.5 py-0.5 font-mono text-[9px] text-muted-foreground">
          ld
        </span>
        <span className="ml-2 rounded border border-border bg-muted/50 px-1.5 py-0.5 font-mono text-[9px] text-muted-foreground">
          {prog.pou_type === "function_block" ? "fb" : "prg"}
        </span>
        <span className="ml-3">
          {prog.rungs.length} rung{prog.rungs.length === 1 ? "" : "s"} ·{" "}
          {prog.variables.length} var{prog.variables.length === 1 ? "" : "s"}
        </span>
      </div>

      <div className="flex-1 overflow-auto bg-background">
        <VariableLegend variables={prog.variables} />
        <div className="space-y-3 px-4 py-3">
          {prog.rungs.map((rung, i) => (
            <RungView key={rung.id} rung={rung} index={i} />
          ))}
        </div>
      </div>
    </div>
  )
}

/** Tiny variables panel rendered above the rungs. We deliberately keep
 *  this compact (not a separate tab) so the relationship between var
 *  names visible on contacts and their types stays one glance away. */
function VariableLegend({
  variables,
}: {
  variables: LdProgram["variables"]
}) {
  if (variables.length === 0) {
    return (
      <div className="border-b border-border px-4 py-2 text-[11px] text-muted-foreground">
        no variables declared
      </div>
    )
  }
  const groups: Array<{ label: string; section: string }> = [
    { label: "VAR_INPUT", section: "input" },
    { label: "VAR_OUTPUT", section: "output" },
    { label: "VAR", section: "internal" },
  ]
  return (
    <div className="grid grid-cols-3 gap-3 border-b border-border bg-muted/10 px-4 py-2 text-[11px]">
      {groups.map((g) => {
        const vs = variables.filter((v) => v.section === g.section)
        if (vs.length === 0) return null
        return (
          <div key={g.section}>
            <div className="mb-1 font-mono text-[9px] uppercase tracking-wider text-muted-foreground">
              {g.label}
            </div>
            <ul className="space-y-0.5">
              {vs.map((v) => (
                <li key={v.name} className="font-mono">
                  <span className="text-foreground">{v.name}</span>
                  <span className="ml-2 text-muted-foreground">{v.type}</span>
                  {v.init !== null && v.init !== undefined && (
                    <span className="ml-1 text-muted-foreground">
                      := {v.init}
                    </span>
                  )}
                </li>
              ))}
            </ul>
          </div>
        )
      })}
    </div>
  )
}

// =================================================================
//   Rung rendering — recursive layout from the boolean tree
// =================================================================

const CELL_W = 80
const CELL_H = 36
const RAIL_PAD = 16
const COIL_W = 96

interface LayoutBox {
  /** Width of this sub-tree's bounding box in cells. */
  cols: number
  /** Number of horizontal rows the sub-tree occupies. */
  rows: number
}

/** Compute the (cols, rows) bounding box for a subtree without
 *  emitting SVG. Used so the renderer knows the canvas size before
 *  recursing again to draw. */
function measure(node: LdNode): LayoutBox {
  switch (node.op) {
    case "contact":
    case "const":
      return { cols: 1, rows: 1 }
    case "not":
      return measure(node.arg)
    case "and": {
      if (node.args.length === 0) return { cols: 1, rows: 1 }
      let cols = 0
      let rows = 1
      for (const a of node.args) {
        const m = measure(a)
        cols += m.cols
        rows = Math.max(rows, m.rows)
      }
      return { cols, rows }
    }
    case "or": {
      if (node.args.length === 0) return { cols: 1, rows: 1 }
      let cols = 1
      let rows = 0
      for (const a of node.args) {
        const m = measure(a)
        cols = Math.max(cols, m.cols)
        rows += m.rows
      }
      return { cols, rows }
    }
  }
}

function RungView({ rung, index }: { rung: LdRung; index: number }) {
  const inner = measure(rung.logic)
  const cols = inner.cols
  const rows = inner.rows
  const width = RAIL_PAD * 2 + cols * CELL_W + COIL_W * Math.max(rung.coils.length, 1) + 24
  const height = Math.max(rows, 1) * CELL_H + 16

  return (
    <div className="rounded-md border border-border bg-card">
      <div className="flex items-center justify-between border-b border-border bg-muted/20 px-2 py-1 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
        <span>
          rung {index} · <span className="font-mono normal-case">{rung.id}</span>
        </span>
        {rung.label && (
          <span className="normal-case tracking-normal text-foreground/80">
            {rung.label}
          </span>
        )}
      </div>
      <svg
        viewBox={`0 0 ${width} ${height}`}
        width="100%"
        className="block"
        style={{ height }}
        // Crisp 1 px strokes regardless of viewBox scaling.
        // Same trick we used for the Monitor sparklines.
        // vectorEffect lives on individual elements below.
      >
        {/* Left + right rails. Coils sit between the right end of the
            logic network and the right rail; the right rail closes off
            the rung visually like a real ladder diagram. */}
        <line
          x1={RAIL_PAD}
          y1={0}
          x2={RAIL_PAD}
          y2={height}
          className="stroke-foreground"
          strokeWidth={1}
          vectorEffect="non-scaling-stroke"
        />
        <line
          x1={width - RAIL_PAD}
          y1={0}
          x2={width - RAIL_PAD}
          y2={height}
          className="stroke-foreground"
          strokeWidth={1}
          vectorEffect="non-scaling-stroke"
        />

        {/* Network output rail — the horizontal wire just to the left of
            the first coil, where AND-series ends and any OR-branches
            have merged back. */}
        <RenderNode
          node={rung.logic}
          x={RAIL_PAD}
          y={8}
          cols={cols}
          rows={rows}
        />

        {/* Coil(s) — one per declared coil, placed to the right of the
            network. Multiple coils stack horizontally; aesthetically a
            real ladder uses a "tee" off the output wire, which we
            approximate by drawing a single horizontal wire to each. */}
        {rung.coils.map((coil, ci) => {
          const cx = RAIL_PAD + cols * CELL_W + ci * COIL_W
          const cy = 8 + ((rows - 1) * CELL_H) / 2 + CELL_H / 2
          return (
            <g key={ci}>
              <line
                x1={cx}
                y1={cy}
                x2={cx + COIL_W - 8}
                y2={cy}
                className="stroke-foreground"
                strokeWidth={1}
                vectorEffect="non-scaling-stroke"
              />
              <line
                x1={cx + COIL_W - 8}
                y1={cy}
                x2={width - RAIL_PAD}
                y2={cy}
                className="stroke-foreground"
                strokeWidth={1}
                vectorEffect="non-scaling-stroke"
              />
              <CoilGlyph coil={coil} x={cx + (COIL_W - 36) / 2} y={cy} />
            </g>
          )
        })}
      </svg>
    </div>
  )
}

function RenderNode({
  node,
  x,
  y,
  cols,
  rows,
}: {
  node: LdNode
  x: number
  y: number
  cols: number
  rows: number
}) {
  const midY = y + ((rows - 1) * CELL_H) / 2 + CELL_H / 2

  switch (node.op) {
    case "contact":
      return (
        <ContactGlyph
          x={x}
          y={midY}
          width={CELL_W}
          name={node.var}
          negated={node.negated}
        />
      )
    case "const":
      return (
        <ConstGlyph x={x} y={midY} width={CELL_W} value={node.value} />
      )
    case "not": {
      // Render the wrapped node with an outer "NOT" annotation. For a
      // bare contact, prefer the inline negated-contact glyph instead
      // — visually the same and cheaper. For deeper subtrees, wrap with
      // a dashed box labelled "NOT".
      if (node.arg.op === "contact") {
        return (
          <ContactGlyph
            x={x}
            y={midY}
            width={CELL_W}
            name={node.arg.var}
            negated={!node.arg.negated}
          />
        )
      }
      const m = measure(node.arg)
      return (
        <g>
          <RenderNode node={node.arg} x={x} y={y} cols={m.cols} rows={m.rows} />
          <rect
            x={x + 2}
            y={y + 2}
            width={m.cols * CELL_W - 4}
            height={m.rows * CELL_H - 4}
            fill="none"
            className="stroke-muted-foreground"
            strokeWidth={1}
            strokeDasharray="3 3"
            vectorEffect="non-scaling-stroke"
          />
          <text
            x={x + 4}
            y={y + 12}
            className="fill-muted-foreground"
            fontSize="9"
            fontFamily="ui-monospace, monospace"
          >
            NOT
          </text>
        </g>
      )
    }
    case "and": {
      // Lay children left-to-right, share the same vertical band.
      if (node.args.length === 0) {
        return <ConstGlyph x={x} y={midY} width={CELL_W} value={true} />
      }
      let cursor = x
      return (
        <g>
          {node.args.map((child, i) => {
            const m = measure(child)
            const out = (
              <RenderNode
                key={i}
                node={child}
                x={cursor}
                y={y}
                cols={m.cols}
                rows={rows}
              />
            )
            cursor += m.cols * CELL_W
            return out
          })}
        </g>
      )
    }
    case "or": {
      // Stack children top-to-bottom; each branch occupies its own
      // horizontal lane. Vertical short-circuit wires connect the
      // start and end of each lane back to the rail above/below.
      if (node.args.length === 0) {
        return <ConstGlyph x={x} y={midY} width={CELL_W} value={false} />
      }
      let rowCursor = 0
      const elems: React.ReactNode[] = []
      const inX = x
      const outX = x + cols * CELL_W
      // Stub-in: extend lane wires so all child sub-trees are the same
      // width as `cols` (pad shorter branches with a horizontal line).
      for (let i = 0; i < node.args.length; i++) {
        const child = node.args[i]
        const m = measure(child)
        const laneY = y + rowCursor * CELL_H
        const laneMidY = laneY + ((m.rows - 1) * CELL_H) / 2 + CELL_H / 2
        elems.push(
          <RenderNode
            key={i}
            node={child}
            x={inX}
            y={laneY}
            cols={cols}
            rows={m.rows}
          />,
        )
        // Pad the lane out to `cols` if the sub-tree is narrower.
        if (m.cols < cols) {
          elems.push(
            <line
              key={`pad-${i}`}
              x1={inX + m.cols * CELL_W}
              y1={laneMidY}
              x2={outX}
              y2={laneMidY}
              className="stroke-foreground"
              strokeWidth={1}
              vectorEffect="non-scaling-stroke"
            />,
          )
        }
        rowCursor += m.rows
      }
      // Vertical merge wires on left and right of the OR group.
      const firstY = y + CELL_H / 2
      const lastChildRows = measure(node.args[node.args.length - 1]).rows
      const lastY =
        y + (rowCursor - lastChildRows) * CELL_H + lastChildRows * CELL_H - CELL_H / 2
      elems.push(
        <line
          key="merge-in"
          x1={inX}
          y1={firstY}
          x2={inX}
          y2={lastY}
          className="stroke-foreground"
          strokeWidth={1}
          vectorEffect="non-scaling-stroke"
        />,
        <line
          key="merge-out"
          x1={outX}
          y1={firstY}
          x2={outX}
          y2={lastY}
          className="stroke-foreground"
          strokeWidth={1}
          vectorEffect="non-scaling-stroke"
        />,
      )
      return <g>{elems}</g>
    }
  }
}

/** Normally-open (`-| |-`) and normally-closed (`-|/|-`) contact. */
function ContactGlyph({
  x,
  y,
  width,
  name,
  negated,
}: {
  x: number
  y: number
  width: number
  name: string
  negated: boolean
}) {
  const cx = x + width / 2
  const half = 10
  return (
    <g>
      {/* horizontal wires */}
      <line
        x1={x}
        y1={y}
        x2={cx - half}
        y2={y}
        className="stroke-foreground"
        strokeWidth={1}
        vectorEffect="non-scaling-stroke"
      />
      <line
        x1={cx + half}
        y1={y}
        x2={x + width}
        y2={y}
        className="stroke-foreground"
        strokeWidth={1}
        vectorEffect="non-scaling-stroke"
      />
      {/* contact bars */}
      <line
        x1={cx - half}
        y1={y - 9}
        x2={cx - half}
        y2={y + 9}
        className="stroke-foreground"
        strokeWidth={1.5}
        vectorEffect="non-scaling-stroke"
      />
      <line
        x1={cx + half}
        y1={y - 9}
        x2={cx + half}
        y2={y + 9}
        className="stroke-foreground"
        strokeWidth={1.5}
        vectorEffect="non-scaling-stroke"
      />
      {negated && (
        <line
          x1={cx - half}
          y1={y + 9}
          x2={cx + half}
          y2={y - 9}
          className="stroke-foreground"
          strokeWidth={1.5}
          vectorEffect="non-scaling-stroke"
        />
      )}
      {/* label */}
      <text
        x={cx}
        y={y - 14}
        textAnchor="middle"
        className="fill-foreground"
        fontSize="10"
        fontFamily="ui-monospace, monospace"
      >
        {name}
      </text>
    </g>
  )
}

/** Always-passing (TRUE) or always-blocking (FALSE) literal. */
function ConstGlyph({
  x,
  y,
  width,
  value,
}: {
  x: number
  y: number
  width: number
  value: boolean
}) {
  return (
    <g>
      <line
        x1={x}
        y1={y}
        x2={x + width}
        y2={y}
        className={value ? "stroke-foreground" : "stroke-muted-foreground"}
        strokeWidth={1}
        strokeDasharray={value ? undefined : "4 3"}
        vectorEffect="non-scaling-stroke"
      />
      <text
        x={x + width / 2}
        y={y - 8}
        textAnchor="middle"
        className="fill-muted-foreground"
        fontSize="9"
        fontFamily="ui-monospace, monospace"
      >
        {value ? "TRUE" : "FALSE"}
      </text>
    </g>
  )
}

/** Output coil glyph. Round parens for standard `-( )-`, `S` for set,
 *  `R` for reset. Variable name is rendered above. */
function CoilGlyph({ coil, x, y }: { coil: LdCoil; x: number; y: number }) {
  const w = 36
  const r = 10
  const cx = x + w / 2
  const inner =
    coil.kind === "set" ? "S" : coil.kind === "reset" ? "R" : null
  return (
    <g>
      <text
        x={cx}
        y={y - 14}
        textAnchor="middle"
        className="fill-foreground"
        fontSize="10"
        fontFamily="ui-monospace, monospace"
      >
        {coil.var}
      </text>
      {/* parentheses: two arcs facing each other */}
      <path
        d={`M ${cx - r} ${y - r} A ${r * 1.2} ${r * 1.2} 0 0 0 ${cx - r} ${y + r}`}
        fill="none"
        className="stroke-foreground"
        strokeWidth={1.5}
        vectorEffect="non-scaling-stroke"
      />
      <path
        d={`M ${cx + r} ${y - r} A ${r * 1.2} ${r * 1.2} 0 0 1 ${cx + r} ${y + r}`}
        fill="none"
        className="stroke-foreground"
        strokeWidth={1.5}
        vectorEffect="non-scaling-stroke"
      />
      {inner && (
        <text
          x={cx}
          y={y + 4}
          textAnchor="middle"
          className="fill-foreground"
          fontSize="11"
          fontFamily="ui-monospace, monospace"
          fontWeight={600}
        >
          {inner}
        </text>
      )}
    </g>
  )
}

// =================================================================
//   JSON fallback — visible when parsing fails or as an escape hatch.
// =================================================================

function LDJsonFallback({ source }: { source: string }) {
  return (
    <pre className="flex-1 overflow-auto bg-muted/20 px-4 py-3 font-mono text-xs leading-relaxed text-foreground">
      {source}
    </pre>
  )
}

// =================================================================
//   Parsing helpers
// =================================================================

type Parsed =
  | { kind: "ok"; program: LdProgram }
  | { kind: "error"; message: string }

function parse(source: string): Parsed {
  try {
    const program = JSON.parse(source) as LdProgram
    if (!program || typeof program !== "object") {
      return { kind: "error", message: "top-level value is not an object" }
    }
    if (!Array.isArray(program.rungs)) {
      return { kind: "error", message: "missing `rungs` array" }
    }
    return { kind: "ok", program }
  } catch (e) {
    return { kind: "error", message: String(e) }
  }
}
