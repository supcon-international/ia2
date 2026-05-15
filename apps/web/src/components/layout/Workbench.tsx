import { useEffect, useState } from "react"
import { Group, Panel, Separator, type Layout } from "react-resizable-panels"

import { ProjectEmptyState } from "@/components/dialogs/ProjectEmptyState"
import { RuntimeProvider, useRuntime } from "@/state/runtime"
import { DevicePane } from "./DevicePane"
import { EdgePane } from "./EdgePane"
import { IoMapPane } from "./IoMapPane"
import { MonitorPane } from "./MonitorPane"
import { ProgramPane } from "./ProgramPane"
import { ProjectPane } from "./ProjectPane"
import { TasksPane } from "./TasksPane"

// The .dark class is applied at module load by lib/dark-mode.ts (reads
// localStorage). Components that care about the current theme subscribe
// via useDarkMode(); the toggle lives in the header.

export function Workbench() {
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

/** useState-backed Layout that mirrors itself to localStorage. */
function usePersistedLayout(
  key: string,
  fallback: Layout,
): [Layout, (next: Layout) => void] {
  const [layout, setLayout] = useState<Layout>(
    () => loadLayout(key) ?? fallback,
  )
  useEffect(() => {
    saveLayout(key, layout)
  }, [key, layout])
  return [layout, setLayout]
}

// Stable IDs — used as React keys and as layout dict keys.
const PANEL_PROJECT = "project"
const PANEL_CENTER = "center"
const PANEL_EDITOR = "editor"
const PANEL_MONITOR = "monitor"

function Shell() {
  const { project, projectLoading, view } = useRuntime()

  // Storage keys bumped to v3 with the removal of the right-hand
  // Agents pane — without the bump, anyone with a saved v2 layout
  // would have a leftover `agents` slot in the dict that no panel
  // claims, leaving the center stuck at its old narrow width.
  const [hLayout, setHLayout] = usePersistedLayout("cs.shell.h.v3", {
    [PANEL_PROJECT]: 18,
    [PANEL_CENTER]: 82,
  })
  const [vLayout, setVLayout] = usePersistedLayout("cs.shell.v.v2", {
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

  // Note: Monitor used to auto-hide when no run had happened yet, but
  // adding/removing the Panel mid-session made the lib redistribute the
  // vertical space to an equal split instead of restoring defaultLayout.
  // It's also more honest to always show Monitor (with its "click Run
  // to start" placeholder) so the user knows where live data appears.
  // For users who want it gone, the bottom gutter drags down to
  // `collapsedSize` (3% — a tiny stub).

  return (
    <div className="h-screen w-screen overflow-hidden bg-background text-foreground">
      <Group
        orientation="horizontal"
        defaultLayout={hLayout}
        onLayoutChange={setHLayout}
        className="h-full w-full"
      >
        <Panel
          id={PANEL_PROJECT}
          minSize="10%"
          maxSize="40%"
          collapsible
          collapsedSize="0%"
        >
          <ProjectPane />
        </Panel>
        <Gutter orientation="vertical" />
        <Panel id={PANEL_CENTER} minSize="30%">
          <Group
            orientation="vertical"
            defaultLayout={vLayout}
            onLayoutChange={setVLayout}
            className="h-full w-full"
          >
            <Panel id={PANEL_EDITOR} minSize="20%">
              {center}
            </Panel>
            <Gutter orientation="horizontal" />
            <Panel
              id={PANEL_MONITOR}
              minSize="5%"
              collapsible
              collapsedSize="3%"
            >
              <MonitorPane />
            </Panel>
          </Group>
        </Panel>
      </Group>
    </div>
  )
}

/**
 * Drag handle between two panels.
 *
 * Visually: a 4-px-wide (or tall) hit-area centered on a 1-px border line.
 * The hit area is transparent at rest and turns into a 2-px accent strip
 * when hovered or being dragged. This gives a generous grab target without
 * a fat permanent gutter eating screen real estate.
 */
function Gutter({ orientation }: { orientation: "vertical" | "horizontal" }) {
  // `orientation === "vertical"` means the SEPARATOR LINE is vertical, i.e.
  // it sits between two horizontally-arranged panels (left/right).
  const vertical = orientation === "vertical"
  const classes = [
    "group relative shrink-0",
    vertical
      ? "w-1 cursor-col-resize"
      : "h-1 cursor-row-resize",
  ].join(" ")
  return (
    <Separator className={classes}>
      {/* Always-visible 1px hairline */}
      <span
        aria-hidden
        className={
          vertical
            ? "pointer-events-none absolute inset-y-0 left-1/2 w-px -translate-x-1/2 bg-border"
            : "pointer-events-none absolute inset-x-0 top-1/2 h-px -translate-y-1/2 bg-border"
        }
      />
      {/* Hover / drag highlight on top */}
      <span
        aria-hidden
        className={
          (vertical
            ? "pointer-events-none absolute inset-y-0 left-1/2 w-[2px] -translate-x-1/2 "
            : "pointer-events-none absolute inset-x-0 top-1/2 h-[2px] -translate-y-1/2 ") +
          "bg-accent opacity-0 transition-opacity group-hover:opacity-100 group-data-[separator-state=drag]:opacity-100"
        }
      />
    </Separator>
  )
}
