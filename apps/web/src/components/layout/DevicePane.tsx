import { Info, Link2, Plus, Save, Trash2, X } from "lucide-react"
import { useEffect, useMemo, useState } from "react"

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
import { fetchPouVariables } from "@/lib/api"
import { useRuntime } from "@/state/runtime"
import type { Device } from "@/types/generated/Device"
import type { Direction } from "@/types/generated/Direction"
import type { EthercatChannel } from "@/types/generated/EthercatChannel"
import type { EthercatDataType } from "@/types/generated/EthercatDataType"
import type { EthercatPdoDirection } from "@/types/generated/EthercatPdoDirection"
import type { EthercatSlave } from "@/types/generated/EthercatSlave"
import type { IoMap } from "@/types/generated/IoMap"
import type { Mapping } from "@/types/generated/Mapping"
import type { ModbusChannel } from "@/types/generated/ModbusChannel"
import type { ModbusChannelKind } from "@/types/generated/ModbusChannelKind"
import type { VariableInfo } from "@/types/generated/VariableInfo"

export function DevicePane() {
  const { currentDevice, project, iomap, saveDevice, saveIomap } = useRuntime()
  const [varsByApp, setVarsByApp] = useState<Record<string, VariableInfo[]>>({})

  // Pre-fetch variables for every POU once, so the inline add-binding form
  // can offer autocomplete without per-row latency.
  useEffect(() => {
    if (!project) return
    let cancelled = false
    Promise.all(
      project.pous.map((p) =>
        fetchPouVariables(p.path)
          .then((vs) => [p.path, vs] as const)
          .catch(() => [p.path, [] as VariableInfo[]] as const),
      ),
    ).then((entries) => {
      if (!cancelled) setVarsByApp(Object.fromEntries(entries))
    })
    return () => {
      cancelled = true
    }
  }, [project])

  if (!currentDevice) {
    return (
      <main className="flex h-full min-h-0 min-w-0 flex-col">
        <Header title="Device" />
        <div className="grid flex-1 place-items-center text-sm text-muted-foreground">
          Select a device from the project tree.
        </div>
      </main>
    )
  }

  // Editing through the Linked-to column commits straight to iomap.toml.
  // The Mapping wire-format identifies the device by name, so we just
  // splice the entries for this device.
  const linkProps: LinkProps = {
    deviceName: currentDevice.name,
    iomap,
    saveIomap,
    apps: project?.pous.map((p) => p.path) ?? [],
    varsByApp,
  }

  return (
    <main className="flex h-full min-h-0 min-w-0 flex-col">
      {currentDevice.protocol === "modbus" ? (
        <ModbusDeviceEditor
          device={currentDevice}
          onSave={saveDevice}
          link={linkProps}
        />
      ) : (
        <EthercatDeviceEditor
          device={currentDevice}
          onSave={saveDevice}
          link={linkProps}
        />
      )}
    </main>
  )
}

/** Bundled props the Linked-to column needs from the runtime. Passed down
 * to both Modbus and EtherCAT editors so each channel row can render the
 * same LinkedToCell. */
type LinkProps = {
  deviceName: string
  iomap: IoMap
  saveIomap: (next: IoMap) => Promise<void>
  apps: string[]
  varsByApp: Record<string, VariableInfo[]>
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
  link,
}: {
  device: Device
  onSave: (d: Device) => Promise<void>
  link: LinkProps
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
                  })
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
            <table className="w-full text-sm">
              <thead className="text-[10px] uppercase tracking-wider text-muted-foreground">
                <tr className="border-b border-border">
                  <th className="px-2 py-1.5 text-left">Name</th>
                  <th className="px-2 py-1.5 text-left">Kind</th>
                  <th className="px-2 py-1.5 text-left">Address</th>
                  <th className="px-2 py-1.5 text-left">Linked to</th>
                  <th className="px-2 py-1.5"></th>
                </tr>
              </thead>
              <tbody>
                {draft.channels.map((ch, i) => (
                  <tr
                    key={i}
                    className="border-b border-border align-top last:border-0"
                  >
                    <td className="px-2 py-1.5">
                      <Input
                        value={ch.name}
                        onChange={(e) => setChannel(i, { name: e.target.value })}
                        className="h-8 w-40"
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
                    <td className="px-2 py-1.5">
                      <LinkedToCell channelName={ch.name} link={link} />
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
//  EtherCAT form
// ============================================================

const PDO_DATA_TYPES: { value: EthercatDataType; label: string; bits: number }[] = [
  { value: "bool", label: "BOOL", bits: 1 },
  { value: "u8", label: "USINT (u8)", bits: 8 },
  { value: "i8", label: "SINT (i8)", bits: 8 },
  { value: "u16", label: "UINT (u16)", bits: 16 },
  { value: "i16", label: "INT (i16)", bits: 16 },
  { value: "u32", label: "UDINT (u32)", bits: 32 },
  { value: "i32", label: "DINT (i32)", bits: 32 },
  { value: "real", label: "REAL (f32)", bits: 32 },
]

function defaultBitsFor(t: EthercatDataType): number {
  return PDO_DATA_TYPES.find((d) => d.value === t)?.bits ?? 1
}

/** Render a u16/u32 as 0x-prefixed hex; used for object dictionary indices. */
function toHex(n: number, width: number): string {
  if (!Number.isFinite(n) || n < 0) return "0x0"
  return `0x${n.toString(16).toUpperCase().padStart(width, "0")}`
}

/** Parse user-entered hex (with or without 0x) or decimal. Returns 0 on junk. */
function parseHexOrDec(s: string): number {
  const t = s.trim()
  if (!t) return 0
  const cleaned = t.toLowerCase().startsWith("0x") ? t : t.match(/^[0-9a-f]+$/i) && /[a-f]/i.test(t) ? `0x${t}` : t
  const n = Number(cleaned)
  return Number.isFinite(n) ? Math.max(0, Math.trunc(n)) : 0
}

function EthercatDeviceEditor({
  device,
  onSave,
  link,
}: {
  device: Device
  onSave: (d: Device) => Promise<void>
  link: LinkProps
}) {
  const [draft, setDraft] = useState<Device>(device)
  useEffect(() => {
    setDraft(device)
  }, [device])

  if (draft.protocol !== "ethercat") return null
  const dirty = JSON.stringify(draft) !== JSON.stringify(device)

  const update = (patch: Partial<typeof draft>) =>
    setDraft({ ...draft, ...patch } as Device)

  // ----- Slaves -----
  const setSlave = (idx: number, patch: Partial<EthercatSlave>) => {
    const next = [...draft.slaves]
    next[idx] = { ...next[idx], ...patch }
    update({ slaves: next })
  }
  const addSlave = () => {
    const used = new Set(draft.slaves.map((s) => s.index))
    let nextIndex = 0
    while (used.has(nextIndex)) nextIndex++
    update({
      slaves: [
        ...draft.slaves,
        {
          index: nextIndex,
          name: `slave_${nextIndex}`,
          vendor_id: 0,
          product_id: 0,
        },
      ],
    })
  }
  const removeSlave = (idx: number) => {
    update({ slaves: draft.slaves.filter((_, i) => i !== idx) })
  }

  // ----- Channels -----
  const setChannel = (idx: number, patch: Partial<EthercatChannel>) => {
    const next = [...draft.channels]
    next[idx] = { ...next[idx], ...patch }
    update({ channels: next })
  }
  const addChannel = () => {
    const firstSlave = draft.slaves[0]?.index ?? 0
    update({
      channels: [
        ...draft.channels,
        {
          name: `pdo_${draft.channels.length}`,
          slave_index: firstSlave,
          direction: "tx_pdo",
          pdo_index: 0x6000,
          sub_index: 1,
          bit_length: 16,
          data_type: "u16",
          // Default to (0, 0): correct for the first PDO entry on a
          // single-channel slave. Multi-channel slaves need real
          // numbers from the slave's ESI / vendor docs.
          pdi_byte_offset: 0,
          pdi_bit_offset: 0,
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
          <span className="rounded bg-sky-500/15 px-1.5 py-0.5 font-mono text-[9px] uppercase tracking-wider text-sky-700 dark:text-sky-400">
            ethercat
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
        {draft.nic === "_sim" || draft.nic.trim() === "" ? (
          <div className="flex items-start gap-2 rounded-md border border-sky-500/30 bg-sky-500/5 px-3 py-2 text-[12px] text-sky-900 dark:text-sky-200">
            <Info className="mt-0.5 size-3.5 shrink-0" />
            <div>
              <div className="font-medium">Simulation mode</div>
              <div className="text-[11px] text-sky-900/80 dark:text-sky-200/80">
                <span className="font-mono">nic = "_sim"</span> selects an
                in-memory PDO buffer. Output channels echo what the program
                writes; inputs start at zero. Set <span className="font-mono">nic</span>{" "}
                to a real interface name (e.g. <span className="font-mono">eth0</span>)
                to drive an actual EtherCAT bus via{" "}
                <span className="font-mono">ethercrab</span>. Real mode requires
                Linux + <span className="font-mono">CAP_NET_RAW</span>.
              </div>
            </div>
          </div>
        ) : (
          <div className="flex items-start gap-2 rounded-md border border-emerald-500/30 bg-emerald-500/5 px-3 py-2 text-[12px] text-emerald-900 dark:text-emerald-200">
            <Info className="mt-0.5 size-3.5 shrink-0" />
            <div>
              <div className="font-medium">Real-bus mode</div>
              <div className="text-[11px] text-emerald-900/80 dark:text-emerald-200/80">
                The runtime will open a raw socket on{" "}
                <span className="font-mono">{draft.nic}</span> and walk the
                EtherCAT bus on startup. Each channel needs an accurate{" "}
                <span className="font-mono">Byte off</span> /{" "}
                <span className="font-mono">Bit off</span> within the
                SubDevice's PDI region — pull these from the slave's ESI / vendor
                docs. To go back to simulation, set <span className="font-mono">nic = "_sim"</span>.
              </div>
            </div>
          </div>
        )}

        <section>
          <div className="mb-3 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
            MainDevice
          </div>
          <div className="grid grid-cols-2 gap-3 max-w-xl">
            <Field label="Network interface">
              <Input
                value={draft.nic}
                onChange={(e) => update({ nic: e.target.value })}
                placeholder="en0 / eth0"
              />
            </Field>
            <Field label="Cycle time (µs)">
              <Input
                type="number"
                min={50}
                step={50}
                value={draft.cycle_us}
                onChange={(e) =>
                  update({ cycle_us: Math.max(50, Number(e.target.value) || 0) })
                }
              />
            </Field>
          </div>
        </section>

        <section>
          <div className="mb-3 flex items-center justify-between">
            <div className="text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
              SubDevices ({draft.slaves.length})
            </div>
            <Button size="sm" variant="ghost" onClick={addSlave}>
              <Plus className="mr-1 size-3" />
              Add slave
            </Button>
          </div>
          {draft.slaves.length === 0 ? (
            <div className="rounded-md border border-dashed border-border p-4 text-center text-xs text-muted-foreground">
              No slaves declared. Click{" "}
              <span className="font-mono">+ Add slave</span> to describe one
              SubDevice on the ring.
            </div>
          ) : (
            <table className="w-full max-w-3xl text-sm">
              <thead className="text-[10px] uppercase tracking-wider text-muted-foreground">
                <tr className="border-b border-border">
                  <th className="px-2 py-1.5 text-left">Index</th>
                  <th className="px-2 py-1.5 text-left">Name</th>
                  <th className="px-2 py-1.5 text-left">Vendor ID</th>
                  <th className="px-2 py-1.5 text-left">Product code</th>
                  <th className="px-2 py-1.5"></th>
                </tr>
              </thead>
              <tbody>
                {draft.slaves.map((s, i) => (
                  <tr key={i} className="border-b border-border last:border-0">
                    <td className="px-2 py-1.5">
                      <Input
                        type="number"
                        min={0}
                        value={s.index}
                        onChange={(e) =>
                          setSlave(i, {
                            index: Math.max(0, Number(e.target.value) || 0),
                          })
                        }
                        className="h-8 w-20"
                      />
                    </td>
                    <td className="px-2 py-1.5">
                      <Input
                        value={s.name}
                        onChange={(e) => setSlave(i, { name: e.target.value })}
                        className="h-8 w-40"
                      />
                    </td>
                    <td className="px-2 py-1.5">
                      <Input
                        value={toHex(s.vendor_id, 8)}
                        onChange={(e) =>
                          setSlave(i, { vendor_id: parseHexOrDec(e.target.value) })
                        }
                        className="h-8 w-32 font-mono"
                        placeholder="0x00000000"
                      />
                    </td>
                    <td className="px-2 py-1.5">
                      <Input
                        value={toHex(s.product_id, 8)}
                        onChange={(e) =>
                          setSlave(i, { product_id: parseHexOrDec(e.target.value) })
                        }
                        className="h-8 w-32 font-mono"
                        placeholder="0x00000000"
                      />
                    </td>
                    <td className="px-2 py-1.5 text-right">
                      <button
                        type="button"
                        onClick={() => removeSlave(i)}
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

        <section>
          <div className="mb-3 flex items-center justify-between">
            <div className="text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
              PDO Channels ({draft.channels.length})
            </div>
            <Button
              size="sm"
              variant="ghost"
              onClick={addChannel}
              disabled={draft.slaves.length === 0}
              title={
                draft.slaves.length === 0
                  ? "Add at least one slave first"
                  : "Add a PDO channel"
              }
            >
              <Plus className="mr-1 size-3" />
              Add channel
            </Button>
          </div>
          {draft.channels.length === 0 ? (
            <div className="rounded-md border border-dashed border-border p-4 text-center text-xs text-muted-foreground">
              {draft.slaves.length === 0
                ? "Declare a slave first, then bind PDO entries here."
                : "No channels. Each channel maps a single PDO entry (object dictionary index + sub-index) on one slave."}
            </div>
          ) : (
            <table className="w-full text-sm">
              <thead className="text-[10px] uppercase tracking-wider text-muted-foreground">
                <tr className="border-b border-border">
                  <th className="px-2 py-1.5 text-left">Name</th>
                  <th className="px-2 py-1.5 text-left">Slave</th>
                  <th className="px-2 py-1.5 text-left">Direction</th>
                  <th className="px-2 py-1.5 text-left">PDO index</th>
                  <th className="px-2 py-1.5 text-left">Sub</th>
                  <th className="px-2 py-1.5 text-left">Type</th>
                  <th className="px-2 py-1.5 text-left">Bits</th>
                  <th
                    className="px-2 py-1.5 text-left"
                    title="Byte offset within the SubDevice's PDI region for this direction. Required when running against real hardware; ignored in sim mode."
                  >
                    Byte&nbsp;off
                  </th>
                  <th
                    className="px-2 py-1.5 text-left"
                    title="Bit offset within the byte at Byte off. Only meaningful for sub-byte (1-bit) channels like digital I/O. 0 = LSB."
                  >
                    Bit&nbsp;off
                  </th>
                  <th className="px-2 py-1.5 text-left">Linked to</th>
                  <th className="px-2 py-1.5"></th>
                </tr>
              </thead>
              <tbody>
                {draft.channels.map((ch, i) => (
                  <tr
                    key={i}
                    className="border-b border-border align-top last:border-0"
                  >
                    <td className="px-2 py-1.5">
                      <Input
                        value={ch.name}
                        onChange={(e) =>
                          setChannel(i, { name: e.target.value })
                        }
                        className="h-8 w-40"
                      />
                    </td>
                    <td className="px-2 py-1.5">
                      <Select
                        value={String(ch.slave_index)}
                        onValueChange={(v) =>
                          setChannel(i, { slave_index: Number(v) })
                        }
                      >
                        <SelectTrigger className="h-8 w-40">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {draft.slaves.map((s) => (
                            <SelectItem key={s.index} value={String(s.index)}>
                              [{s.index}] {s.name}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </td>
                    <td className="px-2 py-1.5">
                      <Select
                        value={ch.direction}
                        onValueChange={(v) =>
                          setChannel(i, {
                            direction: v as EthercatPdoDirection,
                          })
                        }
                      >
                        <SelectTrigger className="h-8 w-36">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="tx_pdo">TxPDO (in)</SelectItem>
                          <SelectItem value="rx_pdo">RxPDO (out)</SelectItem>
                        </SelectContent>
                      </Select>
                    </td>
                    <td className="px-2 py-1.5">
                      <Input
                        value={toHex(ch.pdo_index, 4)}
                        onChange={(e) =>
                          setChannel(i, {
                            pdo_index: parseHexOrDec(e.target.value),
                          })
                        }
                        className="h-8 w-24 font-mono"
                        placeholder="0x6000"
                      />
                    </td>
                    <td className="px-2 py-1.5">
                      <Input
                        type="number"
                        min={0}
                        max={255}
                        value={ch.sub_index}
                        onChange={(e) =>
                          setChannel(i, {
                            sub_index: Math.max(
                              0,
                              Math.min(255, Number(e.target.value) || 0),
                            ),
                          })
                        }
                        className="h-8 w-16"
                      />
                    </td>
                    <td className="px-2 py-1.5">
                      <Select
                        value={ch.data_type}
                        onValueChange={(v) => {
                          const dt = v as EthercatDataType
                          setChannel(i, {
                            data_type: dt,
                            // Snap bit length to the type's natural width.
                            bit_length: defaultBitsFor(dt),
                          })
                        }}
                      >
                        <SelectTrigger className="h-8 w-36">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {PDO_DATA_TYPES.map((t) => (
                            <SelectItem key={t.value} value={t.value}>
                              {t.label}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </td>
                    <td className="px-2 py-1.5">
                      <Input
                        type="number"
                        min={1}
                        max={64}
                        value={ch.bit_length}
                        onChange={(e) =>
                          setChannel(i, {
                            bit_length: Math.max(
                              1,
                              Math.min(64, Number(e.target.value) || 1),
                            ),
                          })
                        }
                        className="h-8 w-16"
                      />
                    </td>
                    <td className="px-2 py-1.5">
                      <Input
                        type="number"
                        min={0}
                        max={65535}
                        value={ch.pdi_byte_offset}
                        onChange={(e) =>
                          setChannel(i, {
                            pdi_byte_offset: Math.max(
                              0,
                              Math.min(65535, Number(e.target.value) || 0),
                            ),
                          })
                        }
                        className="h-8 w-16"
                      />
                    </td>
                    <td className="px-2 py-1.5">
                      <Input
                        type="number"
                        min={0}
                        max={7}
                        value={ch.pdi_bit_offset}
                        onChange={(e) =>
                          setChannel(i, {
                            pdi_bit_offset: Math.max(
                              0,
                              Math.min(7, Number(e.target.value) || 0),
                            ),
                          })
                        }
                        className="h-8 w-14"
                      />
                    </td>
                    <td className="px-2 py-1.5">
                      <LinkedToCell channelName={ch.name} link={link} />
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

// ============================================================
//  LinkedToCell — shared across Modbus + EtherCAT channel rows
// ============================================================

/** Stable string key for one Mapping → used to identify pills for removal. */
function mappingKey(m: Mapping): string {
  return `${m.application}::${m.variable}::${m.direction}`
}

function LinkedToCell({
  channelName,
  link,
}: {
  channelName: string
  link: LinkProps
}) {
  const [adding, setAdding] = useState(false)
  const [draft, setDraft] = useState<{
    application: string
    variable: string
    direction: Direction
  }>({
    application: link.apps[0] ?? "",
    variable: "",
    direction: "output",
  })

  const bindings = useMemo(
    () =>
      link.iomap.mappings.filter(
        (m) => m.device === link.deviceName && m.channel === channelName,
      ),
    [link.iomap, link.deviceName, channelName],
  )

  // Variables for the currently-selected app, used for the variable input
  // datalist. Empty array if the app hasn't been fetched yet.
  const appVars = link.varsByApp[draft.application] ?? []
  const varListId = `link-vars-${link.deviceName}-${channelName}-${draft.application || "_"}`

  const remove = async (target: Mapping) => {
    const next: IoMap = {
      mappings: link.iomap.mappings.filter(
        (m) => mappingKey(m) !== mappingKey(target) || m.channel !== channelName,
      ),
    }
    await link.saveIomap(next)
  }

  const commit = async () => {
    const variable = draft.variable.trim()
    if (!variable || !draft.application || !channelName) return
    const newMapping: Mapping = {
      application: draft.application,
      variable,
      direction: draft.direction,
      device: link.deviceName,
      channel: channelName,
    }
    // De-duplicate: if an identical mapping already exists, skip the write.
    const exists = link.iomap.mappings.some(
      (m) =>
        m.application === newMapping.application &&
        m.variable === newMapping.variable &&
        m.direction === newMapping.direction &&
        m.device === newMapping.device &&
        m.channel === newMapping.channel,
    )
    if (!exists) {
      await link.saveIomap({
        mappings: [...link.iomap.mappings, newMapping],
      })
    }
    setDraft((d) => ({ ...d, variable: "" }))
    setAdding(false)
  }

  // Channels with empty names can't be bound (iomap targets by name string).
  if (!channelName) {
    return (
      <span className="text-[11px] italic text-muted-foreground">
        (name this channel first)
      </span>
    )
  }

  return (
    <div className="flex flex-wrap items-center gap-1">
      {bindings.map((m) => (
        <span
          key={mappingKey(m)}
          className="inline-flex items-center gap-1 rounded-md border border-border bg-muted/40 px-1.5 py-0.5 text-[11px] font-mono"
          title={`${m.application}.${m.variable} (${m.direction === "input" ? "bus → var" : "var → bus"})`}
        >
          <span
            className={
              m.direction === "input"
                ? "text-sky-700 dark:text-sky-400"
                : "text-emerald-700 dark:text-emerald-400"
            }
          >
            {m.direction === "input" ? "←" : "→"}
          </span>
          <span className="truncate max-w-[14ch]">
            {m.application}.{m.variable}
          </span>
          <button
            type="button"
            onClick={() => void remove(m)}
            className="rounded text-muted-foreground hover:text-red-600"
            title="Unlink"
          >
            <X className="size-3" />
          </button>
        </span>
      ))}
      {adding ? (
        <div className="flex flex-wrap items-center gap-1 rounded-md border border-dashed border-border bg-background/60 p-1">
          <Select
            value={draft.application}
            onValueChange={(v) => setDraft({ ...draft, application: v })}
          >
            <SelectTrigger className="h-7 w-32 text-[11px]">
              <SelectValue placeholder="app" />
            </SelectTrigger>
            <SelectContent>
              {link.apps.map((a) => (
                <SelectItem key={a} value={a}>
                  {a}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Input
            list={varListId}
            value={draft.variable}
            onChange={(e) => setDraft({ ...draft, variable: e.target.value })}
            onKeyDown={(e) => {
              if (e.key === "Enter") void commit()
              if (e.key === "Escape") setAdding(false)
            }}
            placeholder="variable"
            className="h-7 w-32 text-[11px]"
            autoFocus
          />
          <datalist id={varListId}>
            {appVars.map((v) => (
              <option key={v.name} value={v.name}>
                {v.type_name} · {v.direction}
              </option>
            ))}
          </datalist>
          <Select
            value={draft.direction}
            onValueChange={(v) =>
              setDraft({ ...draft, direction: v as Direction })
            }
          >
            <SelectTrigger className="h-7 w-24 text-[11px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="output">→ output</SelectItem>
              <SelectItem value="input">← input</SelectItem>
            </SelectContent>
          </Select>
          <Button
            size="sm"
            variant="outline"
            onClick={() => void commit()}
            disabled={!draft.variable.trim() || !draft.application}
            className="h-7 px-2 text-[11px]"
          >
            Link
          </Button>
          <button
            type="button"
            onClick={() => setAdding(false)}
            className="rounded p-1 text-muted-foreground hover:text-foreground"
            title="Cancel"
          >
            <X className="size-3" />
          </button>
        </div>
      ) : (
        <button
          type="button"
          onClick={() => {
            setAdding(true)
            // If the previously-chosen app no longer exists, fall back.
            if (!link.apps.includes(draft.application)) {
              setDraft((d) => ({ ...d, application: link.apps[0] ?? "" }))
            }
          }}
          className="inline-flex items-center gap-1 rounded-md border border-dashed border-border px-1.5 py-0.5 text-[11px] text-muted-foreground hover:bg-accent/40 hover:text-foreground"
          title="Link this channel to a POU variable"
        >
          <Link2 className="size-3" />
          link
        </button>
      )}
    </div>
  )
}
