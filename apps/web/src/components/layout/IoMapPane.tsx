import { Plus, Save, Trash2 } from "lucide-react"
import { useEffect, useState } from "react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { useRuntime } from "@/state/runtime"
import type { Direction } from "@/types/generated/Direction"
import type { IoMap } from "@/types/generated/IoMap"
import type { Mapping } from "@/types/generated/Mapping"

export function IoMapPane() {
  const { project, iomap, saveIomap } = useRuntime()
  const [draft, setDraft] = useState<IoMap>(iomap)
  useEffect(() => setDraft(iomap), [iomap])

  const dirty = JSON.stringify(draft) !== JSON.stringify(iomap)

  const set = (idx: number, patch: Partial<Mapping>) => {
    const next = draft.mappings.map((m, i) =>
      i === idx ? { ...m, ...patch } : m,
    )
    setDraft({ mappings: next })
  }

  const add = () => {
    setDraft({
      mappings: [
        ...draft.mappings,
        {
          application: project?.applications[0]?.name ?? "",
          variable: "",
          direction: "output",
          device: project?.devices[0]?.name ?? "",
          channel: "",
        },
      ],
    })
  }

  const remove = (idx: number) => {
    setDraft({ mappings: draft.mappings.filter((_, i) => i !== idx) })
  }

  return (
    <main className="flex min-h-0 min-w-0 flex-col">
      <div className="flex h-9 items-center justify-between border-b border-border pl-3 pr-2 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        <span className="truncate normal-case tracking-normal text-foreground">
          IO Mapping
          {dirty && (
            <span className="ml-2 rounded bg-amber-500/15 px-1.5 py-0.5 font-mono text-[9px] uppercase tracking-wider text-amber-700 dark:text-amber-400">
              modified
            </span>
          )}
        </span>
        <div className="flex items-center gap-2">
          <Button size="sm" variant="ghost" onClick={add}>
            <Plus className="mr-1 size-3" />
            Add mapping
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() => void saveIomap(draft)}
            disabled={!dirty}
          >
            <Save className="mr-1.5 size-3" />
            Save
          </Button>
        </div>
      </div>
      <div className="flex-1 overflow-auto p-4">
        {draft.mappings.length === 0 ? (
          <div className="rounded-md border border-dashed border-border p-6 text-center text-xs text-muted-foreground">
            No mappings. Click <span className="font-mono">+ Add mapping</span>{" "}
            to bind a POU variable to a device channel.
          </div>
        ) : (
          <table className="w-full text-sm">
            <thead className="text-[10px] uppercase tracking-wider text-muted-foreground">
              <tr className="border-b border-border">
                <th className="px-2 py-1.5 text-left">Application</th>
                <th className="px-2 py-1.5 text-left">Variable</th>
                <th className="px-2 py-1.5 text-left">Direction</th>
                <th className="px-2 py-1.5 text-left">Device</th>
                <th className="px-2 py-1.5 text-left">Channel</th>
                <th className="px-2 py-1.5"></th>
              </tr>
            </thead>
            <tbody>
              {draft.mappings.map((m, i) => {
                const deviceChannels =
                  project?.devices.find((d) => d.name === m.device)
                    ?.protocol === "modbus"
                    ? null // we don't have channel summaries in tree; free text below
                    : null
                return (
                  <tr key={i} className="border-b border-border last:border-0">
                    <td className="px-2 py-1.5">
                      <Select
                        value={m.application}
                        onValueChange={(v) => set(i, { application: v })}
                      >
                        <SelectTrigger className="h-8 w-44">
                          <SelectValue placeholder="(none)" />
                        </SelectTrigger>
                        <SelectContent>
                          {project?.applications.map((a) => (
                            <SelectItem key={a.name} value={a.name}>
                              {a.name}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </td>
                    <td className="px-2 py-1.5">
                      <Input
                        value={m.variable}
                        onChange={(e) =>
                          set(i, { variable: e.target.value })
                        }
                        className="h-8 w-40"
                        placeholder="counter"
                      />
                    </td>
                    <td className="px-2 py-1.5">
                      <Select
                        value={m.direction}
                        onValueChange={(v) =>
                          set(i, { direction: v as Direction })
                        }
                      >
                        <SelectTrigger className="h-8 w-32">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="input">input (bus → var)</SelectItem>
                          <SelectItem value="output">output (var → bus)</SelectItem>
                        </SelectContent>
                      </Select>
                    </td>
                    <td className="px-2 py-1.5">
                      <Select
                        value={m.device}
                        onValueChange={(v) => set(i, { device: v })}
                      >
                        <SelectTrigger className="h-8 w-40">
                          <SelectValue placeholder="(none)" />
                        </SelectTrigger>
                        <SelectContent>
                          {project?.devices.map((d) => (
                            <SelectItem key={d.name} value={d.name}>
                              {d.name}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </td>
                    <td className="px-2 py-1.5">
                      <Input
                        value={m.channel}
                        onChange={(e) =>
                          set(i, { channel: e.target.value })
                        }
                        className="h-8 w-40"
                        placeholder="coil_0"
                      />
                    </td>
                    <td className="px-2 py-1.5 text-right">
                      <button
                        type="button"
                        onClick={() => remove(i)}
                        className="rounded p-1 text-muted-foreground hover:bg-accent/40 hover:text-red-600"
                        title="Remove"
                      >
                        <Trash2 className="size-3.5" />
                      </button>
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        )}
      </div>
    </main>
  )
}
