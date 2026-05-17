import { useState } from "react"

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
import { Label } from "@/components/ui/label"
import { useRuntime } from "@/state/runtime"

type Props = {
  trigger: React.ReactNode
}

export function OpenProjectDialog({ trigger }: Props) {
  const { openProject } = useRuntime()
  const [open, setOpen] = useState(false)
  const [path, setPath] = useState("")
  const [submitting, setSubmitting] = useState(false)
  const trimmed = path.trim()

  const submit = async () => {
    if (!trimmed) return
    setSubmitting(true)
    await openProject(trimmed)
    setSubmitting(false)
    setOpen(false)
    setPath("")
  }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>{trigger}</DialogTrigger>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Open project</DialogTitle>
          <DialogDescription>
            Absolute path to a project directory containing{" "}
            <code className="font-mono">project.toml</code>.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-2">
          <Label htmlFor="project-path">Project path</Label>
          <Input
            id="project-path"
            placeholder="/Users/you/Documents/IA2/my-controller"
            value={path}
            onChange={(e) => setPath(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") submit()
            }}
            autoFocus
          />
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={() => setOpen(false)}>
            Cancel
          </Button>
          <Button onClick={submit} disabled={!trimmed || submitting}>
            Open
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
