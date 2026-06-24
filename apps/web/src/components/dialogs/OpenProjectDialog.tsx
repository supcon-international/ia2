import { ArrowUp, Folder, FolderCheck, Loader2 } from "lucide-react"
import { useEffect, useState } from "react"

import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { browseFs } from "@/lib/api"
import { useRuntime } from "@/state/runtime"
import type { FsListing } from "@/types/generated/FsListing"

type Props = {
  trigger: React.ReactNode
}

/**
 * Folder picker for opening a project. A browser can't surface a native OS
 * path dialog, so we navigate the filesystem through the local server's
 * `/api/fs/browse` endpoint: list sub-folders, descend into them, go up to
 * the parent, and open any folder that is an IA2 project (contains
 * `project.toml`). A manual path field remains as a fallback for typing /
 * pasting an absolute path.
 */
export function OpenProjectDialog({ trigger }: Props) {
  const { openProject } = useRuntime()
  const [open, setOpen] = useState(false)
  const [listing, setListing] = useState<FsListing | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [manual, setManual] = useState("")
  const [submitting, setSubmitting] = useState(false)

  // Load the default projects dir when the dialog opens; reset on close.
  useEffect(() => {
    if (!open) {
      setListing(null)
      setManual("")
      setError(null)
      return
    }
    void navigate(undefined)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open])

  const navigate = async (path: string | undefined) => {
    setLoading(true)
    setError(null)
    try {
      const next = await browseFs(path)
      setListing(next)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setLoading(false)
    }
  }

  const doOpen = async (path: string) => {
    const p = path.trim()
    if (!p) return
    setSubmitting(true)
    try {
      await openProject(p)
      setOpen(false)
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>{trigger}</DialogTrigger>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Open project</DialogTitle>
          <DialogDescription>
            Browse to a project folder (one containing{" "}
            <code className="font-mono">project.toml</code>) and open it.
          </DialogDescription>
        </DialogHeader>

        {/* Current path + up button */}
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            className="h-8 shrink-0 px-2"
            disabled={!listing?.parent || loading}
            onClick={() => navigate(listing?.parent ?? undefined)}
            title="Up one folder"
          >
            <ArrowUp className="size-3.5" />
          </Button>
          <div className="min-w-0 flex-1 truncate rounded-md border border-border bg-muted/30 px-2 py-1.5 font-mono text-[11px] text-muted-foreground">
            {listing?.path ?? "…"}
          </div>
          {listing?.is_project && (
            <Button
              size="sm"
              className="h-8 shrink-0"
              disabled={submitting}
              onClick={() => doOpen(listing.path)}
            >
              Open this
            </Button>
          )}
        </div>

        {/* Folder list */}
        <div className="h-64 overflow-auto rounded-md border border-border">
          {loading ? (
            <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
              <Loader2 className="mr-2 size-4 animate-spin" /> Loading…
            </div>
          ) : error ? (
            <div className="p-3 text-sm text-red-600 dark:text-red-400">
              {error}
            </div>
          ) : !listing || listing.entries.length === 0 ? (
            <div className="grid h-full place-items-center text-sm text-muted-foreground">
              No sub-folders here.
            </div>
          ) : (
            <ul className="divide-y divide-border/60">
              {listing.entries.map((e) => (
                <li key={e.path}>
                  <button
                    type="button"
                    className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm hover:bg-accent/40"
                    onDoubleClick={() => navigate(e.path)}
                    onClick={() =>
                      e.is_project ? void doOpen(e.path) : navigate(e.path)
                    }
                    title={
                      e.is_project
                        ? "Open this project"
                        : "Open folder (double-click to enter)"
                    }
                  >
                    {e.is_project ? (
                      <FolderCheck className="size-4 shrink-0 text-emerald-600 dark:text-emerald-400" />
                    ) : (
                      <Folder className="size-4 shrink-0 text-muted-foreground" />
                    )}
                    <span className="truncate">{e.name}</span>
                    {e.is_project && (
                      <span className="ml-auto rounded bg-emerald-500/15 px-1.5 py-0.5 text-[9px] font-medium uppercase tracking-wider text-emerald-700 dark:text-emerald-400">
                        project
                      </span>
                    )}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>

        {/* Manual path fallback */}
        <Input
          placeholder="…or paste an absolute path"
          value={manual}
          onChange={(e) => setManual(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void doOpen(manual)
          }}
          className="font-mono text-xs"
        />

        <DialogFooter>
          <Button variant="ghost" onClick={() => setOpen(false)}>
            Cancel
          </Button>
          <Button
            onClick={() => doOpen(manual)}
            disabled={!manual.trim() || submitting}
          >
            Open path
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
