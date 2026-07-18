/**
 * The project tree's HMI section — self-contained (screens live behind
 * their own endpoint, not in the project tree payload): fetches the list,
 * refreshes on `hmi` SSE mutations, creates via a small dialog, and
 * highlights the screen the HMI view is showing.
 */

import { useCallback, useEffect, useState } from "react"
import { ChevronDown, ChevronRight, MonitorDot, Plus, Trash2 } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { createHmi, deleteHmi, fetchHmis } from "@/lib/api"
import { cn } from "@/lib/utils"
import { useHmiMutation } from "@/state/hmi-live"
import { useRuntime } from "@/state/runtime"
import type { HmiListEntry } from "@/types/generated/HmiListEntry"

export function HmiSection() {
  const { view, currentHmi, selectHmi } = useRuntime()
  const [open, setOpen] = useState(true)
  const [screens, setScreens] = useState<HmiListEntry[]>([])
  const [dialogOpen, setDialogOpen] = useState(false)
  const mutation = useHmiMutation()

  const refresh = useCallback(async () => {
    try {
      setScreens(await fetchHmis())
    } catch {
      /* project may have just closed */
    }
  }, [])

  useEffect(() => {
    void refresh()
  }, [refresh])
  useEffect(() => {
    if (mutation) void refresh()
  }, [mutation, refresh])

  return (
    <div>
      <div className="group flex items-center gap-1 px-2 py-1">
        <button
          type="button"
          onClick={() => setOpen(!open)}
          className="flex min-w-0 flex-1 items-center gap-1 text-left text-[11px] font-medium uppercase tracking-wider text-muted-foreground hover:text-foreground"
        >
          {open ? (
            <ChevronDown className="size-3 shrink-0" />
          ) : (
            <ChevronRight className="size-3 shrink-0" />
          )}
          HMI
          <span className="font-mono text-[10px]">{screens.length}</span>
        </button>
        <button
          type="button"
          title="New screen"
          onClick={() => setDialogOpen(true)}
          className="rounded p-0.5 text-muted-foreground opacity-0 hover:bg-accent/50 hover:text-foreground group-hover:opacity-100"
        >
          <Plus className="size-3.5" />
        </button>
      </div>
      {open &&
        screens.map((s) => (
          <div
            key={s.path}
            className={cn(
              "group flex w-full items-center gap-1.5 py-0.5 pl-6 pr-2 text-[12px] transition-colors hover:bg-accent/40",
              view === "hmi" && currentHmi === s.path
                ? "bg-accent/60 text-foreground"
                : "text-muted-foreground",
            )}
          >
            <button
              type="button"
              onClick={() => void selectHmi(s.path)}
              className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
            >
              <MonitorDot className="size-3.5 shrink-0 text-muted-foreground" />
              <span className="truncate">{s.path}</span>
              <span className="ml-auto shrink-0 font-mono text-[9px] uppercase tracking-wider text-muted-foreground">
                L{s.level}
              </span>
            </button>
            <button
              type="button"
              title={`Delete screen "${s.path}"`}
              onClick={() => {
                if (confirm(`Delete screen "${s.path}"?`)) {
                  void deleteHmi(s.path).then(refresh)
                }
              }}
              className="rounded p-0.5 text-muted-foreground opacity-0 hover:text-destructive group-hover:opacity-100"
            >
              <Trash2 className="size-3" />
            </button>
          </div>
        ))}
      {open && screens.length === 0 && (
        <div className="py-1 pl-6 pr-2 text-[11px] text-muted-foreground/70">
          No screens — click + or run `cs hmi generate`.
        </div>
      )}
      <NewHmiDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        onCreated={(path) => {
          void refresh().then(() => selectHmi(path))
        }}
      />
    </div>
  )
}

function NewHmiDialog({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean
  onOpenChange: (v: boolean) => void
  onCreated: (path: string) => void
}) {
  const [name, setName] = useState("")
  const [title, setTitle] = useState("")
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (open) {
      setName("")
      setTitle("")
      setError(null)
    }
  }, [open])

  const create = async () => {
    const slug = name.trim()
    if (!slug) return
    try {
      await createHmi(slug, title.trim() || undefined)
      onOpenChange(false)
      onCreated(slug)
    } catch (e) {
      setError(String(e))
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[380px]">
        <DialogHeader>
          <DialogTitle>New HMI screen</DialogTitle>
        </DialogHeader>
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label htmlFor="hmi-name">Name (slug)</Label>
            <Input
              id="hmi-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="overview"
              autoFocus
              onKeyDown={(e) => {
                if (e.key === "Enter") void create()
              }}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="hmi-title">Title (operator-facing)</Label>
            <Input
              id="hmi-title"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="Plant — Overview"
              onKeyDown={(e) => {
                if (e.key === "Enter") void create()
              }}
            />
          </div>
          {error && (
            <div className="text-[11px] text-destructive">{error}</div>
          )}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button onClick={() => void create()} disabled={!name.trim()}>
            Create
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
