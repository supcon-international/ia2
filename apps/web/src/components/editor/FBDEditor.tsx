/**
 * Function Block Diagram (FBD) — read-only renderer.
 *
 * Parses `.fbd.json` source, runs a tiny layered auto-layout (no
 * external graph library — we don't need react-flow's drag/zoom for
 * read-only viewing), and draws blocks + wires as plain SVG.
 *
 * Authoring is still JSON-only at this phase (Phase 3b in
 * `MEMORY/graphical-languages.md`). When we add interactive editing
 * (Phase 3c) we'll bring in react-flow + dagre then — adding a 100KB
 * dependency for a viewer would violate the principles doc.
 *
 * Online mode (live FB output coloring) reuses the same "look up
 * `<instance>.<pin>` in the live snapshot" trick as LD's FbCall
 * rendering.
 */

import { useEffect, useMemo, useState } from "react"

import { checkProgram } from "@/lib/api"
import { cn } from "@/lib/utils"
import { useRuntime } from "@/state/runtime"
import type { CheckDiagnostic } from "@/types/generated/CheckDiagnostic"
import type { FbdBlock } from "@/types/generated/FbdBlock"
import type { FbdInputBinding } from "@/types/generated/FbdInputBinding"
import type { FbdProgram } from "@/types/generated/FbdProgram"
import type { FbdLocation } from "@/types/generated/FbdLocation"

// =================================================================
//   Top-level controlled component
// =================================================================

export function FBDEditor({
  value,
  onChange: _onChange,
  className,
}: {
  value: string
  /** Reserved for the editing phase (3c). Currently a no-op — FBD is
   *  authoring-via-JSON only at the moment. The viewer is read-only. */
  onChange: (next: string) => void
  className?: string
}) {
  const parsed = useMemo(() => safeParse(value), [value])

  // Live values for online-mode coloring. FBD outputs are normally
  // observable through their instance variable (e.g. `myT.Q`), so the
  // same key-lookup trick LD uses works here too.
  const { lastSnapshot, isRunning } = useRuntime()
  const liveValues = useMemo<{ bools: Record<string, boolean> } | null>(() => {
    if (!isRunning || !lastSnapshot) return null
    const bools: Record<string, boolean> = {}
    for (const v of lastSnapshot.vars) {
      if (v.type_name === "BOOL") {
        bools[v.name] = v.value === "TRUE"
      }
    }
    return { bools }
  }, [lastSnapshot, isRunning])

  // ---- Diagnostics (parallels LDEditor) ----
  const [diagnostics, setDiagnostics] = useState<CheckDiagnostic[]>([])
  useEffect(() => {
    if (parsed.kind === "error") {
      setDiagnostics([])
      return
    }
    const handle = setTimeout(async () => {
      try {
        const diags = await checkProgram(value, "fbd")
        setDiagnostics(diags)
      } catch (e) {
        console.warn("FBD diagnostics fetch failed:", e)
      }
    }, 350)
    return () => clearTimeout(handle)
  }, [value, parsed.kind])

  if (parsed.kind === "error") {
    return (
      <div className={cn("flex h-full min-h-0 flex-col", className)}>
        <div className="border-b border-destructive/40 bg-destructive/5 px-3 py-2 text-xs text-destructive">
          FBD JSON parse error: {parsed.message}
        </div>
        <pre className="flex-1 overflow-auto bg-muted/20 px-4 py-3 font-mono text-xs leading-relaxed text-foreground">
          {value}
        </pre>
      </div>
    )
  }

  const prog = parsed.program
  const layout = useMemo(() => layoutBlocks(prog), [prog])

  // Index diagnostics by block_id / variable for quick lookup.
  const diagIndex = useMemo(() => indexDiagnostics(diagnostics), [diagnostics])

  return (
    <div className={cn("flex h-full min-h-0 flex-col", className)}>
      <Header prog={prog} />
      {diagnostics.length > 0 && (
        <DiagnosticsBanner diagnostics={diagnostics} />
      )}
      <div className="flex-1 overflow-auto bg-background">
        <VariablePanel prog={prog} diagIndex={diagIndex} />
        <div className="p-4">
          <FbdCanvas
            prog={prog}
            layout={layout}
            liveValues={liveValues}
            diagIndex={diagIndex}
          />
        </div>
        <div className="px-4 pb-4 text-[11px] text-muted-foreground">
          <p>
            FBD viewer is read-only — author by editing the JSON
            directly. The full editor lands with phase 3c (drag-to-
            place blocks + wire-drawing).
          </p>
        </div>
      </div>
    </div>
  )
}

// =================================================================
//   Header
// =================================================================

function Header({ prog }: { prog: FbdProgram }) {
  return (
    <div className="border-b border-border bg-muted/30 px-3 py-1.5 text-[11px] uppercase tracking-wider text-muted-foreground">
      <span className="font-mono normal-case tracking-normal text-foreground">
        {prog.name}
      </span>
      <span className="ml-2 rounded border border-border bg-muted/50 px-1.5 py-0.5 font-mono text-[9px] text-muted-foreground">
        fbd
      </span>
      <span className="ml-2 rounded border border-border bg-muted/50 px-1.5 py-0.5 font-mono text-[9px] text-muted-foreground">
        {prog.pou_type === "function_block" ? "fb" : "prg"}
      </span>
      <span className="ml-3">
        {prog.blocks.length} block{prog.blocks.length === 1 ? "" : "s"} ·{" "}
        {prog.outputs.length} output binding{prog.outputs.length === 1 ? "" : "s"}
      </span>
    </div>
  )
}

// =================================================================
//   Variable panel
// =================================================================

function VariablePanel({
  prog,
  diagIndex,
}: {
  prog: FbdProgram
  diagIndex: DiagIndex
}) {
  const groups: Array<{ label: string; section: "input" | "output" | "internal" }> = [
    { label: "VAR_INPUT", section: "input" },
    { label: "VAR_OUTPUT", section: "output" },
    { label: "VAR", section: "internal" },
  ]
  return (
    <div className="grid grid-cols-3 gap-3 border-b border-border bg-muted/10 px-4 py-2 text-[11px]">
      {groups.map((g) => {
        const vs = prog.variables.filter((v) => v.section === g.section)
        return (
          <div key={g.section}>
            <div className="mb-1 font-mono text-[9px] uppercase tracking-wider text-muted-foreground">
              {g.label}
            </div>
            <ul className="space-y-0.5">
              {vs.length === 0 && (
                <li className="text-muted-foreground italic">—</li>
              )}
              {vs.map((v) => (
                <li
                  key={v.name}
                  className={cn(
                    "flex items-center gap-1 rounded px-1 font-mono",
                    diagIndex.byVariable.has(v.name) && "ring-1 ring-destructive/60",
                  )}
                  title={diagIndex.byVariable.get(v.name)?.[0]?.message ?? undefined}
                >
                  <span className="text-foreground">{v.name}</span>
                  <span className="text-muted-foreground">{v.type}</span>
                  {v.init !== null && v.init !== undefined && (
                    <span className="text-muted-foreground">:= {v.init}</span>
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
//   Layout — layered, no external libraries
// =================================================================

interface BlockLayout {
  /** Layer 0 = no Block-typed input edges. */
  layer: number
  /** Position within the layer (0-indexed). */
  row: number
  /** Pixel x, derived from layer. */
  x: number
  /** Pixel y, derived from row. */
  y: number
  /** Pixel width — function of pin count. */
  w: number
  /** Pixel height — function of max(input pins, 1). */
  h: number
}

const BLOCK_W = 160
const PIN_ROW_H = 18
const HEADER_H = 26
const LAYER_GAP = 80
const ROW_GAP = 20
const CANVAS_PAD = 24

interface FbdLayout {
  blockById: Map<string, BlockLayout>
  width: number
  height: number
}

/**
 * Compute a layered layout: Kahn's-style topo-pass assigns each block
 * to a layer = 1 + max(layer of its block-typed predecessors). Within
 * a layer, blocks are ordered by their position in the source JSON
 * (preserves authoring intent without a real graph layout library).
 *
 * Cycles render with all-cyclic blocks dumped in layer 0 — the
 * transpiler will reject the file anyway, but we don't want the
 * viewer to throw mid-render.
 */
function layoutBlocks(prog: FbdProgram): FbdLayout {
  const n = prog.blocks.length
  const layer = new Array<number>(n).fill(0)
  const idToIdx = new Map<string, number>()
  prog.blocks.forEach((b, i) => idToIdx.set(b.id, i))

  // Iterate to a fixed point. n iterations is the worst case (DAG
  // depth ≤ n); each pass extends a block's layer to one past the
  // deepest predecessor seen so far.
  for (let pass = 0; pass < n; pass++) {
    let changed = false
    for (let i = 0; i < n; i++) {
      let deepest = -1
      for (const input of prog.blocks[i].inputs) {
        if (input.value.kind === "block") {
          const src = idToIdx.get(input.value.block_id)
          if (src !== undefined && src !== i) {
            deepest = Math.max(deepest, layer[src])
          }
        }
      }
      const want = deepest + 1
      if (want > layer[i]) {
        layer[i] = want
        changed = true
      }
    }
    if (!changed) break
  }

  // Group by layer, in authoring order.
  const byLayer = new Map<number, number[]>()
  for (let i = 0; i < n; i++) {
    const l = layer[i]
    if (!byLayer.has(l)) byLayer.set(l, [])
    byLayer.get(l)!.push(i)
  }

  // Compute heights based on max(input pin count, 1).
  const heights = prog.blocks.map(
    (b) => HEADER_H + Math.max(b.inputs.length, 1) * PIN_ROW_H + 8,
  )

  const blockById = new Map<string, BlockLayout>()
  let maxLayer = 0
  let maxRowYByLayer = 0
  for (const [l, members] of byLayer.entries()) {
    let cursorY = CANVAS_PAD
    members.forEach((i, row) => {
      const h = heights[i]
      blockById.set(prog.blocks[i].id, {
        layer: l,
        row,
        x: CANVAS_PAD + l * (BLOCK_W + LAYER_GAP),
        y: cursorY,
        w: BLOCK_W,
        h,
      })
      cursorY += h + ROW_GAP
    })
    maxLayer = Math.max(maxLayer, l)
    maxRowYByLayer = Math.max(maxRowYByLayer, cursorY)
  }

  const width = CANVAS_PAD * 2 + (maxLayer + 1) * BLOCK_W + maxLayer * LAYER_GAP
  const height = Math.max(maxRowYByLayer + CANVAS_PAD, 200)

  return { blockById, width, height }
}

// =================================================================
//   Canvas
// =================================================================

function FbdCanvas({
  prog,
  layout,
  liveValues,
  diagIndex,
}: {
  prog: FbdProgram
  layout: FbdLayout
  liveValues: { bools: Record<string, boolean> } | null
  diagIndex: DiagIndex
}) {
  // Wires fan out from each block's `Q` (or chosen output pin) on its
  // right edge into the dependent block's pin on its left edge.
  return (
    <svg
      viewBox={`0 0 ${layout.width} ${layout.height}`}
      width={layout.width}
      height={layout.height}
      className="block max-w-full"
    >
      {/* Render wires BEFORE blocks so block borders sit on top. */}
      {prog.blocks.flatMap((b, i) =>
        b.inputs
          .filter((inp) => inp.value.kind === "block")
          .map((inp, j) => {
            const src = inp.value
            if (src.kind !== "block") return null
            const from = layout.blockById.get(src.block_id)
            const to = layout.blockById.get(b.id)
            if (!from || !to) return null
            const sourcePinIdx = approxOutputPinIndex(prog.blocks, src.block_id, src.pin)
            const targetPinIdx = b.inputs.findIndex((x) => x.pin === inp.pin)
            const x1 = from.x + from.w
            const y1 = from.y + HEADER_H + sourcePinIdx * PIN_ROW_H + PIN_ROW_H / 2
            const x2 = to.x
            const y2 = to.y + HEADER_H + targetPinIdx * PIN_ROW_H + PIN_ROW_H / 2
            // Bezier-ish curve: horizontal exit + horizontal entry.
            const dx = Math.max(20, (x2 - x1) / 2)
            const d = `M ${x1} ${y1} C ${x1 + dx} ${y1}, ${x2 - dx} ${y2}, ${x2} ${y2}`
            // Power state of the wire = the source instance.pin value.
            const srcBlock = prog.blocks.find((bb) => bb.id === src.block_id)
            const wireKey = srcBlock ? `${srcBlock.instance}.${src.pin}` : null
            const powered =
              liveValues && wireKey
                ? liveValues.bools[wireKey] === true
                : null
            return (
              <path
                key={`wire-${i}-${j}`}
                d={d}
                fill="none"
                strokeWidth={1.5}
                vectorEffect="non-scaling-stroke"
                className={powerClass(powered)}
              />
            )
          }),
      )}

      {prog.blocks.map((b) => (
        <BlockGlyph
          key={b.id}
          block={b}
          layout={layout.blockById.get(b.id)!}
          liveValues={liveValues}
          hasError={diagIndex.byBlock.has(b.id)}
        />
      ))}

      {/* Output bindings rendered as labelled stub wires off the right side. */}
      {prog.outputs.map((o, i) => {
        const from = layout.blockById.get(o.from_block)
        if (!from) return null
        const srcPinIdx = approxOutputPinIndex(prog.blocks, o.from_block, o.from_pin)
        const x1 = from.x + from.w
        const y1 = from.y + HEADER_H + srcPinIdx * PIN_ROW_H + PIN_ROW_H / 2
        const x2 = x1 + 40
        const srcBlock = prog.blocks.find((b) => b.id === o.from_block)
        const wireKey = srcBlock ? `${srcBlock.instance}.${o.from_pin}` : null
        const powered =
          liveValues && wireKey ? liveValues.bools[wireKey] === true : null
        return (
          <g key={`out-${i}`}>
            <line
              x1={x1}
              y1={y1}
              x2={x2}
              y2={y1}
              strokeWidth={1.5}
              vectorEffect="non-scaling-stroke"
              className={powerClass(powered)}
            />
            <text
              x={x2 + 4}
              y={y1 + 3}
              className="fill-foreground"
              fontSize="11"
              fontFamily="ui-monospace, monospace"
            >
              → {o.variable}
            </text>
          </g>
        )
      })}
    </svg>
  )
}

function BlockGlyph({
  block,
  layout,
  liveValues,
  hasError,
}: {
  block: FbdBlock
  layout: BlockLayout
  liveValues: { bools: Record<string, boolean> } | null
  hasError: boolean
}) {
  return (
    <g>
      {/* Box */}
      <rect
        x={layout.x}
        y={layout.y}
        width={layout.w}
        height={layout.h}
        rx={4}
        className={cn(
          "fill-card",
          hasError ? "stroke-destructive" : "stroke-foreground",
        )}
        strokeWidth={hasError ? 2 : 1.5}
        vectorEffect="non-scaling-stroke"
      />
      {/* Header strip */}
      <rect
        x={layout.x}
        y={layout.y}
        width={layout.w}
        height={HEADER_H}
        rx={4}
        className={hasError ? "fill-destructive/15" : "fill-muted/40"}
      />
      <text
        x={layout.x + layout.w / 2}
        y={layout.y + 16}
        textAnchor="middle"
        className="fill-foreground"
        fontSize="12"
        fontFamily="ui-monospace, monospace"
      >
        {block.instance}
        <tspan className="fill-muted-foreground"> : {block.fb_type}</tspan>
      </text>

      {/* Input pins (left) */}
      {(block.inputs.length > 0 ? block.inputs : [null]).map((input, i) => {
        const y = layout.y + HEADER_H + i * PIN_ROW_H + PIN_ROW_H / 2
        if (!input) {
          return (
            <text
              key="empty"
              x={layout.x + 8}
              y={y + 3}
              className="fill-muted-foreground"
              fontSize="10"
              fontFamily="ui-monospace, monospace"
            >
              (no inputs)
            </text>
          )
        }
        return <PinRow key={input.pin} input={input} layout={layout} y={y} />
      })}

      {/* Output pin marker on the right edge, vertically centered. */}
      <circle
        cx={layout.x + layout.w}
        cy={layout.y + layout.h / 2}
        r={3}
        className={powerClass(
          liveValues
            ? liveValues.bools[`${block.instance}.Q`] === true
            : null,
        ).replace("stroke-", "fill-")}
      />
    </g>
  )
}

function PinRow({
  input,
  layout,
  y,
}: {
  input: FbdInputBinding
  layout: BlockLayout
  y: number
}) {
  const text =
    input.value.kind === "var"
      ? input.value.name
      : input.value.kind === "literal"
        ? input.value.value
        : `${input.value.block_id}.${input.value.pin}`
  return (
    <g>
      <circle
        cx={layout.x}
        cy={y}
        r={2}
        className="fill-foreground"
      />
      <text
        x={layout.x + 8}
        y={y + 3}
        className="fill-foreground"
        fontSize="10"
        fontFamily="ui-monospace, monospace"
      >
        {input.pin}:
        <tspan className="fill-muted-foreground">
          {" "}
          {text}
        </tspan>
      </text>
    </g>
  )
}

// =================================================================
//   Diagnostics
// =================================================================

interface DiagIndex {
  byBlock: Map<string, CheckDiagnostic[]>
  byVariable: Map<string, CheckDiagnostic[]>
}

function indexDiagnostics(diags: CheckDiagnostic[]): DiagIndex {
  const byBlock = new Map<string, CheckDiagnostic[]>()
  const byVariable = new Map<string, CheckDiagnostic[]>()
  for (const d of diags) {
    const loc = d.fbd_location
    if (!loc) continue
    if (loc.kind === "block") {
      const list = byBlock.get(loc.block_id) ?? []
      list.push(d)
      byBlock.set(loc.block_id, list)
    } else if (loc.kind === "variable") {
      const list = byVariable.get(loc.name) ?? []
      list.push(d)
      byVariable.set(loc.name, list)
    }
  }
  return { byBlock, byVariable }
}

function describeLocation(loc: FbdLocation | null | undefined): string {
  if (!loc) return "—"
  switch (loc.kind) {
    case "variable":
      return `var ${loc.name}`
    case "block":
      return `block ${loc.block_id}`
    case "output":
      return `output ${loc.variable}`
  }
}

function DiagnosticsBanner({ diagnostics }: { diagnostics: CheckDiagnostic[] }) {
  return (
    <div className="border-b border-destructive/30 bg-destructive/5 text-xs">
      <div className="flex items-center gap-2 px-3 py-1.5">
        <span className="font-mono font-medium text-destructive">
          {diagnostics.length} {diagnostics.length === 1 ? "error" : "errors"}
        </span>
      </div>
      <ul className="divide-y divide-destructive/15">
        {diagnostics.slice(0, 8).map((d, i) => (
          <li key={i} className="flex items-start gap-2 px-3 py-1">
            <span className="font-mono text-[10px] text-destructive">
              {d.code}
            </span>
            <span className="flex-1 text-foreground">{d.message}</span>
            <span className="font-mono text-[10px] text-muted-foreground">
              {describeLocation(d.fbd_location)}
            </span>
          </li>
        ))}
        {diagnostics.length > 8 && (
          <li className="px-3 py-1 text-muted-foreground">
            +{diagnostics.length - 8} more…
          </li>
        )}
      </ul>
    </div>
  )
}

// =================================================================
//   Helpers
// =================================================================

/** Approximate where a given output pin sits vertically on the block.
 *  We don't (yet) track output pins individually in the layout, so we
 *  centre wires on the block; this is fine for FBs with a single BOOL
 *  output (the common case). Refinement deferred until we draw
 *  multiple output pins explicitly. */
function approxOutputPinIndex(
  blocks: FbdBlock[],
  blockId: string,
  _pin: string,
): number {
  const b = blocks.find((bb) => bb.id === blockId)
  if (!b) return 0
  // Centre vertically: place wire at the middle pin row.
  return Math.max(0, Math.floor(b.inputs.length / 2))
}

function powerClass(powered: boolean | null): string {
  if (powered === null) return "stroke-foreground"
  return powered ? "stroke-highlight" : "stroke-muted-foreground/40"
}

type Parsed =
  | { kind: "ok"; program: FbdProgram }
  | { kind: "error"; message: string }

function safeParse(source: string): Parsed {
  try {
    const obj = JSON.parse(source)
    // Tiny shape check; the real validation lives in `cs check` /
    // the server. Anything that mostly looks like an FbdProgram
    // gets through here.
    if (!obj || typeof obj !== "object" || !Array.isArray(obj.blocks)) {
      return { kind: "error", message: "missing `blocks` array" }
    }
    return { kind: "ok", program: obj as FbdProgram }
  } catch (e) {
    return { kind: "error", message: String(e) }
  }
}
