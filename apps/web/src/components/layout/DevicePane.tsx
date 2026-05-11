import { Plus, Save, Trash2 } from "lucide-react"
import { useEffect, useState } from "react"

import { Button } from "@/components/ui/button"
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
import type { Device } from "@/types/generated/Device"
import type { ModbusChannel } from "@/types/generated/ModbusChannel"
import type { ModbusChannelKind } from "@/types/generated/ModbusChannelKind"

export function DevicePane() {
  const { currentDevice, saveDevice } = useRuntime()

  if (!currentDevice) {
    return (
      <main className="flex min-h-0 min-w-0 flex-col">
        <Header title="Device" />
        <div className="grid flex-1 place-items-center text-sm text-muted-foreground">
          Select a device from the project tree.
        </div>
      </main>
    )
  }

  return (
    <main className="flex min-h-0 min-w-0 flex-col">
      {currentDevice.protocol === "modbus" ? (
        <ModbusDeviceEditor device={currentDevice} onSave={saveDevice} />
      ) : (
        <EthercatPlaceholder device={currentDevice} />
      )}
    </main>
  )
}

function Header({ title, badge }: { title: string; badge?: string }) {
  return (
    <div className="flex h-9 items-center gap-2 border-b border-border pl-3 pr-2 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
      <span className="truncate normal-case tracking-normal text-foreground">
        {title}
      </span>
      {badge && (
        <span className="rounded bg-muted px-1.5 py-0.5 font-mono text-[9px] uppercase tracking-wider">
          {badge}
        </span>
      )}
    </div>
  )
}

// ============================================================
//  Modbus form
// ============================================================

function ModbusDeviceEditor({
  device,
  onSave,
}: {
  device: Device
  onSave: (d: Device) => Promise<void>
}) {
  const initial = device
  const [draft, setDraft] = useState<Device>(initial)
  // Reset draft whenever the upstream device changes (e.g., selected a
  // different device).
  useEffect(() => {
    setDraft(device)
  }, [device])

  if (draft.protocol !== "modbus") return null
  const dirty = JSON.stringify(draft) !== JSON.stringify(initial)

  const update = (patch: Partial<typeof draft>) =>
    setDraft({ ...draft, ...patch } as Device)

  const setChannel = (idx: number, patch: Partial<ModbusChannel>) => {
    const next = [...draft.channels]
    next[idx] = { ...next[idx], ...patch }
    update({ channels: next })
  }

  const addChannel = () => {
    update({
      channels: [
        ...draft.channels,
        {
          name: `ch_${draft.channels.length}`,
          kind: "coil",
          address: 0,
        },
      ],
    })
  }

  const removeChannel = (idx: number) => {
    update({ channels: draft.channels.filter((_, i) => i !== idx) })
  }

  return (
    <>
      <div className="flex h-9 items-center justify-between border-b border-border pl-3 pr-2 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        <span className="flex items-center gap-2 truncate normal-case tracking-normal text-foreground">
          <span className="truncate font-mono">{device.name}</span>
          <span className="rounded bg-amber-500/15 px-1.5 py-0.5 font-mono text-[9px] uppercase tracking-wider text-amber-700 dark:text-amber-400">
            modbus
          </span>
          {dirty && (
            <span className="rounded bg-amber-500/15 px-1.5 py-0.5 font-mono text-[9px] uppercase tracking-wider text-amber-700 dark:text-amber-400">
              modified
            </span>
          )}
        </span>
        <Button
          size="sm"
          variant="outline"
          onClick={() => void onSave(draft)}
          disabled={!dirty}
        >
          <Save className="mr-1.5 size-3" />
          Save
        </Button>
      </div>

      <div className="flex-1 space-y-6 overflow-auto p-5">
        <section>
          <div className="mb-3 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
            Connection
          </div>
          <div className="grid grid-cols-2 gap-3 max-w-xl">
            <Field label="Host">
              <Input
                value={draft.host}
                onChange={(e) => update({ host: e.target.value })}
              />
            </Field>
            <Field label="Port">
              <Input
                type="number"
                value={draft.port}
                onChange={(e) =>
                  update({ port: Number(e.target.value) || 0 })
                }
              />
            </Field>
            <Field label="Slave ID">
              <Input
                type="number"
                value={draft.slave_id}
                onChange={(e) =>
                  update({ slave_id: Number(e.target.value) || 0 })
                }
              />
            </Field>
            <Field label="Poll interval (ms)">
              <Input
                type="number"
                value={draft.poll_interval_ms}
                onChange={(e) =>
                  update({
                    poll_interval_ms: Number(e.target.value) || 0,
                  } as Device)
                }
              />
            </Field>
          </div>
        </section>

        <section>
          <div className="mb-3 flex items-center justify-between">
            <div className="text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
              Channels
            </div>
            <Button size="sm" variant="ghost" onClick={addChannel}>
              <Plus className="mr-1 size-3" />
              Add channel
            </Button>
          </div>
          {draft.channels.length === 0 ? (
            <div className="rounded-md border border-dashed border-border p-4 text-center text-xs text-muted-foreground">
              No channels. Click <span className="font-mono">+ Add channel</span> to define one.
            </div>
          ) : (
            <table className="w-full max-w-2xl text-sm">
              <thead className="text-[10px] uppercase tracking-wider text-muted-foreground">
                <tr className="border-b border-border">
                  <th className="px-2 py-1.5 text-left">Name</th>
                  <th className="px-2 py-1.5 text-left">Kind</th>
                  <th className="px-2 py-1.5 text-left">Address</th>
                  <th className="px-2 py-1.5"></th>
                </tr>
              </thead>
              <tbody>
                {draft.channels.map((ch, i) => (
                  <tr key={i} className="border-b border-border last:border-0">
                    <td className="px-2 py-1.5">
                      <Input
                        value={ch.name}
                        onChange={(e) => setChannel(i, { name: e.target.value })}
                        className="h-8"
                      />
                    </td>
                    <td className="px-2 py-1.5">
                      <Select
                        value={ch.kind}
                        onValueChange={(v) =>
                          setChannel(i, { kind: v as ModbusChannelKind })
                        }
                      >
                        <SelectTrigger className="h-8 w-44">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="coil">Coil</SelectItem>
                          <SelectItem value="discrete_input">
                            Discrete Input
                          </SelectItem>
                          <SelectItem value="holding_register">
                            Holding Register
                          </SelectItem>
                          <SelectItem value="input_register">
                            Input Register
                          </SelectItem>
                        </SelectContent>
                      </Select>
                    </td>
                    <td className="px-2 py-1.5">
                      <Input
                        type="number"
                        value={ch.address}
                        onChange={(e) =>
                          setChannel(i, {
                            address: Number(e.target.value) || 0,
                          })
                        }
                        className="h-8 w-24"
                      />
                    </td>
                    <td className="px-2 py-1.5 text-right">
                      <button
                        type="button"
                        onClick={() => removeChannel(i)}
                        className="rounded p-1 text-muted-foreground hover:bg-accent/40 hover:text-red-600"
                        title="Remove"
                      >
                        <Trash2 className="size-3.5" />
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </section>
      </div>
    </>
  )
}

function Field({
  label,
  children,
}: {
  label: string
  children: React.ReactNode
}) {
  return (
    <div className="space-y-1.5">
      <Label className="text-[11px] uppercase tracking-wider text-muted-foreground">
        {label}
      </Label>
      {children}
    </div>
  )
}

// ============================================================
//  EtherCAT placeholder
// ============================================================

function EthercatPlaceholder({ device }: { device: Device }) {
  return (
    <>
      <Header title={device.name} badge="ethercat" />
      <div className="grid flex-1 place-items-center p-8 text-center text-sm text-muted-foreground">
        EtherCAT configuration UI isn't wired up yet. The device file lives
        on disk; runtime support comes in a follow-up.
      </div>
    </>
  )
}
