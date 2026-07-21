/**
 * HMI view: toolbar (title, ISA level, operate/arrange switch, check) +
 * the live canvas, plus the human editing surface — Arrange mode shows the
 * palette strip and an editable inspector (geometry, props, bindings,
 * actions), Operate mode a read-only one. Both agents (via `cs hmi op`)
 * and humans edit through the same /ops endpoint, so either side's
 * changes land live on the other's canvas.
 */

import { useCallback, useEffect, useMemo, useState } from "react"
import { Hand, MousePointerClick, ShieldAlert } from "lucide-react"

import { findNode, HmiCanvas, type CanvasMode } from "@/components/hmi/HmiCanvas"
import { HmiInspector, HmiPalette } from "@/components/hmi/HmiEditorPanel"
import { HmiHostProvider, type HmiHost } from "@/components/hmi/host"
import {
  checkHmi,
  fetchHmi,
  fetchProjectVariables,
  fetchRuntimeStatus,
  saveHmi,
  writeVariable,
} from "@/lib/api"
import { cn } from "@/lib/utils"
import { useHmiMutation } from "@/state/hmi-live"
import { useRuntime } from "@/state/runtime"
import type { HmiDoc } from "@/types/generated/HmiDoc"
import type { HmiIssue } from "@/types/generated/HmiIssue"

export function HmiPane() {
  const { currentHmi, selectHmi } = useRuntime()
  const [mode, setMode] = useState<CanvasMode>("operate")
  const [selected, setSelected] = useState<string | null>(null)
  const [doc, setDoc] = useState<HmiDoc | null>(null)
  const [issues, setIssues] = useState<HmiIssue[]>([])
  const [variables, setVariables] = useState<string[]>([])
  const mutation = useHmiMutation()

  // The IDE-side host: documents and writes go through the project
  // server; nav switches the workbench's active screen. The standalone
  // edge panel provides its own implementation of this seam.
  const host = useMemo<HmiHost>(
    () => ({
      fetchDoc: fetchHmi,
      saveDoc: saveHmi,
      write: writeVariable,
      nav: (target) => void selectHmi(target),
      runtimeState: async () => {
        const s = await fetchRuntimeStatus()
        // mode rides along so a paused scan loop doesn't show as a
        // green "Running" in the canvas alarmbar.
        return {
          running: s.running,
          alarm: s.last_error ?? null,
          mode: s.mode?.kind,
        }
      },
    }),
    [selectHmi],
  )

  useEffect(() => {
    void fetchProjectVariables()
      .then((r) => setVariables([...new Set(r.variables.map((v) => v.name))]))
      .catch(() => {})
  }, [currentHmi])

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
        <ModeSwitch
          mode={mode}
          onChange={(m) => {
            setMode(m)
            // Selection is Arrange-only; entering Operate drops it so
            // no inspector lingers over the operator surface.
            if (m === "operate") setSelected(null)
          }}
        />
      </div>

      {mode === "arrange" && <HmiPalette path={currentHmi} doc={doc} />}
      <div className="flex min-h-0 flex-1">
        <div className="min-w-0 flex-1">
          <HmiHostProvider value={host}>
            <HmiCanvas
              path={currentHmi}
              mode={mode}
              selected={selected}
              onSelect={setSelected}
              onDocLoaded={setDoc}
            />
          </HmiHostProvider>
        </div>
        {selectedNode && mode === "arrange" && (
          <HmiInspector
            path={currentHmi}
            node={selectedNode}
            variables={variables}
            onClose={() => setSelected(null)}
          />
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

