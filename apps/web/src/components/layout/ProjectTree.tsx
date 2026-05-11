import { useState } from "react"
import {
  ChevronDown,
  ChevronRight,
  Cpu,
  FileCode2,
  Folder,
  Gauge,
  ToggleRight,
} from "lucide-react"
import { cn } from "@/lib/utils"

type Kind = "folder" | "st" | "fb" | "device" | "valve" | "sensor"

type TreeNode = {
  id: string
  label: string
  kind: Kind
  children?: TreeNode[]
}

const SAMPLE: TreeNode = {
  id: "project",
  label: "Project",
  kind: "folder",
  children: [
    {
      id: "app",
      label: "Application",
      kind: "folder",
      children: [
        { id: "valve_logic", label: "Valve_logic (ST)", kind: "st" },
        { id: "tank_logic", label: "Tank_logic (FB)", kind: "fb" },
      ],
    },
    {
      id: "device",
      label: "Device",
      kind: "folder",
      children: [
        {
          id: "tank",
          label: "Tank",
          kind: "device",
          children: [
            { id: "valve1", label: "Valve1", kind: "valve" },
            { id: "ti-001", label: "TI-001", kind: "sensor" },
            { id: "pi-001", label: "PI-001", kind: "sensor" },
          ],
        },
      ],
    },
  ],
}

function NodeIcon({ kind }: { kind: Kind }) {
  const c = "size-3.5 shrink-0"
  switch (kind) {
    case "folder":
      return <Folder className={cn(c, "text-muted-foreground")} />
    case "st":
    case "fb":
      return <FileCode2 className={cn(c, "text-sky-600 dark:text-sky-400")} />
    case "device":
      return <Cpu className={cn(c, "text-amber-600 dark:text-amber-400")} />
    case "valve":
      return <ToggleRight className={cn(c, "text-emerald-600 dark:text-emerald-400")} />
    case "sensor":
      return <Gauge className={cn(c, "text-violet-600 dark:text-violet-400")} />
  }
}

function Node({ node, depth }: { node: TreeNode; depth: number }) {
  const [open, setOpen] = useState(true)
  const has = !!node.children?.length
  return (
    <>
      <button
        type="button"
        onClick={() => has && setOpen(!open)}
        className="flex w-full items-center gap-1.5 px-2 py-1 text-left text-sm transition-colors hover:bg-accent/40"
        style={{ paddingLeft: 8 + depth * 14 }}
      >
        {has ? (
          open ? (
            <ChevronDown className="size-3 shrink-0 text-muted-foreground" />
          ) : (
            <ChevronRight className="size-3 shrink-0 text-muted-foreground" />
          )
        ) : (
          <span className="inline-block size-3 shrink-0" />
        )}
        <NodeIcon kind={node.kind} />
        <span className="truncate">{node.label}</span>
      </button>
      {open &&
        node.children?.map((c) => <Node key={c.id} node={c} depth={depth + 1} />)}
    </>
  )
}

export function ProjectTree() {
  return (
    <div className="py-2">
      <Node node={SAMPLE} depth={0} />
    </div>
  )
}
