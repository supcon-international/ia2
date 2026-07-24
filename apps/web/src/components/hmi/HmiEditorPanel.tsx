/**
 * The human editing surface for HMI screens — a palette to add elements
 * and an editable inspector for the selected one. Every edit goes through
 * the SAME `/ops` endpoint agents use, so human and agent authorship are
 * indistinguishable to the document, the SSE stream, and the canvas's
 * spawn animation. There is no separate "editor save" path to drift.
 */

import { useEffect, useMemo, useState } from "react"
import { Plus, Trash2 } from "lucide-react"

import { fetchHmiSymbols, hmiOps } from "@/lib/api"
import { canHostAction } from "@/lib/hmi-actions"
import type { HmiAction } from "@/types/generated/HmiAction"
import type { HmiBinding } from "@/types/generated/HmiBinding"
import type { HmiDoc } from "@/types/generated/HmiDoc"
import type { HmiNode } from "@/types/generated/HmiNode"
import type { HmiSymbolInfo } from "@/types/generated/HmiSymbolInfo"

// ---- palette -------------------------------------------------------

/** Non-symbol node kinds a human can place, with sensible defaults. */
const BASE_KINDS: {
  label: string
  make: () => Record<string, unknown>
  w: number
  h: number
}[] = [
  { label: "text", make: () => ({ type: "text", text: "Label", style: "section" }), w: 160, h: 20 },
  { label: "value", make: () => ({ type: "value", label: "value" }), w: 220, h: 24 },
  { label: "button", make: () => ({ type: "button", label: "Button" }), w: 120, h: 32 },
  { label: "input", make: () => ({ type: "input", label: "setpoint" }), w: 220, h: 28 },
  { label: "trend", make: () => ({ type: "trend", series: [], window_s: 300 }), w: 480, h: 180 },
  { label: "alarmbar", make: () => ({ type: "alarmbar" }), w: 600, h: 32 },
  { label: "nav", make: () => ({ type: "nav", label: "Detail", target: "" }), w: 120, h: 32 },
  { label: "rect", make: () => ({ type: "shape", shape: "rect", points: [] }), w: 160, h: 100 },
  { label: "ellipse", make: () => ({ type: "shape", shape: "ellipse", points: [] }), w: 100, h: 100 },
]

export function HmiPalette({
  path,
  doc,
}: {
  path: string
  doc: HmiDoc | null
}) {
  const [symbols, setSymbols] = useState<HmiSymbolInfo[]>([])
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    void fetchHmiSymbols().then(setSymbols).catch(() => {})
  }, [])

  const add = async (
    base: Record<string, unknown>,
    w: number,
    h: number,
    prefix: string,
  ) => {
    setError(null)
    const id = freshId(doc, prefix)
    // Stagger placements so consecutive adds don't stack invisibly.
    const n = countNodes(doc)
    const node = {
      id,
      x: 96 + (n % 6) * 24,
      y: 96 + (n % 6) * 24,
      w,
      h,
      ...base,
    }
    try {
      await hmiOps(path, [
        { op: "add_node", parent: null, node: node as unknown as HmiNode, index: null },
      ])
    } catch (e) {
      setError(String(e))
    }
  }

  return (
    <div className="flex flex-wrap items-center gap-1 border-b border-border bg-secondary/60 px-2 py-1.5">
      <span className="mr-1 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
        Add
      </span>
      {symbols.map((s) => (
        <PaletteChip
          key={s.name}
          label={s.name}
          title={s.description}
          onClick={() =>
            void add(
              { type: "symbol", symbol: s.name, props: {} },
              s.default_size[0],
              s.default_size[1],
              s.name,
            )
          }
        />
      ))}
      <span className="mx-1 h-4 w-px bg-border" />
      {BASE_KINDS.map((k) => (
        <PaletteChip
          key={k.label}
          label={k.label}
          onClick={() => void add(k.make(), k.w, k.h, k.label)}
        />
      ))}
      {error && (
        <span className="ml-2 truncate text-[10px] text-destructive">{error}</span>
      )}
    </div>
  )
}

function PaletteChip({
  label,
  title,
  onClick,
}: {
  label: string
  title?: string
  onClick: () => void
}) {
  return (
    <button
      type="button"
      title={title ?? `Add ${label}`}
      onClick={onClick}
      className="flex items-center gap-0.5 rounded border border-border bg-card px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground hover:bg-accent/50 hover:text-foreground"
    >
      <Plus className="size-2.5" />
      {label}
    </button>
  )
}

function freshId(doc: HmiDoc | null, prefix: string): string {
  const used = new Set<string>()
  const walk = (n: HmiNode) => {
    used.add(n.id)
    if (n.type === "group") n.children.forEach(walk)
  }
  if (doc) walk(doc.root)
  let i = 1
  while (used.has(`${prefix}_${i}`)) i++
  return `${prefix}_${i}`
}

function countNodes(doc: HmiDoc | null): number {
  let n = 0
  const walk = (node: HmiNode) => {
    n++
    if (node.type === "group") node.children.forEach(walk)
  }
  if (doc) walk(doc.root)
  return n
}

// ---- editable inspector -------------------------------------------

export function HmiInspector({
  path,
  node,
  variables,
  onClose,
}: {
  path: string
  node: HmiNode
  variables: string[]
  onClose: () => void
}) {
  const [error, setError] = useState<string | null>(null)

  const patch = async (p: Record<string, unknown>) => {
    setError(null)
    try {
      await hmiOps(path, [{ op: "update_node", id: node.id, patch: p }])
    } catch (e) {
      setError(String(e))
    }
  }
  const remove = async () => {
    if (!confirm(`Delete element "${node.id}"?`)) return
    try {
      await hmiOps(path, [{ op: "remove_node", id: node.id }])
      onClose()
    } catch (e) {
      setError(String(e))
    }
  }

  return (
    <aside className="flex w-[240px] shrink-0 flex-col gap-3 overflow-auto border-l border-border bg-secondary/40 p-3 text-[11px]">
      <div className="flex items-center justify-between">
        <span className="truncate font-mono text-[12px] text-foreground">
          {node.id}
        </span>
        <div className="flex items-center gap-1">
          <button
            type="button"
            title="Delete element"
            onClick={() => void remove()}
            className="rounded p-1 text-muted-foreground hover:text-destructive"
          >
            <Trash2 className="size-3.5" />
          </button>
          <button
            type="button"
            onClick={onClose}
            className="rounded px-1 text-muted-foreground hover:text-foreground"
          >
            ×
          </button>
        </div>
      </div>
      <div className="-mt-2 font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
        {node.type}
        {node.type === "symbol" && ` · ${node.symbol}`}
      </div>

      <Section label="Geometry">
        <div className="grid grid-cols-2 gap-1.5">
          {(["x", "y", "w", "h"] as const).map((k) => (
            <NumField
              key={k}
              label={k}
              value={node[k]}
              onCommit={(v) => void patch({ [k]: v })}
            />
          ))}
        </div>
      </Section>

      <TypeFields node={node} patch={patch} />

      <Section label="Bindings">
        <BindingsEditor node={node} variables={variables} patch={patch} />
      </Section>

      {/* validate_hmi rejects actions on non-control node types; keep the
          editor from authoring them. A legacy action still shows so it
          can be removed. */}
      {(canHostAction(node.type) || Object.keys(node.action).length > 0) && (
        <Section label="Actions">
          <ActionsEditor node={node} variables={variables} patch={patch} />
        </Section>
      )}

      {error && <div className="text-[10px] text-destructive">{error}</div>}

      {/* Shared datalist for every variable field in this panel. */}
      <datalist id="hmi-vars">
        {variables.map((v) => (
          <option key={v} value={v} />
        ))}
      </datalist>
    </aside>
  )
}

function Section({
  label,
  children,
}: {
  label: string
  children: React.ReactNode
}) {
  return (
    <div>
      <div className="mb-1 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
        {label}
      </div>
      {children}
    </div>
  )
}

function NumField({
  label,
  value,
  onCommit,
}: {
  label: string
  value: number
  onCommit: (v: number) => void
}) {
  const [text, setText] = useState(String(value))
  useEffect(() => setText(String(value)), [value])
  const commit = () => {
    const v = Number(text)
    if (!Number.isNaN(v) && v !== value) onCommit(Math.round(v))
  }
  return (
    <label className="flex items-center gap-1">
      <span className="w-3 font-mono text-[10px] text-muted-foreground">
        {label}
      </span>
      <input
        value={text}
        onChange={(e) => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => e.key === "Enter" && commit()}
        className="h-6 w-full min-w-0 rounded border border-input bg-background px-1 font-mono text-[11px] text-foreground outline-none focus:border-ring"
      />
    </label>
  )
}

function TextField({
  label,
  value,
  placeholder,
  list,
  onCommit,
}: {
  label: string
  value: string
  placeholder?: string
  list?: string
  onCommit: (v: string) => void
}) {
  const [text, setText] = useState(value)
  useEffect(() => setText(value), [value])
  const commit = () => {
    if (text !== value) onCommit(text)
  }
  return (
    <label className="flex items-center gap-1.5">
      <span className="w-10 shrink-0 font-mono text-[10px] text-muted-foreground">
        {label}
      </span>
      <input
        value={text}
        list={list}
        placeholder={placeholder}
        onChange={(e) => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => e.key === "Enter" && commit()}
        className="h-6 w-full min-w-0 rounded border border-input bg-background px-1 font-mono text-[11px] text-foreground outline-none focus:border-ring"
      />
    </label>
  )
}

/** Per-type editable fields (the common ones; exotic props stay CLI-side). */
function TypeFields({
  node,
  patch,
}: {
  node: HmiNode
  patch: (p: Record<string, unknown>) => Promise<void>
}) {
  switch (node.type) {
    case "text": {
      const color = typeof node.props["color"] === "string" ? (node.props["color"] as string) : ""
      const size = typeof node.props["size"] === "number" ? String(node.props["size"]) : ""
      return (
        <Section label="Text">
          <div className="space-y-1.5">
            <TextField label="text" value={node.text} onCommit={(v) => void patch({ text: v })} />
            <TextField
              label="style"
              value={node.style}
              placeholder="body | section | title | caption"
              onCommit={(v) => void patch({ style: v })}
            />
            <TextField
              label="color"
              value={color}
              placeholder="ok | warn | alarm | #hex"
              onCommit={(v) => void patch({ props: { color: v || null } })}
            />
            <TextField
              label="size"
              value={size}
              placeholder="px"
              onCommit={(v) => {
                const n = Number(v)
                void patch({ props: { size: v === "" || Number.isNaN(n) ? null : n } })
              }}
            />
          </div>
        </Section>
      )
    }
    case "shape": {
      const fill = typeof node.props["fill"] === "string" ? (node.props["fill"] as string) : ""
      const stroke = typeof node.props["stroke"] === "string" ? (node.props["stroke"] as string) : ""
      const sw =
        typeof node.props["stroke_width"] === "number" ? String(node.props["stroke_width"]) : ""
      return (
        <Section label="Shape">
          <div className="space-y-1.5">
            <TextField
              label="fill"
              value={fill}
              placeholder="ok | warn | #hex | empty"
              onCommit={(v) => void patch({ props: { fill: v || null } })}
            />
            <TextField
              label="stroke"
              value={stroke}
              placeholder="color/token"
              onCommit={(v) => void patch({ props: { stroke: v || null } })}
            />
            <TextField
              label="width"
              value={sw}
              placeholder="px"
              onCommit={(v) => {
                const n = Number(v)
                void patch({ props: { stroke_width: v === "" || Number.isNaN(n) ? null : n } })
              }}
            />
          </div>
        </Section>
      )
    }
    case "value":
    case "input":
      return (
        <Section label={node.type === "input" ? "Input" : "Value"}>
          <div className="space-y-1.5">
            <TextField
              label="label"
              value={node.label ?? ""}
              onCommit={(v) => void patch({ label: v || null })}
            />
            <TextField
              label="unit"
              value={node.unit ?? ""}
              onCommit={(v) => void patch({ unit: v || null })}
            />
          </div>
        </Section>
      )
    case "button":
      return (
        <Section label="Button">
          <TextField label="label" value={node.label} onCommit={(v) => void patch({ label: v })} />
        </Section>
      )
    case "nav":
      return (
        <Section label="Nav">
          <div className="space-y-1.5">
            <TextField label="label" value={node.label} onCommit={(v) => void patch({ label: v })} />
            <TextField
              label="target"
              value={node.target}
              placeholder="screen slug"
              onCommit={(v) => void patch({ target: v })}
            />
          </div>
        </Section>
      )
    case "symbol": {
      const label = typeof node.props["label"] === "string" ? (node.props["label"] as string) : ""
      const unit = typeof node.props["unit"] === "string" ? (node.props["unit"] as string) : ""
      return (
        <Section label="Symbol props">
          <div className="space-y-1.5">
            <TextField
              label="label"
              value={label}
              onCommit={(v) => void patch({ props: { label: v || null } })}
            />
            <TextField
              label="unit"
              value={unit}
              onCommit={(v) => void patch({ props: { unit: v || null } })}
            />
          </div>
        </Section>
      )
    }
    case "trend":
      return (
        <Section label="Trend series (comma-sep variables)">
          <TextField
            label="vars"
            value={node.series.map((s) => s.variable).join(", ")}
            list="hmi-vars"
            onCommit={(v) =>
              void patch({
                series: v
                  .split(",")
                  .map((s) => s.trim())
                  .filter(Boolean)
                  .map((variable) => ({ variable, label: null })),
              })
            }
          />
        </Section>
      )
    default:
      return null
  }
}

/** The bind map, editable per known key of the node type (plus the keys
 *  already present). Value = variable name with autocomplete; empty
 *  clears the binding. */
function BindingsEditor({
  node,
  variables: _variables,
  patch,
}: {
  node: HmiNode
  variables: string[]
  patch: (p: Record<string, unknown>) => Promise<void>
}) {
  const keys = useMemo(() => {
    const known: Record<string, string[]> = {
      value: ["value", "color"],
      input: ["value"],
      button: ["on"],
      symbol: symbolBindKeys(node),
      text: ["text", "color"],
      shape: ["fill", "stroke"],
      trend: [],
      alarmbar: [],
    }
    const base = known[node.type] ?? []
    const existing = Object.keys(node.bind)
    // Every element can be condition-shown; keep `visible` last so the
    // common keys stay on top.
    return Array.from(new Set([...base, ...existing, "visible"]))
  }, [node])

  if (keys.length === 0) {
    return (
      <div className="text-[10px] text-muted-foreground/70">
        This element has no bindable props.
      </div>
    )
  }
  return (
    <div className="space-y-1.5">
      {keys.map((k) => {
        const b = node.bind[k]
        const current = b == null ? "" : typeof b === "string" ? b : b.variable
        const isSpec = b != null && typeof b !== "string"
        return (
          <TextField
            key={k}
            label={isSpec ? `${k}*` : k}
            value={current}
            placeholder="variable"
            list="hmi-vars"
            onCommit={(v) => {
              const name = v.trim()
              // A spec binding (expr/format/map) keeps its transform when
              // only the variable is renamed here; clearing removes it all.
              const next =
                name === ""
                  ? null
                  : isSpec
                    ? { ...(b as object), variable: name }
                    : name
              void patch({ bind: { [k]: next } })
            }}
          />
        )
      })}
    </div>
  )
}

function symbolBindKeys(node: HmiNode): string[] {
  if (node.type !== "symbol") return []
  switch (node.symbol) {
    case "tank":
      return ["value", "alarm", "color"]
    case "valve":
      return ["open", "fault"]
    case "pump":
    case "motor":
      return ["running", "fault"]
    case "fan":
    case "conveyor":
      return ["running", "fault"]
    case "gauge":
    case "setpoint":
    case "sparkline":
      return ["value"]
    case "analog":
      return ["value", "sp"]
    case "bar":
    case "led":
      return ["value", "color"]
    case "pipe":
    case "pipe_h":
    case "pipe_v":
      return ["flow", "color"]
    case "indicator":
      return ["on", "alarm"]
    default:
      return []
  }
}

/** Simple action editor: one row per gesture the node type supports
 *  (`tap` for most, `commit` for inputs) — kind, variable, confirm. */
function ActionsEditor({
  node,
  variables: _variables,
  patch,
}: {
  node: HmiNode
  variables: string[]
  patch: (p: Record<string, unknown>) => Promise<void>
}) {
  const gesture = node.type === "input" ? "commit" : "tap"
  const a = node.action[gesture]
  const kinds =
    node.type === "input"
      ? ["set_value"]
      : ["toggle", "write", "pulse", "nav"]

  const setKind = (kind: string) => {
    if (kind === "") {
      void patch({ action: { [gesture]: null } })
      return
    }
    const next: Record<string, unknown> =
      kind === "nav"
        ? { kind, target: "" }
        : kind === "write"
          ? { kind, variable: "", value: 1, confirm: true }
          : kind === "pulse"
            ? { kind, variable: "", ms: 500, confirm: true }
            : kind === "set_value"
              ? { kind, variable: "", min: null, max: null, confirm: true }
              : { kind, variable: "", confirm: true }
    void patch({ action: { [gesture]: next } })
  }

  return (
    <div className="space-y-1.5">
      <label className="flex items-center gap-1.5">
        <span className="w-10 shrink-0 font-mono text-[10px] text-muted-foreground">
          {gesture}
        </span>
        <select
          value={a?.kind ?? ""}
          onChange={(e) => setKind(e.target.value)}
          className="h-6 w-full rounded border border-input bg-background px-1 font-mono text-[11px] text-foreground outline-none"
        >
          <option value="">none</option>
          {kinds.map((k) => (
            <option key={k} value={k}>
              {k}
            </option>
          ))}
        </select>
      </label>
      {a && a.kind !== "nav" && (
        <TextField
          label="var"
          value={"variable" in a ? a.variable : ""}
          list="hmi-vars"
          onCommit={(v) =>
            void patch({ action: { [gesture]: { ...actionAsJson(a), variable: v } } })
          }
        />
      )}
      {a && a.kind === "nav" && (
        <TextField
          label="target"
          value={a.target}
          placeholder="screen slug"
          onCommit={(v) =>
            void patch({ action: { [gesture]: { kind: "nav", target: v } } })
          }
        />
      )}
      {a && a.kind !== "nav" && (
        <label className="flex items-center gap-1.5 text-[10px] text-muted-foreground">
          <input
            type="checkbox"
            checked={"confirm" in a ? a.confirm : true}
            onChange={(e) =>
              void patch({
                action: { [gesture]: { ...actionAsJson(a), confirm: e.target.checked } },
              })
            }
          />
          require confirmation
        </label>
      )}
    </div>
  )
}

function actionAsJson(a: HmiAction): Record<string, unknown> {
  return JSON.parse(JSON.stringify(a)) as Record<string, unknown>
}

export type { HmiBinding }
