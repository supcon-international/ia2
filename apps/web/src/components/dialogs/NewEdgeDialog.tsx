import { useEffect, useState } from "react"

import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useRuntime } from "@/state/runtime"

type ControlledProps = {
  trigger?: undefined
  open: boolean
  onOpenChange: (next: boolean) => void
  parent?: string
}

type UncontrolledProps = {
  trigger: React.ReactNode
  open?: undefined
  onOpenChange?: undefined
  parent?: string
}

type Props = ControlledProps | UncontrolledProps

export function NewEdgeDialog(props: Props) {
  const { createEdge } = useRuntime()
  const [internalOpen, setInternalOpen] = useState(false)
  const open = props.open ?? internalOpen
  const setOpen = props.onOpenChange ?? setInternalOpen
  const parent = props.parent ?? ""
  const [name, setName] = useState("")
  const [host, setHost] = useState("")
  const [submitting, setSubmitting] = useState(false)

  useEffect(() => {
    if (open) {
      setName("")
      setHost("")
    }
  }, [open, parent])

  const trimmedName = name.trim()
  const trimmedHost = host.trim()
  const fullPath = parent ? `${parent}/${trimmedName}` : trimmedName

  const submit = async () => {
    if (!trimmedName || !trimmedHost) return
    setSubmitting(true)
    const ok = await createEdge(fullPath, trimmedHost)
    setSubmitting(false)
    // Stay open on failure; the error shows in the global toast.
    if (ok) setOpen(false)
  }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      {props.trigger ? (
        <DialogTrigger asChild>{props.trigger}</DialogTrigger>
      ) : null}
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            New edge{" "}
            {parent && (
              <span className="font-mono text-xs text-muted-foreground">
                under {parent}
              </span>
            )}
          </DialogTitle>
        </DialogHeader>
        <div className="space-y-3">
          <div className="space-y-2">
            <Label htmlFor="edge-name">Name</Label>
            <Input
              id="edge-name"
              placeholder="line1-controller"
              value={name}
              onChange={(e) => setName(e.target.value)}
              autoFocus
            />
            {trimmedName && parent && (
              <div className="font-mono text-[11px] text-muted-foreground">
                edges/{fullPath}.toml
              </div>
            )}
          </div>
          <div className="space-y-2">
            <Label htmlFor="edge-host">SSH host or ~/.ssh/config alias</Label>
            <Input
              id="edge-host"
              placeholder="line1.lan or production-line-1"
              value={host}
              onChange={(e) => setHost(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") submit()
              }}
            />
            <p className="text-[11px] text-muted-foreground">
              The IDE runs <span className="font-mono">ssh {host || "<host>"}</span>{" "}
              — credentials come from your SSH agent / ~/.ssh/config, never
              stored in the project.
            </p>
          </div>
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={() => setOpen(false)}>
            Cancel
          </Button>
          <Button
            onClick={submit}
            disabled={!trimmedName || !trimmedHost || submitting}
          >
            Create
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
