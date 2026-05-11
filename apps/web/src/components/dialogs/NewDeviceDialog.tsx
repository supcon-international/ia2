import { useState } from "react"

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
import type { Protocol } from "@/types/generated/Protocol"

type Props = {
  trigger: React.ReactNode
}

export function NewDeviceDialog({ trigger }: Props) {
  const { createDevice } = useRuntime()
  const [open, setOpen] = useState(false)
  const [name, setName] = useState("")
  const [protocol, setProtocol] = useState<Protocol>("modbus")
  const [submitting, setSubmitting] = useState(false)
  const trimmed = name.trim()

  const submit = async () => {
    if (!trimmed) return
    setSubmitting(true)
    await createDevice(trimmed, protocol)
    setSubmitting(false)
    setOpen(false)
    setName("")
    setProtocol("modbus")
  }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>{trigger}</DialogTrigger>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>New device</DialogTitle>
        </DialogHeader>
        <div className="space-y-3">
          <div className="space-y-2">
            <Label htmlFor="device-name">Name</Label>
            <Input
              id="device-name"
              placeholder="tank1"
              value={name}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") submit()
              }}
              autoFocus
            />
          </div>
          <div className="space-y-2">
            <Label>Protocol</Label>
            <Select
              value={protocol}
              onValueChange={(v) => setProtocol(v as Protocol)}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="modbus">Modbus TCP</SelectItem>
                <SelectItem value="ethercat">EtherCAT</SelectItem>
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
