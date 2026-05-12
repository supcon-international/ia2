import { useEffect, useState } from "react"
import { Group, Panel, Separator, type Layout } from "react-resizable-panels"

import { ProjectEmptyState } from "@/components/dialogs/ProjectEmptyState"
import { useDarkMode } from "@/lib/dark-mode"
import { RuntimeProvider, useRuntime } from "@/state/runtime"
import { AgentsPane } from "./AgentsPane"
import { DevicePane } from "./DevicePane"
import { EdgePane } from "./EdgePane"
import { IoMapPane } from "./IoMapPane"
import { MonitorPane } from "./MonitorPane"
import { ProgramPane } from "./ProgramPane"
import { ProjectPane } from "./ProjectPane"
import { TasksPane } from "./TasksPane"

export function Workbench() {
  useDarkMode()
  return (
    <RuntimeProvider>
      <Shell />
    </RuntimeProvider>
  )
}

/** Read a saved {panelId: size} layout from localStorage; tolerant of
 * corrupted entries (returns `undefined` instead of throwing). */
function loadLayout(key: string): Layout | undefined {
  try {
    const raw = window.localStorage.getItem(key)
    if (!raw) return undefined
    const parsed = JSON.parse(raw)
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      return parsed as Layout
    }
  } catch {
    /* ignore */
  }
  return undefined
}

function saveLayout(key: string, layout: Layout) {
  try {
    window.localStorage.setItem(key, JSON.stringify(layout))
  } catch {
    /* localStorage may be unavailable (private mode, etc.) — fine */
  }
}

/** A useState-backed Layout that mirrors itself to localStorage on every
 * change. Initial value loads from storage; falls back to `fallback`. */
function usePersistedLayout(
  key: string,
  fallback: Layout,
): [Layout, (next: Layout) => void] {
  const [layout, setLayout] = useState<Layout>(
    () => loadLayout(key) ?? fallback,
  )
  // Persist on every change. The Group's onLayoutChange fires per-drag
  // (not on every animation frame), so this isn't write-heavy.
  useEffect(() => {
    saveLayout(key, layout)
  }, [key, layout])
  return [layout, setLayout]
}

// Stable IDs — these double as React keys and as layout dict keys.
const PANEL_PROJECT = "project"
const PANEL_CENTER = "center"
const PANEL_AGENTS = "agents"
const PANEL_EDITOR = "editor"
const PANEL_MONITOR = "monitor"

function Shell() {
  const { project, projectLoading, view } = useRuntime()

  // Default sizes are percentages of the parent group. Picked to roughly
  // match the previous fixed layout (260 / 1fr / 320 → ~18 / 60 / 22).
  const [hLayout, setHLayout] = usePersistedLayout("cs.shell.h", {
    [PANEL_PROJECT]: 18,
    [PANEL_CENTER]: 60,
    [PANEL_AGENTS]: 22,
  })
  const [vLayout, setVLayout] = usePersistedLayout("cs.shell.v", {
    [PANEL_EDITOR]: 68,
    [PANEL_MONITOR]: 32,
  })

  if (projectLoading) {
    return (
      <div className="grid h-screen place-items-center bg-background text-sm text-muted-foreground">
        Loading…
      </div>
    )
  }

  if (!project) {
    return <ProjectEmptyState />
  }

  const center =
    view === "device" ? (
      <DevicePane />
    ) : view === "edge" ? (
      <EdgePane />
    ) : view === "iomap" ? (
      <IoMapPane />
    ) : view === "tasks" ? (
      <TasksPane />
    ) : (
      <ProgramPane />
    )

  return (
    <div className="h-screen bg-background text-foreground">
      <Group
        orientation="horizontal"
        defaultLayout={hLayout}
        onLayoutChange={setHLayout}
        className="h-full w-full"
      >
        <Panel
          id={PANEL_PROJECT}
          minSize={10}
          maxSize={40}
          collapsible
          collapsedSize={0}
        >
          <ProjectPane />
        </Panel>
        <Separator className={separatorClass("vertical")} />
        <Panel id={PANEL_CENTER} minSize={30}>
          <Group
            orientation="vertical"
            defaultLayout={vLayout}
            onLayoutChange={setVLayout}
            className="h-full w-full border-x border-border"
          >
            <Panel id={PANEL_EDITOR} minSize={20}>
              {center}
            </Panel>
            <Separator className={separatorClass("horizontal")} />
            <Panel id={PANEL_MONITOR} minSize={5} collapsible collapsedSize={3}>
              <MonitorPane />
            </Panel>
          </Group>
        </Panel>
        <Separator className={separatorClass("vertical")} />
        <Panel
          id={PANEL_AGENTS}
          minSize={10}
          maxSize={45}
          collapsible
          collapsedSize={0}
        >
          <AgentsPane />
        </Panel>
      </Group>
    </div>
  )
}

/** Thin gutter line that thickens to 3px + highlights on hover/drag.
 * Vertical separators sit between horizontal panels (so they're
 * themselves vertical lines); naming matches the orientation of the
 * LINE, not the group. */
function separatorClass(orient: "vertical" | "horizontal"): string {
  return orient === "vertical"
    ? "w-px bg-border transition-colors hover:bg-accent data-[separator-state=drag]:bg-accent cursor-col-resize"
    : "h-px bg-border transition-colors hover:bg-accent data-[separator-state=drag]:bg-accent cursor-row-resize"
}
