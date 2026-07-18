import { useEffect, useState } from "react"

import { fetchPouVariables } from "@/lib/api"
import { useRuntime } from "@/state/runtime"
import type { VariableInfo } from "@/types/generated/VariableInfo"

import { CanopenDeviceEditor } from "./CanopenDeviceEditor"
import { EthercatDeviceEditor } from "./EthercatDeviceEditor"
import { ModbusDeviceEditor } from "./ModbusDeviceEditor"
import { OpcuaDeviceEditor } from "./OpcuaDeviceEditor"
import type { LinkProps } from "./deviceEditorShared"

/**
 * Thin dispatcher: prefetch the per-POU variable lists (so the inline
 * add-binding form has autocomplete without per-row latency), then hand
 * off to the protocol-specific editor. The per-protocol column sets differ
 * enough (Modbus registers vs EtherCAT PDO offsets vs OPC UA NodeIds vs
 * CANopen object entries) that each editor owns its own table. Genuinely
 * shared bits (draft scaffold, save bar, LinkedToCell) live in
 * `deviceEditorShared`.
 */
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
      ) : currentDevice.protocol === "opcua" ? (
        <OpcuaDeviceEditor
          device={currentDevice}
          onSave={saveDevice}
          link={linkProps}
        />
      ) : currentDevice.protocol === "canopen" ? (
        <CanopenDeviceEditor
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
