import { ChevronDown, Plus } from "lucide-react"
import { useCallback, useEffect, useRef, useState } from "react"

import {
  currentProject,
  fetchOpenProjects,
  fetchProjects as fetchProjectListings,
  openProject as apiOpenProject,
} from "@/lib/api"
import { cn } from "@/lib/utils"
import { useRuntime } from "@/state/runtime"
import type { OpenProjectInfo } from "@/types/generated/OpenProjectInfo"
import type { ProjectListing } from "@/types/generated/ProjectListing"

/**
 * The thin strip at the top of the workbench — also doubles as the
 * macOS title-bar drag region. Hosts the project name + a small
 * picker for switching between open projects or spawning a new
 * window pinned to one.
 *
 * Multi-project model recap (server side): every project the server
 * has open lives in a single registry; each window of the IDE
 * identifies its current project via `?project=<name>` in the URL.
 * Requests carry that name in the `X-IA2-Project` header (set
 * automatically by `apiFetch`).
 *
 * Behaviours implemented here:
 *  - "Switch this window" picks a different project in the same
 *    window (history.pushState + reload runtime, no full navigation
 *    so the takeover overlay / SSE connection persist).
 *  - "Open in new window" calls `window.open(url)`. In the browser
 *    that's a new tab; in the Mac shell, the WebViewHost intercepts
 *    same-origin `window.open` and spawns a real new IA2 window.
 *  - "Open another project…" loads the disk-scanned project list and
 *    lets the user open one that isn't currently in the server's
 *    open set (adds it AND switches this window to it).
 */
export function WindowTitleBar() {
  const { project } = useRuntime()
  const projectName = project?.name ?? currentProject() ?? null

  return (
    <div className="flex h-7 shrink-0 items-center justify-center gap-2 px-2 text-xs">
      {/* Picker sits centred — the document-name-in-titlebar convention
       * (Finder, Mail, Linear), kept for the browser IDE. */}
      <ProjectPicker currentName={projectName} />
    </div>
  )
}

function ProjectPicker({ currentName }: { currentName: string | null }) {
  const [open, setOpen] = useState(false)
  const [openList, setOpenList] = useState<OpenProjectInfo[] | null>(null)
  const [diskList, setDiskList] = useState<ProjectListing[] | null>(null)
  const containerRef = useRef<HTMLDivElement | null>(null)

  // Refresh lists each time the dropdown opens so a project opened
  // in another window shows up here without a manual reload.
  useEffect(() => {
    if (!open) return
    void fetchOpenProjects().then((r) => setOpenList(r.projects))
    void fetchProjectListings().then(setDiskList)
  }, [open])

  // Click-outside to close. Pointerdown so the focused button's
  // own click event still fires before we hide.
  useEffect(() => {
    if (!open) return
    function onDoc(e: PointerEvent) {
      const root = containerRef.current
      if (!root) return
      if (!root.contains(e.target as Node)) setOpen(false)
    }
    document.addEventListener("pointerdown", onDoc)
    return () => document.removeEventListener("pointerdown", onDoc)
  }, [open])

  const switchTo = useCallback(async (name: string, path: string) => {
    // Same-window switch: update the URL search param then trigger
    // a hard reload so RuntimeProvider re-mounts with the new
    // project name. We don't soft-reload because half the state
    // (currentPou, editor source, attachment) is project-scoped and
    // teasing it apart cleanly is more code than it's worth — the
    // page is local and reloads in ~100 ms.
    const url = new URL(window.location.href)
    url.searchParams.set("project", name)
    // Ensure the server has this project open before we navigate —
    // the picker only lists open + disk-scanned projects, so the
    // explicit `openProject` round-trip costs at most one extra
    // request on first switch.
    try {
      await apiOpenProject(path)
    } catch {
      /* ignore — server may already have it open */
    }
    window.location.href = url.toString()
  }, [])

  const openInNewWindow = useCallback(async (name: string, path: string) => {
    try {
      await apiOpenProject(path)
    } catch {
      /* already open is fine */
    }
    const url = new URL(window.location.href)
    url.searchParams.set("project", name)
    // The Mac shell's WebViewHost intercepts same-origin
    // `window.open` and spawns a fresh IA2 NSWindow with the URL;
    // in a regular browser it's a new tab. Pass `noopener` so the
    // new window doesn't get a Window opener handle back to us —
    // each window is its own session.
    window.open(url.toString(), "_blank", "noopener")
  }, [])

  return (
    <div ref={containerRef} className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex items-center gap-1.5 rounded px-2 py-0.5 text-xs font-medium text-foreground/80 hover:bg-accent/40 hover:text-foreground"
        title="Switch project / open another window"
      >
        <span className="truncate">{currentName ?? "No project"}</span>
        <ChevronDown className="size-3 opacity-60" />
      </button>
      {open && (
        <div
          // Width caps so very long project names don't blow the
          // dropdown across the whole window; min-width keeps short
          // names visually consistent.
          className="absolute left-1/2 top-full z-50 mt-1 min-w-[18rem] max-w-[26rem] -translate-x-1/2 rounded-md border border-border bg-popover py-1 text-xs shadow-lg"
        >
          {/* Section header: open in this server right now */}
          <div className="px-3 pb-1 pt-1.5 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
            Open projects
          </div>
          {openList === null ? (
            <div className="px-3 py-1.5 text-muted-foreground">Loading…</div>
          ) : openList.length === 0 ? (
            <div className="px-3 py-1.5 text-muted-foreground">
              No projects open yet
            </div>
          ) : (
            <ul>
              {openList.map((p) => (
                <ProjectRow
                  key={p.name}
                  name={p.name}
                  path={p.path}
                  active={p.name === currentName}
                  onSwitch={() => {
                    setOpen(false)
                    void switchTo(p.name, p.path)
                  }}
                  onOpenInNewWindow={() => {
                    setOpen(false)
                    void openInNewWindow(p.name, p.path)
                  }}
                />
              ))}
            </ul>
          )}

          {/* Section: projects on disk not currently open. Letting
           * the user open one without bouncing through the modal
           * dialog keeps the multi-window flow fast. */}
          {diskList && diskList.length > 0 && (
            <>
              <div className="mx-2 my-1 border-t border-border" />
              <div className="px-3 pb-1 pt-1.5 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
                Other projects on disk
              </div>
              <ul>
                {diskList
                  .filter(
                    (p) =>
                      !(openList ?? []).some((open) => open.path === p.path),
                  )
                  .slice(0, 8)
                  .map((p) => (
                    <ProjectRow
                      key={p.path}
                      name={p.name}
                      path={p.path}
                      active={false}
                      onSwitch={() => {
                        setOpen(false)
                        void switchTo(p.name, p.path)
                      }}
                      onOpenInNewWindow={() => {
                        setOpen(false)
                        void openInNewWindow(p.name, p.path)
                      }}
                    />
                  ))}
              </ul>
            </>
          )}
        </div>
      )}
    </div>
  )
}

function ProjectRow(props: {
  name: string
  path: string
  active: boolean
  onSwitch: () => void
  onOpenInNewWindow: () => void
}) {
  return (
    <li className="group flex items-stretch">
      <button
        type="button"
        onClick={props.onSwitch}
        className={cn(
          "flex-1 truncate px-3 py-1.5 text-left transition-colors",
          props.active
            ? "font-medium text-highlight"
            : "text-foreground/90 hover:bg-accent/40",
        )}
        title={props.path}
      >
        {props.name}
        {props.active && (
          <span className="ml-2 text-[10px] uppercase tracking-wider text-muted-foreground">
            current
          </span>
        )}
      </button>
      <button
        type="button"
        onClick={props.onOpenInNewWindow}
        title="Open in a new window"
        className="flex shrink-0 items-center justify-center px-2 text-muted-foreground opacity-0 transition-opacity hover:text-foreground group-hover:opacity-100"
      >
        <Plus className="size-3.5" />
      </button>
    </li>
  )
}
