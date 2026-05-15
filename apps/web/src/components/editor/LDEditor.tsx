import {
  ArrowDown,
  ArrowUp,
  Plus,
  RotateCw,
  Trash2,
  X,
} from "lucide-react"
import { useEffect, useMemo, useRef, useState } from "react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { cn } from "@/lib/utils"
import {
  addCoil,
  addInParallel,
  addInSeries,
  addRung,
  addVariable,
  deleteCoil,
  deleteNode,
  deleteRung,
  moveRung,
  newContact,
  parseProgram,
  removeVariable,
  serializeProgram,
  setCoilKind,
  setCoilVar,
  setContactVar,
  setRungLabel,
  toggleNegated,
  updateVariable,
  type NodePath,
} from "@/lib/ld-edit"
import type { LdCoilKind } from "@/types/generated/LdCoilKind"
import type { LdNode } from "@/types/generated/LdNode"
import type { LdProgram } from "@/types/generated/LdProgram"
import type { LdVarSection } from "@/types/generated/LdVarSection"

// =================================================================
//   Controlled LD editor.
//
//   Props:
//     value     — pretty-JSON source string (what's on disk).
//     onChange  — called whenever the user mutates the program; the
//                 string passed is the new pretty-JSON to save.
//
//   The editor parses `value` into an `LdProgram` on every render. The
//   JSON IS the source of truth — we never hold "the program" in
//   internal state separately, so external edits (revert, agent push,
//   git pull) round-trip without diverging.
//
//   Selection lives in local state (ephemeral; lost on POU switch /
//   page reload — fine, matches editor convention).
// =================================================================

type Selection =
  | { kind: "node"; rungIdx: number; path: NodePath }
  | { kind: "coil"; rungIdx: number; coilIdx: number }
  | { kind: "rung"; rungIdx: number }
  | { kind: "variable"; name: string }
  | null

export function LDEditor({
  value,
  onChange,
  readOnly = false,
  className,
}: {
  value: string
  onChange: (next: string) => void
  readOnly?: boolean
  className?: string
}) {
  const parsed = useMemo(() => safeParse(value), [value])
  const [sel, setSel] = useState<Selection>(null)

  // Drop selection when the source changes externally (revert, POU
  // switch). React's referential-equality check on `value` is what
  // saves us from infinite render loops on our own onChange.
  useEffect(() => {
    setSel(null)
  }, [value])

  if (parsed.kind === "error") {
    return (
      <div className={cn("flex h-full min-h-0 flex-col", className)}>
        <div className="border-b border-destructive/40 bg-destructive/5 px-3 py-2 text-xs text-destructive">
          LD JSON parse error: {parsed.message}
        </div>
        <pre className="flex-1 overflow-auto bg-muted/20 px-4 py-3 font-mono text-xs leading-relaxed text-foreground">
          {value}
        </pre>
      </div>
    )
  }
  const prog = parsed.program

  const commit = (next: LdProgram) => {
    if (readOnly) return
    onChange(serializeProgram(next))
  }

  return (
    <div className={cn("flex h-full min-h-0 flex-col", className)}>
      <Header prog={prog} />

      <div className="flex-1 overflow-auto bg-background">
        <VariablePanel
          prog={prog}
          selection={sel}
          readOnly={readOnly}
          onSelect={(name) =>
            setSel({ kind: "variable", name })
          }
          onAdd={(v) => commit(addVariable(prog, v))}
          onRemove={(name) => {
            commit(removeVariable(prog, name))
            if (sel?.kind === "variable" && sel.name === name) setSel(null)
          }}
          onUpdate={(name, patch) => commit(updateVariable(prog, name, patch))}
        />

        <div className="space-y-3 px-4 py-3">
          {prog.rungs.map((rung, i) => (
            <RungEditor
              key={rung.id}
              prog={prog}
              rung={rung}
              rungIdx={i}
              totalRungs={prog.rungs.length}
              selection={sel}
              readOnly={readOnly}
              onSelect={setSel}
              onCommit={commit}
            />
          ))}
          {!readOnly && (
            <div className="flex justify-center pt-2">
              <Button
                size="sm"
                variant="outline"
                onClick={() => commit(addRung(prog))}
              >
                <Plus className="mr-1 size-3" />
                Add rung
              </Button>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

// =================================================================
//   Top-of-pane summary
// =================================================================

function Header({ prog }: { prog: LdProgram }) {
  return (
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
  )
}

// =================================================================
//   Variable panel — three columns, inline-editable
// =================================================================

function VariablePanel({
  prog,
  selection,
  readOnly,
  onSelect,
  onAdd,
  onRemove,
  onUpdate,
}: {
  prog: LdProgram
  selection: Selection
  readOnly: boolean
  onSelect: (name: string) => void
  onAdd: (v: LdProgram["variables"][number]) => void
  onRemove: (name: string) => void
  onUpdate: (name: string, patch: Partial<LdProgram["variables"][number]>) => void
}) {
  const groups: Array<{ label: string; section: LdVarSection }> = [
    { label: "VAR_INPUT", section: "input" },
    { label: "VAR_OUTPUT", section: "output" },
    { label: "VAR", section: "internal" },
  ]
  const [drafting, setDrafting] = useState<{
    section: LdVarSection
    name: string
    type: string
  } | null>(null)

  return (
    <div className="grid grid-cols-3 gap-3 border-b border-border bg-muted/10 px-4 py-2 text-[11px]">
      {groups.map((g) => {
        const vs = prog.variables.filter((v) => v.section === g.section)
        return (
          <div key={g.section}>
            <div className="mb-1 flex items-center justify-between font-mono text-[9px] uppercase tracking-wider text-muted-foreground">
              <span>{g.label}</span>
              {!readOnly && (
                <button
                  type="button"
                  className="rounded p-0.5 hover:bg-accent/40 hover:text-foreground"
                  onClick={() =>
                    setDrafting({ section: g.section, name: "", type: "BOOL" })
                  }
                  title={`Add ${g.label}`}
                >
                  <Plus className="size-3" />
                </button>
              )}
            </div>
            <ul className="space-y-0.5">
              {vs.map((v) => (
                <li
                  key={v.name}
                  className={cn(
                    "group flex items-center gap-1 rounded px-1 font-mono cursor-pointer",
                    selection?.kind === "variable" && selection.name === v.name
                      ? "bg-highlight/15"
                      : "hover:bg-accent/30",
                  )}
                  onClick={() => onSelect(v.name)}
                >
                  <span className="text-foreground">{v.name}</span>
                  <span className="text-muted-foreground">{v.type}</span>
                  {v.init !== null && v.init !== undefined && (
                    <span className="text-muted-foreground">:= {v.init}</span>
                  )}
                  {!readOnly && (
                    <button
                      type="button"
                      className="ml-auto rounded p-0.5 opacity-0 transition-opacity hover:bg-destructive/15 hover:text-destructive group-hover:opacity-100"
                      onClick={(e) => {
                        e.stopPropagation()
                        onRemove(v.name)
                      }}
                      title={`Remove ${v.name}`}
                    >
                      <X className="size-3" />
                    </button>
                  )}
                </li>
              ))}
              {drafting && drafting.section === g.section && (
                <li className="flex gap-1 rounded bg-highlight/10 px-1 py-0.5">
                  <Input
                    autoFocus
                    placeholder="name"
                    value={drafting.name}
                    onChange={(e) =>
                      setDrafting({ ...drafting, name: e.target.value })
                    }
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && drafting.name.trim()) {
                        onAdd({
                          name: drafting.name.trim(),
                          type: drafting.type,
                          section: g.section,
                          init: null,
                        })
                        setDrafting(null)
                      } else if (e.key === "Escape") {
                        setDrafting(null)
                      }
                    }}
                    className="h-6 w-20 font-mono text-[11px]"
                  />
                  <Input
                    placeholder="type"
                    value={drafting.type}
                    onChange={(e) =>
                      setDrafting({ ...drafting, type: e.target.value })
                    }
                    className="h-6 w-16 font-mono text-[11px]"
                  />
                </li>
              )}
            </ul>
            {/* Editable details for the selected variable in this section. */}
            {selection?.kind === "variable" &&
              vs.some((v) => v.name === selection.name) && (
                <VariableDetail
                  prog={prog}
                  name={selection.name}
                  onUpdate={onUpdate}
                />
              )}
          </div>
        )
      })}
    </div>
  )
}

function VariableDetail({
  prog,
  name,
  onUpdate,
}: {
  prog: LdProgram
  name: string
  onUpdate: (name: string, patch: Partial<LdProgram["variables"][number]>) => void
}) {
  const v = prog.variables.find((x) => x.name === name)
  if (!v) return null
  return (
    <div className="mt-2 space-y-1 rounded border border-highlight/30 bg-highlight/5 p-1.5 text-[10px]">
      <Row label="type">
        <Input
          value={v.type}
          onChange={(e) => onUpdate(name, { type: e.target.value })}
          className="h-6 font-mono"
        />
      </Row>
      <Row label="init">
        <Input
          placeholder="(none)"
          value={v.init ?? ""}
          onChange={(e) =>
            onUpdate(name, {
              init: e.target.value.trim() ? e.target.value : null,
            })
          }
          className="h-6 font-mono"
        />
      </Row>
    </div>
  )
}

function Row({
  label,
  children,
}: {
  label: string
  children: React.ReactNode
}) {
  return (
    <div className="flex items-center gap-1.5">
      <span className="w-9 shrink-0 font-mono text-[9px] uppercase tracking-wider text-muted-foreground">
        {label}
      </span>
      <div className="flex-1">{children}</div>
    </div>
  )
}

// =================================================================
//   Rung — toolbar + SVG canvas + inline detail overlays
// =================================================================

const CELL_W = 90
const CELL_H = 44
const RAIL_PAD = 16
const COIL_W = 100
const TOP_PAD = 12

interface LayoutBox {
  cols: number
  rows: number
}

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

function RungEditor({
  prog,
  rung,
  rungIdx,
  totalRungs,
  selection,
  readOnly,
  onSelect,
  onCommit,
}: {
  prog: LdProgram
  rung: LdProgram["rungs"][number]
  rungIdx: number
  totalRungs: number
  selection: Selection
  readOnly: boolean
  onSelect: (s: Selection) => void
  onCommit: (next: LdProgram) => void
}) {
  const layoutBox = measure(rung.logic)
  const cols = layoutBox.cols
  const rows = layoutBox.rows
  const width =
    RAIL_PAD * 2 +
    cols * CELL_W +
    COIL_W * Math.max(rung.coils.length, 1) +
    24
  const height = Math.max(rows, 1) * CELL_H + TOP_PAD * 2

  const isSelected =
    selection?.kind === "rung" && selection.rungIdx === rungIdx

  return (
    <div
      className={cn(
        "rounded-md border border-border bg-card",
        isSelected && "ring-1 ring-highlight",
      )}
    >
      <RungToolbar
        rung={rung}
        rungIdx={rungIdx}
        totalRungs={totalRungs}
        readOnly={readOnly}
        onSelectRung={() => onSelect({ kind: "rung", rungIdx })}
        onLabelChange={(label) => onCommit(setRungLabel(prog, rungIdx, label))}
        onDelete={() => {
          onCommit(deleteRung(prog, rungIdx))
          onSelect(null)
        }}
        onMoveUp={() => onCommit(moveRung(prog, rungIdx, rungIdx - 1))}
        onMoveDown={() => onCommit(moveRung(prog, rungIdx, rungIdx + 1))}
        onAddCoil={() => {
          const coilVar =
            prog.variables.find((v) => v.section === "output")?.name ??
            prog.variables.find((v) => v.type === "BOOL")?.name ??
            "out"
          onCommit(addCoil(prog, rungIdx, coilVar))
        }}
      />
      <svg
        viewBox={`0 0 ${width} ${height}`}
        width="100%"
        className="block"
        style={{ height }}
      >
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

        <RenderNode
          node={rung.logic}
          path={[]}
          x={RAIL_PAD}
          y={TOP_PAD}
          cols={cols}
          rows={rows}
          rungIdx={rungIdx}
          selection={selection}
          readOnly={readOnly}
          onSelect={onSelect}
          onCommit={(transform) => onCommit(transform(prog))}
        />

        {rung.coils.map((coil, ci) => {
          const cx = RAIL_PAD + cols * CELL_W + ci * COIL_W
          const cy = TOP_PAD + ((rows - 1) * CELL_H) / 2 + CELL_H / 2
          const sel =
            selection?.kind === "coil" &&
            selection.rungIdx === rungIdx &&
            selection.coilIdx === ci
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
              <CoilGlyph
                coil={coil}
                x={cx + (COIL_W - 36) / 2}
                y={cy}
                selected={sel}
                onClick={() =>
                  onSelect(
                    sel ? null : { kind: "coil", rungIdx, coilIdx: ci },
                  )
                }
              />
            </g>
          )
        })}

        {/* Empty-rung hint when no coil declared. */}
        {rung.coils.length === 0 && (
          <text
            x={RAIL_PAD + cols * CELL_W + 12}
            y={TOP_PAD + ((rows - 1) * CELL_H) / 2 + CELL_H / 2 + 4}
            className="fill-muted-foreground"
            fontSize="10"
            fontFamily="ui-monospace, monospace"
          >
            (no coil — add one →)
          </text>
        )}
      </svg>

      {/* Inline selection editors live below the SVG so they get
          predictable layout / scrolling rather than fighting with SVG
          coordinates. Trade-off: a tiny vertical jump when you select
          versus a popup library. */}
      <SelectionDetail
        prog={prog}
        rungIdx={rungIdx}
        rung={rung}
        selection={selection}
        readOnly={readOnly}
        onClose={() => onSelect(null)}
        onCommit={onCommit}
      />
    </div>
  )
}

function RungToolbar({
  rung,
  rungIdx,
  totalRungs,
  readOnly,
  onSelectRung,
  onLabelChange,
  onDelete,
  onMoveUp,
  onMoveDown,
  onAddCoil,
}: {
  rung: LdProgram["rungs"][number]
  rungIdx: number
  totalRungs: number
  readOnly: boolean
  onSelectRung: () => void
  onLabelChange: (next: string | null) => void
  onDelete: () => void
  onMoveUp: () => void
  onMoveDown: () => void
  onAddCoil: () => void
}) {
  const [editingLabel, setEditingLabel] = useState(false)
  return (
    <div
      className="flex items-center gap-2 border-b border-border bg-muted/20 px-2 py-1 text-[10px] font-medium uppercase tracking-wider text-muted-foreground"
      onClick={onSelectRung}
    >
      <span className="shrink-0">
        rung {rungIdx} · <span className="font-mono normal-case">{rung.id}</span>
      </span>
      <span className="flex-1 normal-case tracking-normal text-foreground/80">
        {editingLabel && !readOnly ? (
          <Input
            autoFocus
            defaultValue={rung.label ?? ""}
            className="h-6 text-xs"
            onBlur={(e) => {
              onLabelChange(e.target.value.trim() || null)
              setEditingLabel(false)
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                onLabelChange((e.target as HTMLInputElement).value.trim() || null)
                setEditingLabel(false)
              } else if (e.key === "Escape") {
                setEditingLabel(false)
              }
            }}
          />
        ) : (
          <span
            className="cursor-text"
            onClick={(e) => {
              e.stopPropagation()
              if (!readOnly) setEditingLabel(true)
            }}
          >
            {rung.label || (
              <span className="text-muted-foreground/60">click to label…</span>
            )}
          </span>
        )}
      </span>
      {!readOnly && (
        <div
          className="flex items-center gap-0.5"
          onClick={(e) => e.stopPropagation()}
        >
          <IconBtn
            title="Move up"
            disabled={rungIdx === 0}
            onClick={onMoveUp}
          >
            <ArrowUp className="size-3" />
          </IconBtn>
          <IconBtn
            title="Move down"
            disabled={rungIdx === totalRungs - 1}
            onClick={onMoveDown}
          >
            <ArrowDown className="size-3" />
          </IconBtn>
          <IconBtn title="Add coil" onClick={onAddCoil}>
            <Plus className="size-3" />
          </IconBtn>
          <IconBtn
            title="Delete rung"
            onClick={onDelete}
            className="hover:text-destructive"
          >
            <Trash2 className="size-3" />
          </IconBtn>
        </div>
      )}
    </div>
  )
}

function IconBtn({
  children,
  onClick,
  disabled,
  title,
  className,
}: {
  children: React.ReactNode
  onClick?: () => void
  disabled?: boolean
  title?: string
  className?: string
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={title}
      className={cn(
        "rounded p-0.5 text-muted-foreground transition-colors hover:bg-accent/40 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-30",
        className,
      )}
    >
      {children}
    </button>
  )
}

// =================================================================
//   SVG node renderer (with click handlers)
// =================================================================

interface NodeRenderProps {
  node: LdNode
  path: NodePath
  x: number
  y: number
  cols: number
  rows: number
  rungIdx: number
  selection: Selection
  readOnly: boolean
  onSelect: (s: Selection) => void
  onCommit: (transform: (prog: LdProgram) => LdProgram) => void
}

function RenderNode(props: NodeRenderProps) {
  const { node, path, x, y, cols, rows, rungIdx, selection, onSelect } = props
  const midY = y + ((rows - 1) * CELL_H) / 2 + CELL_H / 2
  const selected =
    selection?.kind === "node" &&
    selection.rungIdx === rungIdx &&
    pathsEqual(selection.path, path)

  const click = () => onSelect(selected ? null : { kind: "node", rungIdx, path })

  switch (node.op) {
    case "contact":
      return (
        <ContactGlyph
          x={x}
          y={midY}
          width={CELL_W}
          name={node.var}
          negated={node.negated}
          selected={selected}
          onClick={click}
        />
      )
    case "const":
      return (
        <ConstGlyph
          x={x}
          y={midY}
          width={CELL_W}
          value={node.value}
          selected={selected}
          onClick={click}
        />
      )
    case "not": {
      if (node.arg.op === "contact") {
        return (
          <ContactGlyph
            x={x}
            y={midY}
            width={CELL_W}
            name={node.arg.var}
            negated={!node.arg.negated}
            selected={selected}
            onClick={click}
          />
        )
      }
      const m = measure(node.arg)
      return (
        <g>
          <RenderNode
            {...props}
            node={node.arg}
            path={[...path, 0]}
            cols={m.cols}
            rows={m.rows}
          />
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
      if (node.args.length === 0) {
        return (
          <ConstGlyph
            x={x}
            y={midY}
            width={CELL_W}
            value={true}
            selected={selected}
            onClick={click}
          />
        )
      }
      let cursor = x
      const elems: React.ReactNode[] = []
      node.args.forEach((child, i) => {
        const m = measure(child)
        elems.push(
          <RenderNode
            key={i}
            {...props}
            node={child}
            path={[...path, i]}
            x={cursor}
            cols={m.cols}
            rows={rows}
          />,
        )
        cursor += m.cols * CELL_W
      })
      return <g>{elems}</g>
    }
    case "or": {
      if (node.args.length === 0) {
        return (
          <ConstGlyph
            x={x}
            y={midY}
            width={CELL_W}
            value={false}
            selected={selected}
            onClick={click}
          />
        )
      }
      let rowCursor = 0
      const elems: React.ReactNode[] = []
      const inX = x
      const outX = x + cols * CELL_W
      node.args.forEach((child, i) => {
        const m = measure(child)
        const laneY = y + rowCursor * CELL_H
        const laneMidY = laneY + ((m.rows - 1) * CELL_H) / 2 + CELL_H / 2
        elems.push(
          <RenderNode
            key={i}
            {...props}
            node={child}
            path={[...path, i]}
            x={inX}
            y={laneY}
            cols={cols}
            rows={m.rows}
          />,
        )
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
      })
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

// =================================================================
//   Glyphs (with selection ring + click handlers)
// =================================================================

function ContactGlyph({
  x,
  y,
  width,
  name,
  negated,
  selected,
  onClick,
}: {
  x: number
  y: number
  width: number
  name: string
  negated: boolean
  selected: boolean
  onClick: () => void
}) {
  const cx = x + width / 2
  const half = 10
  return (
    <g onClick={onClick} className="cursor-pointer">
      {/* Wider invisible hit target so small contact glyphs are easy
          to click on a touchpad. */}
      <rect
        x={x + 4}
        y={y - 18}
        width={width - 8}
        height={36}
        fill="transparent"
      />
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
      {selected && (
        <rect
          x={cx - half - 3}
          y={y - 12}
          width={half * 2 + 6}
          height={24}
          fill="none"
          className="stroke-highlight"
          strokeWidth={1.5}
          vectorEffect="non-scaling-stroke"
          rx={2}
        />
      )}
    </g>
  )
}

function ConstGlyph({
  x,
  y,
  width,
  value,
  selected,
  onClick,
}: {
  x: number
  y: number
  width: number
  value: boolean
  selected: boolean
  onClick: () => void
}) {
  return (
    <g onClick={onClick} className="cursor-pointer">
      <rect
        x={x}
        y={y - 14}
        width={width}
        height={28}
        fill="transparent"
      />
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
      {selected && (
        <rect
          x={x + 4}
          y={y - 12}
          width={width - 8}
          height={24}
          fill="none"
          className="stroke-highlight"
          strokeWidth={1.5}
          vectorEffect="non-scaling-stroke"
          rx={2}
        />
      )}
    </g>
  )
}

function CoilGlyph({
  coil,
  x,
  y,
  selected,
  onClick,
}: {
  coil: LdProgram["rungs"][number]["coils"][number]
  x: number
  y: number
  selected: boolean
  onClick: () => void
}) {
  const w = 36
  const r = 10
  const cx = x + w / 2
  const inner =
    coil.kind === "set" ? "S" : coil.kind === "reset" ? "R" : null
  return (
    <g onClick={onClick} className="cursor-pointer">
      <rect
        x={x - 4}
        y={y - 18}
        width={w + 8}
        height={36}
        fill="transparent"
      />
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
      {selected && (
        <rect
          x={cx - r - 4}
          y={y - r - 4}
          width={(r + 4) * 2}
          height={(r + 4) * 2}
          fill="none"
          className="stroke-highlight"
          strokeWidth={1.5}
          vectorEffect="non-scaling-stroke"
          rx={2}
        />
      )}
    </g>
  )
}

// =================================================================
//   Selection detail panel (below the SVG)
//
//   When something is selected, an inline editor strip drops in
//   between the rung canvas and the next rung. Contains all the
//   actions you'd otherwise hide in popup menus / right-click.
// =================================================================

function SelectionDetail({
  prog,
  rungIdx,
  rung,
  selection,
  readOnly,
  onClose,
  onCommit,
}: {
  prog: LdProgram
  rungIdx: number
  rung: LdProgram["rungs"][number]
  selection: Selection
  readOnly: boolean
  onClose: () => void
  onCommit: (next: LdProgram) => void
}) {
  if (!selection || readOnly) return null
  if (selection.kind === "node" && selection.rungIdx === rungIdx) {
    return (
      <NodeDetail
        prog={prog}
        rungIdx={rungIdx}
        rung={rung}
        path={selection.path}
        onClose={onClose}
        onCommit={onCommit}
      />
    )
  }
  if (selection.kind === "coil" && selection.rungIdx === rungIdx) {
    return (
      <CoilDetail
        prog={prog}
        rungIdx={rungIdx}
        rung={rung}
        coilIdx={selection.coilIdx}
        onClose={onClose}
        onCommit={onCommit}
      />
    )
  }
  return null
}

function NodeDetail({
  prog,
  rungIdx,
  rung,
  path,
  onClose,
  onCommit,
}: {
  prog: LdProgram
  rungIdx: number
  rung: LdProgram["rungs"][number]
  path: NodePath
  onClose: () => void
  onCommit: (next: LdProgram) => void
}) {
  // Walk the path to find the selected node. Defensive against
  // stale selections that survive a structural edit by one frame.
  let node: LdNode | undefined = rung.logic
  try {
    for (const step of path) {
      if (!node) break
      if (node.op === "and" || node.op === "or") node = node.args[step]
      else if (node.op === "not") node = node.arg
      else node = undefined
    }
  } catch {
    node = undefined
  }
  if (!node) return null

  const boolVars = prog.variables
    .filter((v) => v.type === "BOOL" || v.type === "")
    .map((v) => v.name)

  return (
    <DetailBar onClose={onClose}>
      {node.op === "contact" ? (
        <>
          <DetailLabel>contact</DetailLabel>
          <VarPicker
            value={node.var}
            options={boolVars}
            onChange={(v) => onCommit(setContactVar(prog, rungIdx, path, v))}
          />
          <ToggleBtn
            active={node.negated}
            onClick={() => onCommit(toggleNegated(prog, rungIdx, path))}
            title="Toggle normally-closed"
          >
            ¬ negate
          </ToggleBtn>
        </>
      ) : node.op === "const" ? (
        <>
          <DetailLabel>const</DetailLabel>
          <span className="font-mono text-xs">
            {node.value ? "TRUE" : "FALSE"}
          </span>
        </>
      ) : (
        <>
          <DetailLabel>{node.op}</DetailLabel>
          <span className="text-xs text-muted-foreground">
            {"args" in node && Array.isArray(node.args)
              ? `${node.args.length} branches`
              : "1 child"}
          </span>
        </>
      )}

      <Separator />
      <ActionBtn
        onClick={() =>
          onCommit(
            addInSeries(prog, rungIdx, path, "after", newContact()),
          )
        }
        title="Insert a contact in series to the right"
      >
        <Plus className="size-3" />
        series
      </ActionBtn>
      <ActionBtn
        onClick={() =>
          onCommit(
            addInParallel(prog, rungIdx, path, "after", newContact()),
          )
        }
        title="Insert a contact in parallel below"
      >
        <Plus className="size-3" />
        parallel
      </ActionBtn>
      <Separator />
      <ActionBtn
        onClick={() => {
          onCommit(deleteNode(prog, rungIdx, path))
          onClose()
        }}
        title="Delete this element"
        destructive
      >
        <Trash2 className="size-3" />
        delete
      </ActionBtn>
    </DetailBar>
  )
}

function CoilDetail({
  prog,
  rungIdx,
  rung,
  coilIdx,
  onClose,
  onCommit,
}: {
  prog: LdProgram
  rungIdx: number
  rung: LdProgram["rungs"][number]
  coilIdx: number
  onClose: () => void
  onCommit: (next: LdProgram) => void
}) {
  const coil = rung.coils[coilIdx]
  if (!coil) return null
  const candidates = prog.variables
    .filter((v) => v.type === "BOOL")
    .map((v) => v.name)
  return (
    <DetailBar onClose={onClose}>
      <DetailLabel>coil</DetailLabel>
      <VarPicker
        value={coil.var}
        options={candidates}
        onChange={(v) => onCommit(setCoilVar(prog, rungIdx, coilIdx, v))}
      />
      <Select
        value={coil.kind}
        onValueChange={(v) =>
          onCommit(setCoilKind(prog, rungIdx, coilIdx, v as LdCoilKind))
        }
      >
        <SelectTrigger className="h-7 w-32 text-xs">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="standard">standard ( )</SelectItem>
          <SelectItem value="set">set (S)</SelectItem>
          <SelectItem value="reset">reset (R)</SelectItem>
        </SelectContent>
      </Select>
      <Separator />
      <ActionBtn
        onClick={() => {
          onCommit(deleteCoil(prog, rungIdx, coilIdx))
          onClose()
        }}
        title="Remove this coil"
        destructive
      >
        <Trash2 className="size-3" />
        delete
      </ActionBtn>
    </DetailBar>
  )
}

// =================================================================
//   Detail-bar primitives
// =================================================================

function DetailBar({
  onClose,
  children,
}: {
  onClose: () => void
  children: React.ReactNode
}) {
  return (
    <div className="flex items-center gap-2 border-t border-highlight/30 bg-highlight/5 px-3 py-1.5 text-xs">
      {children}
      <button
        type="button"
        onClick={onClose}
        className="ml-auto rounded p-0.5 text-muted-foreground hover:bg-accent/40 hover:text-foreground"
        title="Close"
      >
        <X className="size-3" />
      </button>
    </div>
  )
}

function DetailLabel({ children }: { children: React.ReactNode }) {
  return (
    <span className="font-mono text-[9px] uppercase tracking-wider text-muted-foreground">
      {children}
    </span>
  )
}

function Separator() {
  return <span className="text-muted-foreground/30">·</span>
}

function ActionBtn({
  onClick,
  title,
  destructive,
  children,
}: {
  onClick: () => void
  title?: string
  destructive?: boolean
  children: React.ReactNode
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={title}
      className={cn(
        "inline-flex items-center gap-1 rounded px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wider transition-colors",
        destructive
          ? "text-destructive hover:bg-destructive/10"
          : "text-foreground hover:bg-accent/40",
      )}
    >
      {children}
    </button>
  )
}

function ToggleBtn({
  active,
  onClick,
  title,
  children,
}: {
  active: boolean
  onClick: () => void
  title?: string
  children: React.ReactNode
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={title}
      className={cn(
        "inline-flex items-center gap-1 rounded px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wider transition-colors",
        active
          ? "bg-highlight/15 text-highlight"
          : "text-muted-foreground hover:bg-accent/40 hover:text-foreground",
      )}
    >
      {children}
    </button>
  )
}

/** Combo input + datalist. Avoids the overhead of a real Select
 *  for the common case ("type the variable name and pick a suggestion
 *  from declared variables").
 *
 *  Internal draft state so typing doesn't trigger a JSON serialise /
 *  parse round-trip on every keystroke. We commit on blur / Enter,
 *  and reset the draft when `value` changes externally (e.g. user
 *  selects a different element). */
function VarPicker({
  value,
  options,
  onChange,
}: {
  value: string
  options: string[]
  onChange: (v: string) => void
}) {
  const id = useRef(`varpicker-${Math.random().toString(36).slice(2, 8)}`)
  const [draft, setDraft] = useState(value)
  useEffect(() => {
    setDraft(value)
  }, [value])
  return (
    <>
      <input
        list={id.current}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => {
          const next = draft.trim()
          if (next && next !== value) onChange(next)
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            const next = draft.trim()
            if (next && next !== value) onChange(next)
          } else if (e.key === "Escape") {
            setDraft(value)
            ;(e.target as HTMLInputElement).blur()
          }
        }}
        className="h-7 w-32 rounded border border-input bg-transparent px-2 font-mono text-xs"
      />
      <datalist id={id.current}>
        {options.map((o) => (
          <option key={o} value={o} />
        ))}
      </datalist>
    </>
  )
}

// =================================================================
//   Helpers
// =================================================================

type Parsed =
  | { kind: "ok"; program: LdProgram }
  | { kind: "error"; message: string }

function safeParse(source: string): Parsed {
  try {
    const program = parseProgram(source)
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

function pathsEqual(a: NodePath, b: NodePath) {
  if (a.length !== b.length) return false
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false
  return true
}

/** Mark `RotateCw` as referenced so its tree-shake doesn't whine —
 *  we ship the icon for a future "rotate / swap" action. */
void RotateCw
