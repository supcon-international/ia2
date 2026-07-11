import { X } from "lucide-react"
import { useEffect, useState } from "react"
import { Group, Panel, Separator, type Layout } from "react-resizable-panels"

import { ProjectEmptyState } from "@/components/dialogs/ProjectEmptyState"
import { RuntimeProvider, useRuntime } from "@/state/runtime"
import { AgentStatusBar } from "./AgentStatusBar"
import { DevicePane } from "./DevicePane"
import { IconRail } from "./IconRail"
import { EdgePane } from "./EdgePane"
import { IoMapPane } from "./IoMapPane"
import { MonitorPane } from "./MonitorPane"
import { ProgramPane } from "./ProgramPane"
import { ProjectPane } from "./ProjectPane"
import { TasksPane } from "./TasksPane"
import { WindowTitleBar } from "./WindowTitleBar"

// The .dark class is applied at module load by lib/dark-mode.ts (reads
// localStorage). Components that care about the current theme subscribe
// via useDarkMode(); the toggle lives in the header.

export function Workbench() {
  return (
    <RuntimeProvider>
      <Shell />
      <GlobalErrorToast />
    </RuntimeProvider>
  )
}

/**
 * Single app-wide surface for failed actions. Every runtime action funnels
 * its failure into the context `error` field; without this, a failure in any
 * pane or dialog that doesn't render `error` itself (Device / IoMap / Tasks /
 * all create dialogs) would vanish silently. Lives inside RuntimeProvider so
 * it works in the loading and empty-project states too. Auto-dismisses, and
 * is manually dismissable.
 */
function GlobalErrorToast() {
  const { error, clearError } = useRuntime()
  useEffect(() => {
    if (!error) return
    const t = window.setTimeout(clearError, 8000)
    return () => window.clearTimeout(t)
  }, [error, clearError])
  if (!error) return null
  return (
    <div
      role="alert"
      // z-[100] keeps the toast above modal dialog overlays (Radix uses
      // z-50) — a create dialog stays open on failure, so its error must
      // float over the dimming overlay, not behind it.
      className="fixed bottom-4 right-4 z-[100] flex max-w-md items-start gap-2 rounded-md border border-red-300 bg-red-50 px-3 py-2 text-xs text-red-800 shadow-lg dark:border-red-900 dark:bg-red-950/90 dark:text-red-300"
    >
      <span className="min-w-0 flex-1 whitespace-pre-wrap break-words">
        {error}
      </span>
      <button
        type="button"
        onClick={clearError}
        aria-label="Dismiss error"
        className="-mt-0.5 -mr-1 shrink-0 rounded p-0.5 text-red-500 hover:bg-red-100 hover:text-red-700 dark:hover:bg-red-900/50"
      >
        <X className="size-3.5" />
      </button>
    </div>
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
      <div className="flex h-screen flex-col text-foreground">
        <div aria-hidden className="ia2-mac-drag-region h-7 shrink-0" />
        <div className="grid flex-1 place-items-center bg-background text-sm text-muted-foreground">
          Loading…
        </div>
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
    <div className="flex h-screen w-screen flex-col overflow-hidden text-foreground">
      {/* macOS titlebar safe area. The Swift shell uses
       *   fullSizeContentView + titlebarAppearsTransparent
       *   + drawsBackground: false on the WebView
       *   + NSVisualEffectView underlay (titlebar material)
       * so this 28px strip stays *transparent* and the OS-level
       * vibrancy blur shows through behind the traffic lights — the
       * same translucent chrome Safari / Linear / Figma get for free.
       * The strip is also a `-webkit-app-region: drag` zone so users
       * can grab it to move the window. On non-mac shells (Linux /
       * Windows / browser) the strip is just a thin transparent
       * gutter that costs ~28px of vertical space — no visual
       * conflict, no special-casing. */}
      <WindowTitleBar />
      {/* Rail + resizable panes share one horizontal row. The rail is a
       * fixed 52px outside the resizable Group so dragging the sidebar
       * gutter never touches it — it's chrome, not a pane. */}
      <div className="ia2-no-drag flex h-full min-h-0 w-full bg-background">
        <IconRail />
        <Group
          orientation="horizontal"
          defaultLayout={hLayout}
          onLayoutChange={setHLayout}
          className="h-full min-h-0 flex-1"
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
      {/* Acid-green agent bar. In normal flow (not an overlay) so the
       * workspace above shrinks by its 26px instead of being covered —
       * an agent editing the bottom line of a file must still be able
       * to see it. Renders nothing when no agent is active. */}
      <AgentStatusBar />
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
