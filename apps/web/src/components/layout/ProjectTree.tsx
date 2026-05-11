import { useState } from "react"
import {
  ChevronDown,
  ChevronRight,
  Cpu,
  FileCode2,
  Network,
  Plus,
} from "lucide-react"

import { NewDeviceDialog } from "@/components/dialogs/NewDeviceDialog"
import { NewPouDialog } from "@/components/dialogs/NewPouDialog"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
import { cn } from "@/lib/utils"
import { useRuntime } from "@/state/runtime"
import type { ApplicationKind } from "@/types/generated/ApplicationKind"
import type { Protocol } from "@/types/generated/Protocol"

export function ProjectTree() {
  const { project, currentApp, selectApp, deleteApp, deleteDevice } =
    useRuntime()
  const [appsOpen, setAppsOpen] = useState(true)
  const [devicesOpen, setDevicesOpen] = useState(true)

  if (!project) return null

  return (
    <div className="py-1 text-sm">
      <SectionHeader
        label="Applications"
        open={appsOpen}
        count={project.applications.length}
        onToggle={() => setAppsOpen(!appsOpen)}
        action={
          <NewPouDialog
            trigger={
              <button
                type="button"
                title="New POU"
                onClick={(e) => e.stopPropagation()}
                className="flex size-5 items-center justify-center rounded text-muted-foreground hover:bg-accent/40 hover:text-foreground"
              >
                <Plus className="size-3.5" />
              </button>
            }
          />
        }
      />
      {appsOpen &&
        project.applications.map((a) => (
          <ContextMenu key={a.name}>
            <ContextMenuTrigger asChild>
              <button
                type="button"
                onClick={() => selectApp(a.name)}
                className={cn(
                  "flex w-full items-center gap-1.5 px-2 py-1 text-left transition-colors hover:bg-accent/40",
                  currentApp?.name === a.name && "bg-accent/60",
                )}
                style={{ paddingLeft: 26 }}
              >
                <FileCode2
                  className={cn(
                    "size-3.5 shrink-0",
                    a.kind === "function_block"
                      ? "text-violet-600 dark:text-violet-400"
                      : "text-sky-600 dark:text-sky-400",
                  )}
                />
                <span className="flex-1 truncate">{a.name}</span>
                <span className="font-mono text-[9px] uppercase text-muted-foreground">
                  {kindAbbrev(a.kind)}
                </span>
              </button>
            </ContextMenuTrigger>
            <ContextMenuContent>
              <ContextMenuItem onSelect={() => selectApp(a.name)}>
                Open
              </ContextMenuItem>
              <ContextMenuSeparator />
              <ContextMenuItem
                variant="destructive"
                onSelect={() => {
                  if (confirm(`Delete POU "${a.name}"?`)) {
                    void deleteApp(a.name)
                  }
                }}
              >
                Delete
              </ContextMenuItem>
            </ContextMenuContent>
          </ContextMenu>
        ))}
      {appsOpen && project.applications.length === 0 && (
        <div
          className="py-1 text-[11px] italic text-muted-foreground"
          style={{ paddingLeft: 26 }}
        >
          (none)
        </div>
      )}

      <SectionHeader
        label="Devices"
        open={devicesOpen}
        count={project.devices.length}
        onToggle={() => setDevicesOpen(!devicesOpen)}
        action={
          <NewDeviceDialog
            trigger={
              <button
                type="button"
                title="New device"
                onClick={(e) => e.stopPropagation()}
                className="flex size-5 items-center justify-center rounded text-muted-foreground hover:bg-accent/40 hover:text-foreground"
              >
                <Plus className="size-3.5" />
              </button>
            }
          />
        }
      />
      {devicesOpen &&
        project.devices.map((d) => (
          <ContextMenu key={d.name}>
            <ContextMenuTrigger asChild>
              <button
                type="button"
                className="flex w-full items-center gap-1.5 px-2 py-1 text-left transition-colors hover:bg-accent/40"
                style={{ paddingLeft: 26 }}
              >
                <ProtocolIcon protocol={d.protocol} />
                <span className="flex-1 truncate">{d.name}</span>
                <span className="font-mono text-[9px] uppercase text-muted-foreground">
                  {d.protocol}
                </span>
              </button>
            </ContextMenuTrigger>
            <ContextMenuContent>
              <ContextMenuItem
                variant="destructive"
                onSelect={() => {
                  if (confirm(`Delete device "${d.name}"?`)) {
                    void deleteDevice(d.name)
                  }
                }}
              >
                Delete
              </ContextMenuItem>
            </ContextMenuContent>
          </ContextMenu>
        ))}
      {devicesOpen && project.devices.length === 0 && (
        <div
          className="py-1 text-[11px] italic text-muted-foreground"
          style={{ paddingLeft: 26 }}
        >
          (none)
        </div>
      )}
    </div>
  )
}

function SectionHeader({
  label,
  open,
  count,
  onToggle,
  action,
}: {
  label: string
  open: boolean
  count: number
  onToggle: () => void
  action: React.ReactNode
}) {
  return (
    <div className="flex items-center justify-between pl-1 pr-1.5">
      <button
        type="button"
        onClick={onToggle}
        className="flex flex-1 items-center gap-1 py-1 text-left text-[11px] font-medium uppercase tracking-wider text-muted-foreground hover:text-foreground"
      >
        {open ? (
          <ChevronDown className="size-3" />
        ) : (
          <ChevronRight className="size-3" />
        )}
        {label}
        <span className="font-mono text-[10px] tracking-normal opacity-60">
          {count}
        </span>
      </button>
      {action}
    </div>
  )
}

function ProtocolIcon({ protocol }: { protocol: Protocol }) {
  if (protocol === "modbus") {
    return <Network className="size-3.5 shrink-0 text-amber-600 dark:text-amber-400" />
  }
  return <Cpu className="size-3.5 shrink-0 text-emerald-600 dark:text-emerald-400" />
}

function kindAbbrev(k: ApplicationKind): string {
  return k === "function_block" ? "fb" : "prg"
}
