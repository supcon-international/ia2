/**
 * Standalone operator panel — the hmi.html entry's whole UI.
 *
 * Served by `ia2-runtime --static-dir` on the edge box: `/hmi` lists the
 * screens deployed with the project, `/hmi/<screen>` renders one live.
 * Everything talks to the runtime's own surface on this same origin:
 * `/api/hmi` (read-only documents), `/events` (bare VarSnapshot SSE),
 * `/write` (confirmed actions), `/status` (fault strip + project name).
 *
 * The canvas itself is the exact component the IDE uses — only the
 * HmiHost implementation differs, so operate-mode behaviour (confirm
 * flows, bindings, trends) can't drift between the two surfaces.
 */

import { useCallback, useEffect, useMemo, useState } from "react"
import { Moon, Sun } from "lucide-react"

import { HmiCanvas } from "@/components/hmi/HmiCanvas"
import { HmiHostProvider, type HmiHost } from "@/components/hmi/host"
import { useThemeToggle } from "@/lib/dark-mode"
import { encodeForWrite } from "@/lib/write-encoding"
import { liveFeedStore, useConnected } from "@/state/live-feed"
import type { HmiListEntry } from "@/types/generated/HmiListEntry"

/** The slice of the runtime's /status the panel reads. The full shape is
 *  documented in docs/api.md (edge runtime table). */
type EdgeStatus = {
  project?: string | null
  fault?: string | null
}

async function jget<T>(url: string): Promise<T> {
  const res = await fetch(url)
  if (!res.ok) {
    throw new Error(`${res.status}: ${(await res.text()) || url}`)
  }
  return (await res.json()) as T
}

/** `/hmi/<screen>` when served by the runtime; `hmi.html?screen=<slug>`
 *  when opened straight off a vite dev server. */
function slugFromLocation(): string | null {
  const m = window.location.pathname.match(/^\/hmi\/(.+)$/)
  if (m) return decodeURIComponent(m[1])
  return new URLSearchParams(window.location.search).get("screen")
}

function urlFor(slug: string | null): string {
  if (window.location.pathname.startsWith("/hmi")) {
    return slug ? `/hmi/${encodeURIComponent(slug)}` : "/hmi"
  }
  const q = slug ? `?screen=${encodeURIComponent(slug)}` : ""
  return `${window.location.pathname}${q}`
}

export function HmiStandalone() {
  const [slug, setSlug] = useState<string | null>(slugFromLocation)
  const [screens, setScreens] = useState<HmiListEntry[] | null>(null)
  const [listError, setListError] = useState<string | null>(null)
  const [project, setProject] = useState("")
  const connected = useConnected()
  const { theme, toggle } = useThemeToggle()

  const navigate = useCallback((target: string | null) => {
    window.history.pushState(null, "", urlFor(target))
    setSlug(target)
  }, [])

  useEffect(() => {
    const onPop = () => setSlug(slugFromLocation())
    window.addEventListener("popstate", onPop)
    return () => window.removeEventListener("popstate", onPop)
  }, [])

  useEffect(() => {
    void jget<HmiListEntry[]>("/api/hmi")
      .then((rows) => {
        setScreens(rows)
        setListError(null)
      })
      .catch((e) => setListError(String(e)))
    void jget<EdgeStatus>("/status")
      .then((s) => setProject(s.project ?? ""))
      .catch(() => {})
  }, [])

  // The runtime's /events is a bare VarSnapshot stream (no AppEvent
  // wrapper) — feed it straight into the same live store the canvas
  // already reads. EventSource reconnects on its own.
  useEffect(() => {
    const es = new EventSource("/events")
    es.onopen = () => liveFeedStore.setConnected(true)
    es.onerror = () => liveFeedStore.setConnected(false)
    es.onmessage = (e) => {
      try {
        liveFeedStore.setSnapshot(JSON.parse(e.data))
      } catch {
        /* malformed tick — skip */
      }
    }
    return () => {
      es.close()
      liveFeedStore.setConnected(false)
    }
  }, [])

  const host = useMemo<HmiHost>(
    () => ({
      fetchDoc: (p) => jget(`/api/hmi/${encodeURIComponent(p)}`),
      // no saveDoc: screens are edited in the IDE and arrive via deploy
      write: async (name, value, typeName) => {
        const res = await fetch("/write", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ name, value: encodeForWrite(value, typeName) }),
        })
        if (!res.ok) {
          throw new Error(`${res.status}: ${await res.text()}`)
        }
      },
      nav: (target) => navigate(target),
      runtimeState: async () => {
        const s = await jget<EdgeStatus>("/status")
        return { running: s.fault == null, alarm: s.fault ?? null }
      },
    }),
    [navigate],
  )

  const title =
    (slug && screens?.find((s) => s.path === slug)?.title) || slug || ""

  return (
    <div className="flex h-dvh flex-col bg-background text-foreground">
      <header className="flex h-10 shrink-0 items-center justify-between border-b border-border pl-3 pr-2">
        <div className="flex min-w-0 items-baseline gap-2">
          {slug ? (
            <button
              type="button"
              onClick={() => navigate(null)}
              className="shrink-0 font-mono text-[11px] text-muted-foreground hover:text-foreground"
            >
              ← Screens
            </button>
          ) : (
            <span className="font-mono text-[11px] uppercase tracking-wider text-muted-foreground">
              HMI
            </span>
          )}
          <span className="truncate text-[13px] font-medium">
            {slug ? title : project}
          </span>
          {slug && project && (
            <span className="truncate font-mono text-[11px] text-muted-foreground">
              {project}
            </span>
          )}
        </div>
        <div className="flex items-center gap-2">
          <span
            title={connected ? "Live values connected" : "No live connection"}
            className={
              "size-2 rounded-full " +
              (connected ? "bg-highlight" : "bg-muted-foreground/40")
            }
          />
          <button
            type="button"
            onClick={toggle}
            title="Toggle theme"
            className="grid size-7 place-items-center rounded text-muted-foreground hover:bg-accent/50 hover:text-foreground"
          >
            {theme === "dark" ? <Sun className="size-3.5" /> : <Moon className="size-3.5" />}
          </button>
        </div>
      </header>

      {slug ? (
        <div className="min-h-0 flex-1">
          <HmiHostProvider value={host}>
            <HmiCanvas
              path={slug}
              mode="operate"
              selected={null}
              onSelect={() => {}}
            />
          </HmiHostProvider>
        </div>
      ) : (
        <ScreenIndex screens={screens} error={listError} onOpen={navigate} />
      )}
    </div>
  )
}

function ScreenIndex({
  screens,
  error,
  onOpen,
}: {
  screens: HmiListEntry[] | null
  error: string | null
  onOpen: (slug: string) => void
}) {
  if (error) {
    return (
      <div className="grid flex-1 place-items-center p-6 text-center text-sm text-muted-foreground">
        Could not load screens: {error}
      </div>
    )
  }
  if (!screens) {
    return (
      <div className="grid flex-1 place-items-center text-sm text-muted-foreground">
        Loading…
      </div>
    )
  }
  if (screens.length === 0) {
    return (
      <div className="grid flex-1 place-items-center p-6 text-center text-sm text-muted-foreground">
        The deployed project has no HMI screens yet.
      </div>
    )
  }
  return (
    <div className="flex-1 overflow-auto p-6">
      <div className="mx-auto grid max-w-3xl gap-3 sm:grid-cols-2">
        {screens.map((s) => (
          <button
            key={s.path}
            type="button"
            onClick={() => onOpen(s.path)}
            className="rounded-lg border border-border bg-card p-4 text-left hover:border-ring"
          >
            <div className="flex items-center justify-between">
              <span className="font-mono text-[12px] text-muted-foreground">
                {s.path}
              </span>
              <span className="rounded bg-muted/60 px-1.5 py-0.5 font-mono text-[9px] uppercase tracking-wider text-muted-foreground">
                L{s.level}
              </span>
            </div>
            <div className="mt-1 text-[14px] font-medium text-foreground">
              {s.title}
            </div>
          </button>
        ))}
      </div>
    </div>
  )
}
