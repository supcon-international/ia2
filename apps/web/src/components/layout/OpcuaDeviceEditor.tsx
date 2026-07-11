import { Plus, Trash2 } from "lucide-react"

import { Button } from "@/components/ui/button"
import { EnumSelect } from "@/components/ui/enum-select"
import { Field } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { NumberCell } from "@/components/ui/number-cell"
import type { Device } from "@/types/generated/Device"
import type { OpcuaAccess } from "@/types/generated/OpcuaAccess"
import type { OpcuaChannel } from "@/types/generated/OpcuaChannel"
import type { OpcuaDataType } from "@/types/generated/OpcuaDataType"

import {
  DeviceSaveBar,
  EmptyBox,
  LinkedToCell,
  SectionHeader,
  useDeviceDraft,
  type DeviceEditorProps,
} from "./deviceEditorShared"

const OPCUA_DATA_TYPES: { value: OpcuaDataType; label: string }[] = [
  { value: "bool", label: "Boolean" },
  { value: "i16", label: "Int16" },
  { value: "u16", label: "UInt16" },
  { value: "i32", label: "Int32" },
  { value: "u32", label: "UInt32" },
  { value: "f32", label: "Float (f32)" },
  { value: "f64", label: "Double (f64)" },
]

export function OpcuaDeviceEditor({ device, onSave, link }: DeviceEditorProps) {
  const { draft, setDraft, dirty } = useDeviceDraft(device)
  if (draft.protocol !== "opcua") return null

  const update = (patch: Partial<typeof draft>) =>
    setDraft({ ...draft, ...patch } as Device)

  const setChannel = (idx: number, patch: Partial<OpcuaChannel>) => {
    const next = [...draft.channels]
    next[idx] = { ...next[idx], ...patch }
    update({ channels: next })
  }
  const addChannel = () => {
    update({
      channels: [
        ...draft.channels,
        {
          name: `tag_${draft.channels.length}`,
          node_id: "ns=2;s=",
          data_type: "f32",
          access: "read",
          failsafe: null,
        },
      ],
    })
  }
  const removeChannel = (idx: number) => {
    update({ channels: draft.channels.filter((_, i) => i !== idx) })
  }

  const auth = draft.auth ?? { kind: "anonymous" }

  return (
    <>
      <DeviceSaveBar
        name={device.name}
        protocol="opc ua"
        dirty={dirty}
        onSave={() => void onSave(draft)}
      />

      <div className="flex-1 space-y-6 overflow-auto p-5">
        <section>
          <SectionHeader title="Server" />
          <div className="grid grid-cols-2 gap-3 max-w-2xl">
            <Field label="Endpoint URL">
              <Input
                value={draft.endpoint_url}
                onChange={(e) => update({ endpoint_url: e.target.value })}
                placeholder="opc.tcp://10.0.0.10:4840"
                className="font-mono"
              />
            </Field>
            <Field label="Poll interval (ms)">
              <NumberCell
                min={50}
                step={50}
                value={draft.poll_interval_ms}
                onChange={(n) => update({ poll_interval_ms: n })}
              />
            </Field>
            <Field label="Authentication">
              <EnumSelect<"anonymous" | "user_password">
                value={auth.kind}
                onValueChange={(v) =>
                  update({
                    auth:
                      v === "anonymous"
                        ? { kind: "anonymous" }
                        : { kind: "user_password", username: "", password: "" },
                  })
                }
                options={[
                  { value: "anonymous", label: "Anonymous" },
                  { value: "user_password", label: "User / password" },
                ]}
                className="h-9 w-full"
              />
            </Field>
            {auth.kind === "user_password" && (
              <>
                <Field label="Username">
                  <Input
                    value={auth.username}
                    onChange={(e) =>
                      update({ auth: { ...auth, username: e.target.value } })
                    }
                  />
                </Field>
                <Field label="Password">
                  <Input
                    type="password"
                    value={auth.password}
                    onChange={(e) =>
                      update({ auth: { ...auth, password: e.target.value } })
                    }
                  />
                </Field>
              </>
            )}
          </div>
        </section>

        <section>
          <SectionHeader
            title={`Tags (${draft.channels.length})`}
            action={
              <Button size="sm" variant="ghost" onClick={addChannel}>
                <Plus className="mr-1 size-3" />
                Add tag
              </Button>
            }
          />
          {draft.channels.length === 0 ? (
            <EmptyBox>
              No tags. Each tag maps one server NodeId (e.g.{" "}
              <span className="font-mono">ns=2;s=FT0202.PV</span>) to an iomap
              channel.
            </EmptyBox>
          ) : (
            <table className="w-full text-sm">
              <thead className="text-[10px] uppercase tracking-wider text-muted-foreground">
                <tr className="border-b border-border">
                  <th className="px-2 py-1.5 text-left">Name</th>
                  <th className="px-2 py-1.5 text-left">NodeId</th>
                  <th className="px-2 py-1.5 text-left">Type</th>
                  <th className="px-2 py-1.5 text-left">Access</th>
                  <th
                    className="px-2 py-1.5 text-left"
                    title="Optional value written on runtime shutdown/trip. Empty = leave the DCS tag untouched (recommended for a supervisory layer)."
                  >
                    Failsafe
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
                        onChange={(e) => setChannel(i, { name: e.target.value })}
                        className="h-8 w-36"
                      />
                    </td>
                    <td className="px-2 py-1.5">
                      <Input
                        value={ch.node_id}
                        onChange={(e) =>
                          setChannel(i, { node_id: e.target.value })
                        }
                        className="h-8 w-56 font-mono"
                        placeholder="ns=2;s=FT0202.PV"
                      />
                    </td>
                    <td className="px-2 py-1.5">
                      <EnumSelect<OpcuaDataType>
                        value={ch.data_type}
                        onValueChange={(v) => setChannel(i, { data_type: v })}
                        options={OPCUA_DATA_TYPES}
                        className="h-8 w-32"
                      />
                    </td>
                    <td className="px-2 py-1.5">
                      <EnumSelect<OpcuaAccess>
                        value={ch.access ?? "read"}
                        onValueChange={(v) => setChannel(i, { access: v })}
                        options={[
                          { value: "read", label: "read" },
                          { value: "write", label: "write" },
                        ]}
                        className="h-8 w-28"
                      />
                    </td>
                    <td className="px-2 py-1.5">
                      <Input
                        value={ch.failsafe ?? ""}
                        onChange={(e) => {
                          const t = e.target.value.trim()
                          setChannel(i, {
                            failsafe: t === "" ? null : Number(t),
                          })
                        }}
                        className="h-8 w-20"
                        placeholder="—"
                      />
                    </td>
                    <td className="px-2 py-1.5">
                      <LinkedToCell channelName={ch.name} link={link} />
                    </td>
                    <td className="px-2 py-1.5 text-right">
                      <button
                        type="button"
                        onClick={() => removeChannel(i)}
                        className="rounded p-1 text-muted-foreground hover:bg-accent/40 hover:text-destructive"
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
