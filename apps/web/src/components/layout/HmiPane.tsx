/**
 * HMI view: toolbar (title, ISA level, operate/arrange switch, check) +
 * the live canvas + a slim inspector for the selected element. The pane is
 * intentionally thinner than the graphical-language editors — v1's canvas
 * is where agents assemble screens and humans nudge them; the full palette
 * editor is the next phase (docs/hmi-design.md).
 */

import { useCallback, useEffect, useState } from "react"
import { Hand, MousePointerClick, ShieldAlert } from "lucide-react"

import { HmiCanvas, type CanvasMode } from "@/components/hmi/HmiCanvas"
import { checkHmi } from "@/lib/api"
import { cn } from "@/lib/utils"
import { useHmiMutation } from "@/state/hmi-live"
import { useRuntime } from "@/state/runtime"
import type { HmiDoc } from "@/types/generated/HmiDoc"
import type { HmiIssue } from "@/types/generated/HmiIssue"
import type { HmiNode } from "@/types/generated/HmiNode"

export function HmiPane() {
  const { currentHmi } = useRuntime()
  const [mode, setMode] = useState<CanvasMode>("operate")
  const [selected, setSelected] = useState<string | null>(null)
  const [doc, setDoc] = useState<HmiDoc | null>(null)
  const [issues, setIssues] = useState<HmiIssue[]>([])
  const mutation = useHmiMutation()

  const refreshIssues = useCallback(async () => {
    if (!currentHmi) return
    try {
      setIssues(await checkHmi(currentHmi))
    } catch {
      /* screen may not exist yet */
    }
  }, [currentHmi])

  useEffect(() => {
    setSelected(null)
    void refreshIssues()
  }, [refreshIssues])

  useEffect(() => {
    if (mutation && mutation.path === currentHmi) void refreshIssues()
  }, [mutation, currentHmi, refreshIssues])

  if (!currentHmi) {
    return (
      <main className="grid h-full place-items-center text-sm text-muted-foreground">
        Select a screen from the HMI section of the project tree.
      </main>
    )
  }

  const errors = issues.filter((i) => i.severity === "error").length
  const warnings = issues.length - errors
  const selectedNode = doc && selected ? findNode(doc.root, selected) : null

  return (
    <main className="flex h-full min-h-0 min-w-0 flex-col">
      <div className="flex h-9 shrink-0 items-center justify-between border-b border-border pl-3 pr-2 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        <span className="flex min-w-0 items-center gap-2 normal-case tracking-normal">
          <span className="truncate font-mono text-foreground">
            {currentHmi}
          </span>
          {doc && (
            <>
              <span className="truncate text-muted-foreground">
                {doc.title}
              </span>
              <span className="rounded bg-muted/60 px-1.5 py-0.5 font-mono text-[9px] uppercase tracking-wider">
                L{doc.level}
              </span>
            </>
          )}
          {issues.length > 0 && (
            <span
              title={issues.map((i) => i.message).join("\n")}
              className={cn(
                "flex items-center gap-1 rounded px-1.5 py-0.5 font-mono text-[9px] uppercase tracking-wider",
                errors > 0
                  ? "bg-destructive/15 text-destructive"
                  : "bg-warn/15 text-warn",
              )}
            >
              <ShieldAlert className="size-3" />
              {errors > 0 ? `${errors} errors` : `${warnings} warnings`}
            </span>
          )}
        </span>
        <ModeSwitch mode={mode} onChange={setMode} />
      </div>

      <div className="flex min-h-0 flex-1">
        <div className="min-w-0 flex-1">
          <HmiCanvas
            path={currentHmi}
            mode={mode}
            selected={selected}
            onSelect={setSelected}
            onDocLoaded={setDoc}
          />
        </div>
        {selectedNode && (
          <Inspector node={selectedNode} onClose={() => setSelected(null)} />
        )}
      </div>
    </main>
  )
}

function ModeSwitch({
  mode,
  onChange,
}: {
  mode: CanvasMode
  onChange: (m: CanvasMode) => void
}) {
  const btn = (m: CanvasMode, icon: React.ReactNode, label: string) => (
    <button
      type="button"
      onClick={() => onChange(m)}
      title={
        m === "operate"
          ? "Operate: actions are live; layout is locked"
          : "Arrange: drag elements (snap to grid); actions are inert"
      }
      className={cn(
        "flex items-center gap-1 rounded-[4px] px-2 py-[3px] text-[11px] font-medium normal-case tracking-normal",
        mode === m
          ? "bg-card text-foreground shadow-sm"
          : "text-muted-foreground hover:text-foreground",
      )}
    >
      {icon}
      {label}
    </button>
  )
  return (
    <div className="flex items-center gap-0.5 rounded-md bg-muted/60 p-0.5">
      {btn("operate", <MousePointerClick className="size-3" />, "Operate")}
      {btn("arrange", <Hand className="size-3" />, "Arrange")}
    </div>
  )
}

/** Read-only element facts: type, geometry, bindings, actions. Editing
 *  happens through the CLI/agent in P0; the palette editor is P1. */
function Inspector({
  node,
  onClose,
}: {
  node: HmiNode
  onClose: () => void
}) {
  const binds = Object.entries(node.bind)
  const actions = Object.entries(node.action)
  return (
    <aside className="w-[220px] shrink-0 overflow-auto border-l border-border bg-secondary/40 p-3 text-[11px]">
      <div className="flex items-center justify-between">
        <span className="font-mono text-[12px] text-foreground">{node.id}</span>
        <button
          type="button"
          onClick={onClose}
          className="text-muted-foreground hover:text-foreground"
        >
          ×
        </button>
      </div>
      <div className="mt-1 font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
        {node.type}
        {node.type === "symbol" && ` · ${node.symbol}`}
      </div>
      <div className="mt-2 font-mono text-[10px] text-muted-foreground">
        x {node.x} · y {node.y}
        {node.w > 0 && ` · ${node.w}×${node.h}`}
      </div>
      {binds.length > 0 && (
        <>
          <div className="mt-3 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
            Bindings
          </div>
          {binds.map(([k, b]) => (
            <div key={k} className="mt-1 font-mono text-[10px]">
              <span className="text-muted-foreground">{k}</span>{" "}
              <span className="text-foreground">
                {typeof b === "string" ? b : b?.variable}
              </span>
              {typeof b === "object" && b?.expr && (
                <span className="text-muted-foreground"> · {b.expr}</span>
              )}
            </div>
          ))}
        </>
      )}
      {actions.length > 0 && (
        <>
          <div className="mt-3 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
            Actions
          </div>
          {actions.map(([k, a]) => (
            <div key={k} className="mt-1 font-mono text-[10px]">
              <span className="text-muted-foreground">{k}</span>{" "}
              <span className="text-foreground">
                {a?.kind}
                {a && "variable" in a ? ` ${a.variable}` : ""}
              </span>
            </div>
          ))}
        </>
      )}
      <div className="mt-4 border-t border-border pt-2 text-[10px] leading-relaxed text-muted-foreground">
        Edit via <span className="font-mono">cs hmi op</span> — changes render
        here live.
      </div>
    </aside>
  )
}

function findNode(root: HmiNode, id: string): HmiNode | null {
  if (root.id === id) return root
  if (root.type === "group") {
    for (const c of root.children) {
      const hit = findNode(c, id)
      if (hit) return hit
    }
  }
  return null
}
