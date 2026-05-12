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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { useRuntime } from "@/state/runtime"
import type { PouType } from "@/types/generated/PouType"

type ControlledProps = {
  trigger?: undefined
  open: boolean
  onOpenChange: (next: boolean) => void
  /** Optional parent folder; the new POU is placed under it. */
  parent?: string
}

type UncontrolledProps = {
  trigger: React.ReactNode
  open?: undefined
  onOpenChange?: undefined
  parent?: string
}

type Props = ControlledProps | UncontrolledProps

export function NewPouDialog(props: Props) {
  const { createPou } = useRuntime()
  const [internalOpen, setInternalOpen] = useState(false)
  const open = props.open ?? internalOpen
  const setOpen = props.onOpenChange ?? setInternalOpen
  const parent = props.parent ?? ""
  const [name, setName] = useState("")
  const [kind, setKind] = useState<PouType>("program")
  const [submitting, setSubmitting] = useState(false)

  // Clear inputs each time the dialog opens, so re-opening for a
  // different folder doesn't leak last submission's text.
  useEffect(() => {
    if (open) {
      setName("")
      setKind("program")
    }
  }, [open, parent])

  const trimmed = name.trim()
  const fullPath = parent ? `${parent}/${trimmed}` : trimmed

  const submit = async () => {
    if (!trimmed) return
    setSubmitting(true)
    await createPou(fullPath, kind)
    setSubmitting(false)
    setOpen(false)
  }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      {props.trigger ? <DialogTrigger asChild>{props.trigger}</DialogTrigger> : null}
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            New POU{" "}
            {parent && (
              <span className="font-mono text-xs text-muted-foreground">
                under {parent}
              </span>
            )}
          </DialogTitle>
        </DialogHeader>
        <div className="space-y-3">
          <div className="space-y-2">
            <Label htmlFor="pou-name">Name</Label>
            <Input
              id="pou-name"
              placeholder="valve_logic"
              value={name}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") submit()
              }}
              autoFocus
            />
            {trimmed && parent && (
              <div className="font-mono text-[11px] text-muted-foreground">
                applications/{fullPath}.st
              </div>
            )}
          </div>
          <div className="space-y-2">
            <Label>Type</Label>
            <Select
              value={kind}
              onValueChange={(v) => setKind(v as PouType)}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="program">Program (ST)</SelectItem>
                <SelectItem value="function_block">
                  Function Block (ST)
                </SelectItem>
              </SelectContent>
            </Select>
          </div>
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={() => setOpen(false)}>
            Cancel
          </Button>
          <Button onClick={submit} disabled={!trimmed || submitting}>
            Create
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
