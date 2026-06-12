/**
 * Function Block Diagram (FBD) — editor.
 *
 * Editing capabilities (phase 3c):
 *
 *  - Drag a block's header to reposition it. Positions persist to
 *    `FbdBlock.position` — saving the file = saving the layout. That
 *    is also the "phase 4 / CFC" deliverable: no separate format,
 *    just position metadata in the same JSON.
 *  - "+ Block" toolbar picker adds a new FB instance at the canvas
 *    centre. The block can then be dragged anywhere.
 *  - Selecting a block opens a detail bar at the bottom (same UX
 *    pattern as LDEditor) for editing fb_type / instance / per-pin
 *    inputs and for deletion.
 *  - Wires are drawn whenever a pin's input is `kind: "block"`. To
 *    create / break a wire today the user switches a pin between
 *    `var | literal | block` in the OperandPicker — the dedicated
 *    drag-from-port-to-port gesture is the next follow-up.
 *
 * Why no react-flow:
 *  - LDEditor's pointer-events approach already covers everything we
 *    need (drag, select, click-to-edit).
 *  - Skipping a 100KB dependency keeps with the principles doc.
 *  - We have full control over the SVG so colouring / online state
 *    integration stays identical to LD.
 */

import { Plus, Trash2, X } from "lucide-react"
import { useEffect, useMemo, useRef, useState } from "react"

import { checkProgram } from "@/lib/api"
import { DiagnosticsBanner } from "@/components/editor/DiagnosticsBanner"
import {
  addBlock,
  blockBoolOutputs,
  connectWire,
  parseProgram,
  removeBlock,
  removeOutputBinding,
  serializeProgram,
  setBlockFbType,
  setBlockInput,
  setBlockInstance,
  setBlockPosition,
  setOutputBinding,
} from "@/lib/fbd-edit"
import { fbByType, fbInputs, groupedFbs } from "@/lib/ld-fbs"
import { cn } from "@/lib/utils"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { useRuntime } from "@/state/runtime"
import type { CheckDiagnostic } from "@/types/generated/CheckDiagnostic"
import type { FbdBlock } from "@/types/generated/FbdBlock"
import type { FbdInputSource } from "@/types/generated/FbdInputSource"
import type { FbdLocation } from "@/types/generated/FbdLocation"
import type { FbdProgram } from "@/types/generated/FbdProgram"

// =================================================================
//   Wire-drag state (port-to-port connection gesture)
// =================================================================

/** Snapshot of an in-flight wire-drag. `fromBlockId` / `fromPin` are
 *  the source endpoint locked at drag-start; `currentX` / `currentY`
 *  follow the cursor in SVG-space and drive the preview line. */
interface WireDrag {
  fromBlockId: string
  fromPin: string
  /** Source-end SVG coordinate (right edge of the source pin's dot). */
  fromX: number
  fromY: number
  /** Cursor SVG coordinate, updated on every pointermove. */
  currentX: number
  currentY: number
}

/** Target endpoint of a wire-drag drop. */
interface WireDropTarget {
  blockId: string
  pin: string
}

// =================================================================
//   Layout constants — also used by the drag math
// =================================================================

const BLOCK_W = 160
const PIN_ROW_H = 18
const HEADER_H = 26
const LAYER_GAP = 80
const ROW_GAP = 20
const CANVAS_PAD = 24
/** Length of the connector stub that pokes out from the block edge,
 *  in SVG units. Wires terminate at the outer end of the stub (the
 *  dot), not at the block edge — same as Codesys / TIA. */
const STUB_LEN = 8

// =================================================================
//   Top-level component
// =================================================================

export function FBDEditor({
  value,
  onChange,
  className,
  readOnly = false,
  path,
}: {
  value: string
  onChange: (next: string) => void
  className?: string
  readOnly?: boolean
  /** Store slug this buffer came from — keeps the project-aware check
   *  from double-counting the on-disk copy. */
  path?: string
}) {
  const parsed = useMemo(() => safeParse(value), [value])

  // Live values for online-mode wire coloring.
  const { lastSnapshot, isRunning, projectEpoch } = useRuntime()
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

  // ---- Diagnostics ----
  const [diagnostics, setDiagnostics] = useState<CheckDiagnostic[]>([])
  useEffect(() => {
    if (parsed.kind === "error") {
      setDiagnostics([])
      return
    }
    const handle = setTimeout(async () => {
      try {
        const diags = await checkProgram(value, "fbd", path)
        setDiagnostics(diags)
      } catch (e) {
        console.warn("FBD diagnostics fetch failed:", e)
      }
    }, 350)
    return () => clearTimeout(handle)
    // projectEpoch: a library import/remove can (un)resolve this POU's
    // FB references without the buffer changing — re-check.
  }, [value, parsed.kind, path, projectEpoch])

  // ---- Selection ----
  const [selectedBlockId, setSelectedBlockId] = useState<string | null>(null)

  // ---- Wire-drag (port-to-port) state ----
  // Set when the user pointer-downs on a block's output dot. While
  // active, the canvas paints a preview line from the source pin
  // following the cursor; on pointerup we look for a target input
  // pin near the release point and either connect or cancel.
  const [wireDrag, setWireDrag] = useState<WireDrag | null>(null)

  // Reset selection if the source changed externally.
  useEffect(() => {
    setSelectedBlockId(null)
    setWireDrag(null)
  }, [value])

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
  const diagIndex = useMemo(() => indexDiagnostics(diagnostics), [diagnostics])

  const commit = (next: FbdProgram) => {
    if (readOnly) return
    onChange(serializeProgram(next))
  }

  const selectedBlock = selectedBlockId
    ? prog.blocks.find((b) => b.id === selectedBlockId) ?? null
    : null

  return (
    <div className={cn("flex h-full min-h-0 flex-col", className)}>
      <Header prog={prog} />
      <Toolbar
        readOnly={readOnly}
        onAdd={(fbType) => {
          // Drop new blocks somewhere visible — middle of the canvas.
          const cx = layout.width / 2
          const cy = layout.height / 2
          const { prog: next, blockId } = addBlock(prog, fbType, {
            x: Math.round(cx - BLOCK_W / 2),
            y: Math.round(cy - 30),
          })
          commit(next)
          setSelectedBlockId(blockId)
        }}
      />
      {diagnostics.length > 0 && (
        <DiagnosticsBanner
          diagnostics={diagnostics}
          formatLocation={(d) => describeLocation(d.fbd_location)}
        />
      )}
      <div className="flex-1 overflow-auto bg-background">
        <VariablePanel prog={prog} diagIndex={diagIndex} />
        <div className="p-4">
          <FbdCanvas
            prog={prog}
            layout={layout}
            liveValues={liveValues}
            diagIndex={diagIndex}
            selectedBlockId={selectedBlockId}
            readOnly={readOnly}
            wireDrag={wireDrag}
            onSelectBlock={setSelectedBlockId}
            onMoveBlock={(id, pos) => commit(setBlockPosition(prog, id, pos))}
            onStartWireDrag={setWireDrag}
            onUpdateWireDrag={(pos) =>
              setWireDrag((d) => (d ? { ...d, currentX: pos.x, currentY: pos.y } : d))
            }
            onEndWireDrag={(target) => {
              if (wireDrag && target) {
                commit(
                  connectWire(
                    prog,
                    target.blockId,
                    target.pin,
                    wireDrag.fromBlockId,
                    wireDrag.fromPin,
                  ),
                )
              }
              setWireDrag(null)
            }}
          />
        </div>
      </div>
      {selectedBlock && !readOnly && (
        <BlockDetail
          prog={prog}
          block={selectedBlock}
          onCommit={commit}
          onClose={() => setSelectedBlockId(null)}
          onDelete={() => {
            commit(removeBlock(prog, selectedBlock.id))
            setSelectedBlockId(null)
          }}
        />
      )}
      {!selectedBlock && prog.outputs.length > 0 && !readOnly && (
        <OutputsBar
          prog={prog}
          onCommit={commit}
          onClose={() => {}}
        />
      )}
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
        {prog.outputs.length} output binding
        {prog.outputs.length === 1 ? "" : "s"}
      </span>
    </div>
  )
}

// =================================================================
//   Toolbar — "+ Block" picker
// =================================================================

function Toolbar({
  readOnly,
  onAdd,
}: {
  readOnly: boolean
  onAdd: (fbType: string) => void
}) {
  if (readOnly) return null
  return (
    <div className="flex items-center gap-2 border-b border-border bg-muted/10 px-3 py-1 text-xs">
      <Select
        value=""
        onValueChange={(v) => {
          if (v) onAdd(v)
        }}
      >
        <SelectTrigger
          className="h-7 gap-1 px-2 text-xs"
          title="Insert a new function block"
          aria-label="Insert a new function block"
        >
          <Plus className="size-3" />
          Block
        </SelectTrigger>
        <SelectContent>
          {groupedFbs().map((group) => (
            <SelectGroup key={group.label}>
              <SelectLabel>{group.label}</SelectLabel>
              {group.fbs.map((fb) => (
                <SelectItem key={fb.type} value={fb.type}>
                  <span className="font-mono">{fb.type}</span>
                  <span className="ml-2 text-muted-foreground">
                    {fb.label.replace(`${fb.type} — `, "")}
                  </span>
                </SelectItem>
              ))}
            </SelectGroup>
          ))}
        </SelectContent>
      </Select>
      <span className="text-[10px] text-muted-foreground">
        drag a block's header to reposition · click a block to edit
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
                    diagIndex.byVariable.has(v.name) &&
                      "ring-1 ring-destructive/60",
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
  x: number
  y: number
  w: number
  h: number
}

interface FbdLayout {
  blockById: Map<string, BlockLayout>
  width: number
  height: number
}

function layoutBlocks(prog: FbdProgram): FbdLayout {
  // Phase 1: layered defaults (same as the read-only viewer used).
  const fallback = layeredDefaults(prog)

  // Phase 2: any block with a saved `position` overrides its fallback.
  // This lets newly-added blocks land at the explicit drop point
  // immediately, and survive reloads.
  const blockById = new Map<string, BlockLayout>()
  let maxX = 0
  let maxY = 0
  for (const block of prog.blocks) {
    const def = fallback.get(block.id)!
    const pos = block.position
    const layout: BlockLayout = {
      x: pos ? pos.x : def.x,
      y: pos ? pos.y : def.y,
      w: def.w,
      h: def.h,
    }
    blockById.set(block.id, layout)
    maxX = Math.max(maxX, layout.x + layout.w)
    maxY = Math.max(maxY, layout.y + layout.h)
  }

  // Leave a margin for output-binding stubs on the right.
  const width = Math.max(maxX + CANVAS_PAD + 160, 600)
  const height = Math.max(maxY + CANVAS_PAD, 240)

  return { blockById, width, height }
}

function layeredDefaults(prog: FbdProgram): Map<string, BlockLayout> {
  const n = prog.blocks.length
  const layer = new Array<number>(n).fill(0)
  const idToIdx = new Map<string, number>()
  prog.blocks.forEach((b, i) => idToIdx.set(b.id, i))

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

  const heights = prog.blocks.map(
    (b) => HEADER_H + Math.max(b.inputs.length, 1) * PIN_ROW_H + 8,
  )

  const result = new Map<string, BlockLayout>()
  for (const [l, members] of byLayer.entries()) {
    let cursorY = CANVAS_PAD
    members.forEach((i) => {
      const h = heights[i]
      result.set(prog.blocks[i].id, {
        x: CANVAS_PAD + l * (BLOCK_W + LAYER_GAP),
        y: cursorY,
        w: BLOCK_W,
        h,
      })
      cursorY += h + ROW_GAP
    })
  }
  return result
}

// =================================================================
//   Canvas
// =================================================================

function FbdCanvas({
  prog,
  layout,
  liveValues,
  diagIndex,
  selectedBlockId,
  readOnly,
  wireDrag,
  onSelectBlock,
  onMoveBlock,
  onStartWireDrag,
  onUpdateWireDrag,
  onEndWireDrag,
}: {
  prog: FbdProgram
  layout: FbdLayout
  liveValues: { bools: Record<string, boolean> } | null
  diagIndex: DiagIndex
  selectedBlockId: string | null
  readOnly: boolean
  wireDrag: WireDrag | null
  onSelectBlock: (id: string | null) => void
  onMoveBlock: (id: string, pos: { x: number; y: number }) => void
  onStartWireDrag: (drag: WireDrag) => void
  onUpdateWireDrag: (pos: { x: number; y: number }) => void
  onEndWireDrag: (target: WireDropTarget | null) => void
}) {
  // Block drag state — same as before.
  const dragRef = useRef<{
    id: string
    origX: number
    origY: number
    startClientX: number
    startClientY: number
    scale: number
  } | null>(null)
  const svgRef = useRef<SVGSVGElement | null>(null)

  /** Convert a client-space (px) point to SVG-space (layout units),
   *  using the viewBox-to-rendered-size scale. */
  const clientToSvg = (clientX: number, clientY: number): { x: number; y: number } => {
    const svg = svgRef.current
    if (!svg) return { x: clientX, y: clientY }
    const rect = svg.getBoundingClientRect()
    const scaleX = layout.width / rect.width
    const scaleY = layout.height / rect.height
    return {
      x: (clientX - rect.left) * scaleX,
      y: (clientY - rect.top) * scaleY,
    }
  }

  const onCanvasPointerMove = (e: React.PointerEvent) => {
    // Block drag takes precedence (it's the more common gesture).
    const drag = dragRef.current
    if (drag) {
      const dx = (e.clientX - drag.startClientX) / drag.scale
      const dy = (e.clientY - drag.startClientY) / drag.scale
      onMoveBlock(drag.id, {
        x: Math.round(drag.origX + dx),
        y: Math.round(drag.origY + dy),
      })
      return
    }
    if (wireDrag) {
      const p = clientToSvg(e.clientX, e.clientY)
      onUpdateWireDrag(p)
    }
  }

  /** Attempt to find an input pin dot near the cursor on pointerup.
   *  Returns the {blockId, pin} pair if hit within HIT_RADIUS px, or
   *  null otherwise. Iterates all input pins of all blocks; n is small
   *  enough this is fine. */
  const findDropTarget = (svgX: number, svgY: number): WireDropTarget | null => {
    const HIT_RADIUS = 14 // forgiving by design — small dots need a big target
    for (const block of prog.blocks) {
      // Don't self-loop
      if (wireDrag && block.id === wireDrag.fromBlockId) continue
      const lay = layout.blockById.get(block.id)
      if (!lay) return null
      for (let i = 0; i < Math.max(block.inputs.length, 1); i++) {
        const input = block.inputs[i]
        if (!input) continue
        // Hit-test against the stub dot (outside the left edge), where
        // the user expects the connection point to be.
        const x = lay.x - STUB_LEN
        const y = lay.y + HEADER_H + i * PIN_ROW_H + PIN_ROW_H / 2
        const d = Math.hypot(svgX - x, svgY - y)
        if (d <= HIT_RADIUS) {
          return { blockId: block.id, pin: input.pin }
        }
      }
    }
    return null
  }

  const onCanvasPointerUp = (e: React.PointerEvent) => {
    // End block drag.
    dragRef.current = null
    // Resolve wire drag.
    if (wireDrag) {
      const p = clientToSvg(e.clientX, e.clientY)
      const target = findDropTarget(p.x, p.y)
      onEndWireDrag(target)
    }
  }

  const endDrag = () => {
    dragRef.current = null
    if (wireDrag) onEndWireDrag(null)
  }

  return (
    <svg
      ref={svgRef}
      viewBox={`0 0 ${layout.width} ${layout.height}`}
      width={layout.width}
      height={layout.height}
      className="block max-w-full select-none"
      onPointerMove={onCanvasPointerMove}
      onPointerUp={onCanvasPointerUp}
      onPointerCancel={endDrag}
      onPointerLeave={endDrag}
      onMouseDown={(e) => {
        // Clicking blank canvas deselects.
        if (e.target === e.currentTarget) onSelectBlock(null)
      }}
    >
      {/* Wires first so block borders draw on top. */}
      {prog.blocks.flatMap((b, i) =>
        b.inputs
          .filter((inp) => inp.value.kind === "block")
          .map((inp, j) => {
            const src = inp.value
            if (src.kind !== "block") return null
            const from = layout.blockById.get(src.block_id)
            const to = layout.blockById.get(b.id)
            if (!from || !to) return null
            const targetPinIdx = b.inputs.findIndex((x) => x.pin === inp.pin)
            const sourcePinIdx = approxOutputPinIndex(prog.blocks, src.block_id, src.pin)
            // Wires terminate at the STUB dots, not at the block edge.
            const x1 = from.x + from.w + STUB_LEN
            const y1 = from.y + HEADER_H + sourcePinIdx * PIN_ROW_H + PIN_ROW_H / 2
            const x2 = to.x - STUB_LEN
            const y2 = to.y + HEADER_H + targetPinIdx * PIN_ROW_H + PIN_ROW_H / 2
            const dx = Math.max(20, (x2 - x1) / 2)
            const d = `M ${x1} ${y1} C ${x1 + dx} ${y1}, ${x2 - dx} ${y2}, ${x2} ${y2}`
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
          selected={selectedBlockId === b.id}
          readOnly={readOnly}
          onPointerDown={(e) => {
            e.stopPropagation()
            onSelectBlock(b.id)
            if (readOnly) return
            const svg = svgRef.current
            const rect = svg?.getBoundingClientRect()
            const scale = svg && rect ? rect.width / layout.width : 1
            const lay = layout.blockById.get(b.id)!
            dragRef.current = {
              id: b.id,
              origX: lay.x,
              origY: lay.y,
              startClientX: e.clientX,
              startClientY: e.clientY,
              scale,
            }
            ;(e.currentTarget as Element).setPointerCapture(e.pointerId)
          }}
          onOutputPinPointerDown={(pin, e) => {
            e.stopPropagation()
            if (readOnly) return
            const lay = layout.blockById.get(b.id)!
            const outs = blockBoolOutputs(b)
            const pinIdx = outs.indexOf(pin)
            const total = Math.max(outs.length, 1)
            const yOffset =
              HEADER_H + ((pinIdx + 0.5) * (lay.h - HEADER_H)) / total
            // Source anchor for the preview line = the stub dot, not
            // the block edge. Matches the rendered output-pin
            // position so the rubber-band line attaches visually.
            onStartWireDrag({
              fromBlockId: b.id,
              fromPin: pin,
              fromX: lay.x + lay.w + STUB_LEN,
              fromY: lay.y + yOffset,
              currentX: lay.x + lay.w + STUB_LEN,
              currentY: lay.y + yOffset,
            })
          }}
        />
      ))}

      {/* Wire-drag preview line */}
      {wireDrag && (
        <g pointerEvents="none">
          <path
            d={(() => {
              const dx = Math.max(20, (wireDrag.currentX - wireDrag.fromX) / 2)
              return `M ${wireDrag.fromX} ${wireDrag.fromY} C ${wireDrag.fromX + dx} ${wireDrag.fromY}, ${wireDrag.currentX - dx} ${wireDrag.currentY}, ${wireDrag.currentX} ${wireDrag.currentY}`
            })()}
            fill="none"
            className="stroke-highlight"
            strokeWidth={1.5}
            strokeDasharray="4 3"
            vectorEffect="non-scaling-stroke"
          />
          <circle
            cx={wireDrag.currentX}
            cy={wireDrag.currentY}
            r={3}
            className="fill-highlight"
          />
        </g>
      )}

      {prog.outputs.map((o, i) => {
        const from = layout.blockById.get(o.from_block)
        if (!from) return null
        const srcPinIdx = approxOutputPinIndex(prog.blocks, o.from_block, o.from_pin)
        const x1 = from.x + from.w + STUB_LEN
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
  selected,
  readOnly,
  onPointerDown,
  onOutputPinPointerDown,
}: {
  block: FbdBlock
  layout: BlockLayout
  liveValues: { bools: Record<string, boolean> } | null
  hasError: boolean
  selected: boolean
  readOnly: boolean
  onPointerDown: (e: React.PointerEvent) => void
  onOutputPinPointerDown: (pin: string, e: React.PointerEvent) => void
}) {
  const boolOutputs = blockBoolOutputs(block)
  // Industrial-FBD convention (Codesys / TIA / Step7):
  //   - **Instance name** sits ABOVE the block (regular weight, small).
  //     It's the variable name; the rest of the program references it.
  //   - **FB type** is BOLD at the top inside the box. The type tells
  //     you what the block does, so it gets the most visual weight.
  //   - **Input pin names** left-aligned just inside the left edge.
  //   - **Output pin names** right-aligned just inside the right edge.
  //   - **Input values** (vars / literals / `inst.pin`) printed in a
  //     fainter colour after the pin name — present only when the
  //     pin isn't being wired by another block, since wired inputs
  //     are visually obvious from the connecting line.
  //   - The box is a plain rectangle, square corners. No header strip,
  //     no rounded corners, no "card shadow".
  return (
    <g>
      {/* Instance label above the box. Stays outside the click/drag
          target so a quick click doesn't accidentally start a drag. */}
      <text
        x={layout.x + layout.w / 2}
        y={layout.y - 6}
        textAnchor="middle"
        className="fill-foreground pointer-events-none"
        fontSize="10"
        fontFamily="ui-monospace, monospace"
      >
        {block.instance}
      </text>
      {/* The box. Square corners, single-weight border. */}
      <rect
        x={layout.x}
        y={layout.y}
        width={layout.w}
        height={layout.h}
        className={cn(
          "fill-card",
          hasError
            ? "stroke-destructive"
            : selected
              ? "stroke-highlight"
              : "stroke-foreground",
        )}
        strokeWidth={selected || hasError ? 2 : 1.25}
        vectorEffect="non-scaling-stroke"
        onPointerDown={onPointerDown}
        style={{ cursor: readOnly ? "default" : "move" }}
      />
      {/* FB type — bold, centred, top of the inner area. The hit-test
          rect above the body keeps drag working over this region. */}
      <rect
        x={layout.x}
        y={layout.y}
        width={layout.w}
        height={HEADER_H}
        fill="transparent"
        onPointerDown={onPointerDown}
        style={{ cursor: readOnly ? "default" : "move" }}
      />
      <text
        x={layout.x + layout.w / 2}
        y={layout.y + 17}
        textAnchor="middle"
        className="fill-foreground pointer-events-none"
        fontSize="13"
        fontFamily="ui-monospace, monospace"
        fontWeight={700}
      >
        {block.fb_type}
      </text>

      {/* Input pins (left side, inside the box) */}
      {(block.inputs.length > 0 ? block.inputs : [null]).map((input, i) => {
        const y = layout.y + HEADER_H + i * PIN_ROW_H + PIN_ROW_H / 2
        if (!input) {
          return (
            <text
              key="empty"
              x={layout.x + 6}
              y={y + 3}
              className="fill-muted-foreground pointer-events-none"
              fontSize="10"
              fontFamily="ui-monospace, monospace"
              fontStyle="italic"
            >
              no inputs
            </text>
          )
        }
        // Wired inputs show only the pin name — the value is visible
        // from the wire itself, no need to clutter the box with it.
        // For var/literal we print the operand in muted text after
        // the pin name. The narrow `if/else` (vs. ternary) keeps
        // TypeScript's discriminated-union narrowing happy.
        let valueText: string | null = null
        if (input.value.kind === "var") {
          valueText = input.value.name
        } else if (input.value.kind === "literal") {
          valueText = input.value.value
        }
        return (
          <g key={input.pin}>
            {/* Connector stub: short horizontal line poking out from
                the block edge with a dot at its far end (Codesys /
                TIA convention). Wires terminate at the dot, not at
                the block edge. */}
            <line
              x1={layout.x - STUB_LEN}
              y1={y}
              x2={layout.x}
              y2={y}
              className="stroke-foreground pointer-events-none"
              strokeWidth={1}
              vectorEffect="non-scaling-stroke"
            />
            <circle
              cx={layout.x - STUB_LEN}
              cy={y}
              r={2.5}
              className="fill-foreground pointer-events-none"
            />
            {/* Pin name, inside-left, bold. */}
            <text
              x={layout.x + 5}
              y={y + 3}
              className="fill-foreground pointer-events-none"
              fontSize="10"
              fontFamily="ui-monospace, monospace"
              fontWeight={600}
            >
              {input.pin}
            </text>
            {/* Value to the right of the pin name, smaller + muted. */}
            {valueText && (
              <text
                x={layout.x + 5 + input.pin.length * 6.5 + 6}
                y={y + 3}
                className="fill-muted-foreground pointer-events-none"
                fontSize="9"
                fontFamily="ui-monospace, monospace"
              >
                {valueText}
              </text>
            )}
          </g>
        )
      })}

      {/* Output pins on the right side. Same pattern as inputs:
          pin name INSIDE right-aligned, bold, with a connector stub
          (short line + dot) reaching outside the box. Each dot is a
          drag handle for the wire-creation gesture. Non-BOOL outputs
          (ET / CV) are also rendered as labels but without a drag
          stub — they can't feed a BOOL network so they don't get
          wires here. */}
      {(boolOutputs.length > 0 ? boolOutputs : ["Q"]).map((pin, i, arr) => {
        const cy = layout.y + HEADER_H + ((i + 0.5) * (layout.h - HEADER_H)) / arr.length
        const live = liveValues
          ? liveValues.bools[`${block.instance}.${pin}`] === true
          : null
        return (
          <g key={pin}>
            {/* Pin name, inside-right, bold — matches the input
                pattern visually. */}
            <text
              x={layout.x + layout.w - 5}
              y={cy + 3}
              textAnchor="end"
              className="fill-foreground pointer-events-none"
              fontSize="10"
              fontFamily="ui-monospace, monospace"
              fontWeight={600}
            >
              {pin}
            </text>
            {/* Connector stub outside the right edge. */}
            <line
              x1={layout.x + layout.w}
              y1={cy}
              x2={layout.x + layout.w + STUB_LEN}
              y2={cy}
              className="stroke-foreground pointer-events-none"
              strokeWidth={1}
              vectorEffect="non-scaling-stroke"
            />
            {/* Larger transparent hit area for the wire-drag gesture. */}
            <circle
              cx={layout.x + layout.w + STUB_LEN}
              cy={cy}
              r={9}
              fill="transparent"
              style={{ cursor: readOnly ? "default" : "crosshair" }}
              onPointerDown={(e) => onOutputPinPointerDown(pin, e)}
            />
            <circle
              cx={layout.x + layout.w + STUB_LEN}
              cy={cy}
              r={2.5}
              className={powerClass(live).replace("stroke-", "fill-")}
              pointerEvents="none"
            />
          </g>
        )
      })}
    </g>
  )
}

// =================================================================
//   Detail bar (selected block)
// =================================================================

function BlockDetail({
  prog,
  block,
  onCommit,
  onClose,
  onDelete,
}: {
  prog: FbdProgram
  block: FbdBlock
  onCommit: (next: FbdProgram) => void
  onClose: () => void
  onDelete: () => void
}) {
  const def = fbByType(block.fb_type)
  const inputDefs = fbInputs(block.fb_type)
  const outputs = blockBoolOutputs(block)
  const varsByType = (iecType: string) =>
    prog.variables.filter((v) => v.type === iecType).map((v) => v.name)

  return (
    <div className="flex flex-wrap items-center gap-2 border-t border-highlight/30 bg-highlight/5 px-3 py-1.5 text-xs">
      <span className="rounded bg-muted px-1.5 py-0.5 font-mono text-[10px] uppercase text-muted-foreground">
        block {block.id}
      </span>
      <Select
        value={block.fb_type}
        onValueChange={(v) => onCommit(setBlockFbType(prog, block.id, v))}
      >
        <SelectTrigger className="h-7 w-28 text-xs">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {groupedFbs().map((group) => (
            <SelectGroup key={group.label}>
              <SelectLabel>{group.label}</SelectLabel>
              {group.fbs.map((fb) => (
                <SelectItem key={fb.type} value={fb.type}>
                  {fb.type}
                </SelectItem>
              ))}
            </SelectGroup>
          ))}
        </SelectContent>
      </Select>
      <InstanceInput
        value={block.instance}
        onCommit={(v) => onCommit(setBlockInstance(prog, block.id, v))}
      />
      {outputs.length > 0 && (
        <span
          className="font-mono text-[10px] text-muted-foreground"
          title={def ? def.description : undefined}
        >
          out: {outputs.join(" / ")}
        </span>
      )}
      <Separator />
      {inputDefs.map((pin) => {
        const binding = block.inputs.find((i) => i.pin === pin.pin)
        return (
          <span key={pin.pin} className="inline-flex items-center gap-1">
            <span
              className="font-mono text-[10px] text-muted-foreground"
              title={`${pin.doc} (${pin.type})`}
            >
              {pin.pin}:
            </span>
            <PinOperandPicker
              value={
                binding?.value ?? {
                  kind: "literal",
                  value: pin.type === "TIME" ? "T#0ms" : "0",
                }
              }
              blockOptions={prog.blocks.filter((b) => b.id !== block.id)}
              variableOptions={varsByType(pin.type)}
              onChange={(v) =>
                onCommit(setBlockInput(prog, block.id, pin.pin, v))
              }
            />
          </span>
        )
      })}
      <span className="ml-auto inline-flex gap-1">
        <button
          type="button"
          onClick={onDelete}
          title="Delete this block"
          className="flex items-center gap-1 rounded border border-destructive/40 bg-destructive/5 px-1.5 py-1 text-[11px] text-destructive hover:bg-destructive/15"
        >
          <Trash2 className="size-3" />
          delete
        </button>
        <button
          type="button"
          onClick={onClose}
          title="Close"
          className="rounded p-0.5 text-muted-foreground hover:bg-accent/40 hover:text-foreground"
        >
          <X className="size-3" />
        </button>
      </span>
    </div>
  )
}

function Separator() {
  return <span className="mx-1 h-4 w-px bg-border" />
}

function InstanceInput({
  value,
  onCommit,
}: {
  value: string
  onCommit: (v: string) => void
}) {
  const [draft, setDraft] = useState(value)
  useEffect(() => setDraft(value), [value])
  const commit = (next: string) => {
    const t = next.trim()
    if (!t || t === value) {
      setDraft(value)
      return
    }
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(t)) {
      setDraft(value)
      return
    }
    onCommit(t)
  }
  return (
    <input
      type="text"
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={() => commit(draft)}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit(draft)
      }}
      className="h-7 w-24 rounded border border-input bg-transparent px-2 font-mono text-xs"
      title="FB instance name"
    />
  )
}

/** Operand picker for an input pin. Three modes:
 *   - var: pick a declared variable (autocomplete by IEC type)
 *   - literal: free-form ST literal
 *   - block: pick another block + one of its output pins (= a wire)
 */
function PinOperandPicker({
  value,
  blockOptions,
  variableOptions,
  onChange,
}: {
  value: FbdInputSource
  blockOptions: FbdBlock[]
  variableOptions: string[]
  onChange: (next: FbdInputSource) => void
}) {
  const [draft, setDraft] = useState(
    value.kind === "literal"
      ? value.value
      : value.kind === "var"
        ? value.name
        : "",
  )
  useEffect(() => {
    setDraft(
      value.kind === "literal"
        ? value.value
        : value.kind === "var"
          ? value.name
          : "",
    )
  }, [value])

  const commitVar = (next: string) => {
    if (next.trim()) onChange({ kind: "var", name: next.trim() })
  }
  const commitLiteral = (next: string) => {
    if (next.trim()) onChange({ kind: "literal", value: next.trim() })
  }

  return (
    <span className="inline-flex gap-1">
      <Select
        value={value.kind}
        onValueChange={(k) => {
          if (k === "var") {
            onChange({
              kind: "var",
              name: draft || variableOptions[0] || "x",
            })
          } else if (k === "literal") {
            onChange({ kind: "literal", value: draft || "0" })
          } else if (k === "block") {
            // Default to first available block's first output pin.
            const src = blockOptions[0]
            if (src) {
              onChange({
                kind: "block",
                block_id: src.id,
                pin: blockBoolOutputs(src)[0] ?? "Q",
              })
            }
          }
        }}
      >
        <SelectTrigger className="h-7 w-14 text-xs">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="var">var</SelectItem>
          <SelectItem value="literal">lit</SelectItem>
          <SelectItem value="block" disabled={blockOptions.length === 0}>
            wire
          </SelectItem>
        </SelectContent>
      </Select>
      {value.kind === "block" ? (
        <>
          <Select
            value={value.block_id}
            onValueChange={(id) =>
              onChange({
                kind: "block",
                block_id: id,
                pin:
                  blockBoolOutputs(blockOptions.find((b) => b.id === id)!)[0] ??
                  "Q",
              })
            }
          >
            <SelectTrigger className="h-7 w-24 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {blockOptions.map((b) => (
                <SelectItem key={b.id} value={b.id}>
                  {b.instance}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select
            value={value.pin}
            onValueChange={(p) =>
              onChange({
                kind: "block",
                block_id: value.block_id,
                pin: p,
              })
            }
          >
            <SelectTrigger className="h-7 w-14 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {blockBoolOutputs(
                blockOptions.find((b) => b.id === value.block_id) ??
                  blockOptions[0]!,
              ).map((p) => (
                <SelectItem key={p} value={p}>
                  .{p}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </>
      ) : (
        <Input
          type="text"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={() =>
            value.kind === "var" ? commitVar(draft) : commitLiteral(draft)
          }
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              ;(e.target as HTMLInputElement).blur()
            }
          }}
          className="h-7 w-24 font-mono text-xs"
          list={value.kind === "var" ? "fbd-var-options" : undefined}
        />
      )}
      {value.kind === "var" && (
        <datalist id="fbd-var-options">
          {variableOptions.map((o) => (
            <option key={o} value={o} />
          ))}
        </datalist>
      )}
    </span>
  )
}

// =================================================================
//   Outputs bar (when no block selected)
// =================================================================

function OutputsBar({
  prog,
  onCommit,
}: {
  prog: FbdProgram
  onCommit: (next: FbdProgram) => void
  onClose: () => void
}) {
  // Compact view: list each VAR_OUTPUT binding with a remove button,
  // and a picker to add a new binding. Live values intentionally
  // omitted — the canvas already shows them on the stub wires.
  const outputVars = prog.variables.filter((v) => v.section === "output")
  const unbound = outputVars.filter(
    (v) => !prog.outputs.some((o) => o.variable === v.name),
  )
  return (
    <div className="flex flex-wrap items-center gap-2 border-t border-border bg-muted/10 px-3 py-1.5 text-xs">
      <span className="font-mono text-[10px] uppercase text-muted-foreground">
        outputs
      </span>
      {prog.outputs.map((o) => {
        const srcBlock = prog.blocks.find((b) => b.id === o.from_block)
        return (
          <span
            key={o.variable}
            className="inline-flex items-center gap-1 rounded border border-border bg-card px-1.5 py-0.5 font-mono"
          >
            <span className="text-foreground">{o.variable}</span>
            <span className="text-muted-foreground">
              ← {srcBlock?.instance ?? o.from_block}.{o.from_pin}
            </span>
            <button
              type="button"
              onClick={() => onCommit(removeOutputBinding(prog, o.variable))}
              className="ml-0.5 rounded p-0.5 text-muted-foreground hover:bg-destructive/15 hover:text-destructive"
              title={`Unbind ${o.variable}`}
            >
              <X className="size-3" />
            </button>
          </span>
        )
      })}
      {unbound.length > 0 && prog.blocks.length > 0 && (
        <AddOutputBinding
          unbound={unbound.map((v) => v.name)}
          blocks={prog.blocks}
          onAdd={(variable, fromBlock, fromPin) =>
            onCommit(setOutputBinding(prog, variable, fromBlock, fromPin))
          }
        />
      )}
    </div>
  )
}

function AddOutputBinding({
  unbound,
  blocks,
  onAdd,
}: {
  unbound: string[]
  blocks: FbdBlock[]
  onAdd: (variable: string, fromBlock: string, fromPin: string) => void
}) {
  const [variable, setVariable] = useState(unbound[0])
  const [blockId, setBlockId] = useState(blocks[0]?.id ?? "")
  const block = blocks.find((b) => b.id === blockId) ?? blocks[0]
  const outputs = block ? blockBoolOutputs(block) : ["Q"]
  const [pin, setPin] = useState(outputs[0] ?? "Q")
  useEffect(() => {
    if (!unbound.includes(variable)) setVariable(unbound[0])
  }, [unbound, variable])
  if (!variable || !block) return null
  return (
    <span className="inline-flex items-center gap-1">
      <Select value={variable} onValueChange={setVariable}>
        <SelectTrigger className="h-7 w-24 text-xs">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {unbound.map((v) => (
            <SelectItem key={v} value={v}>
              {v}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <span className="text-muted-foreground">←</span>
      <Select value={blockId} onValueChange={setBlockId}>
        <SelectTrigger className="h-7 w-24 text-xs">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {blocks.map((b) => (
            <SelectItem key={b.id} value={b.id}>
              {b.instance}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <Select value={pin} onValueChange={setPin}>
        <SelectTrigger className="h-7 w-14 text-xs">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {outputs.map((p) => (
            <SelectItem key={p} value={p}>
              .{p}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <button
        type="button"
        onClick={() => onAdd(variable, blockId, pin)}
        className="flex items-center gap-1 rounded border border-highlight/40 bg-highlight/10 px-1.5 py-0.5 text-[11px] text-foreground hover:bg-highlight/20"
      >
        <Plus className="size-3" />
        add
      </button>
    </span>
  )
}

// =================================================================
//   Diagnostics index
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

// (DiagnosticsBanner moved to ./DiagnosticsBanner.tsx — shared with
//  LDEditor and SFCEditor. We keep `describeLocation` local because
//  the formatter is FBD-specific.)

// =================================================================
//   Helpers
// =================================================================

function approxOutputPinIndex(
  blocks: FbdBlock[],
  blockId: string,
  _pin: string,
): number {
  const b = blocks.find((bb) => bb.id === blockId)
  if (!b) return 0
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
    if (!obj || typeof obj !== "object" || !Array.isArray(obj.blocks)) {
      return { kind: "error", message: "missing `blocks` array" }
    }
    // Normalise optional-but-required-on-frontend fields so the rest
    // of this file can dereference `prog.outputs.length`, `block.inputs`,
    // etc. without `undefined.length` crashes. Older project files on
    // disk pre-date the "always serialize empty arrays" backend fix
    // and may omit these fields entirely.
    if (!Array.isArray(obj.outputs)) obj.outputs = []
    if (Array.isArray(obj.blocks)) {
      for (const b of obj.blocks) {
        if (b && typeof b === "object" && !Array.isArray(b.inputs)) {
          b.inputs = []
        }
      }
    }
    if (!Array.isArray(obj.variables)) obj.variables = []
    return { kind: "ok", program: parseProgram(JSON.stringify(obj)) }
  } catch (e) {
    return { kind: "error", message: String(e) }
  }
}
