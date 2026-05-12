import { useMemo, useState } from "react"
import {
  Cable,
  Clock,
  ChevronDown,
  ChevronRight,
  Cpu,
  FileCode2,
  Folder,
  FolderOpen,
  FolderPlus,
  Network,
  Plus,
  Server,
} from "lucide-react"

import { NewDeviceDialog } from "@/components/dialogs/NewDeviceDialog"
import { NewEdgeDialog } from "@/components/dialogs/NewEdgeDialog"
import { NewFolderDialog } from "@/components/dialogs/NewFolderDialog"
import { NewPouDialog } from "@/components/dialogs/NewPouDialog"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
import { cn } from "@/lib/utils"
import { buildTree, type TreeNode } from "@/lib/project-tree"
import { useRuntime } from "@/state/runtime"
import type { PouType } from "@/types/generated/PouType"
import type { PouFile } from "@/types/generated/PouFile"
import type { Device } from "@/types/generated/Device"
import type { Edge } from "@/types/generated/Edge"
import type { Protocol } from "@/types/generated/Protocol"

export function ProjectTree() {
  const {
    project,
    view,
    currentPou,
    currentDevice,
    currentEdge,
    attached,
    selectPou,
    selectDevice,
    selectEdge,
    openIoMap,
    openTasks,
    deletePou,
    deleteDevice,
    deleteEdge,
  } = useRuntime()

  // Section open/close. Persists across re-renders so toggling a sibling
  // folder doesn't collapse the section.
  const [appsOpen, setAppsOpen] = useState(true)
  const [devicesOpen, setDevicesOpen] = useState(true)
  const [edgesOpen, setEdgesOpen] = useState(true)

  // Folder open-state keyed by `section/path` so each section is independent.
  const [openFolders, setOpenFolders] = useState<Record<string, boolean>>({})
  const toggleFolder = (key: string) =>
    setOpenFolders((prev) => ({ ...prev, [key]: !prev[key] }))

  // Controlled NewFolder / NewPou / NewDevice dialogs — opened from the
  // folder context menu with a known parent path.
  const [folderDialog, setFolderDialog] = useState<{
    section: "applications" | "devices" | "edges"
    parent: string
  } | null>(null)
  const [pouDialog, setPouDialog] = useState<{ parent: string } | null>(null)
  const [deviceDialog, setDeviceDialog] = useState<{ parent: string } | null>(
    null,
  )
  const [edgeDialog, setEdgeDialog] = useState<{ parent: string } | null>(null)

  const pouTree = useMemo(
    () =>
      project
        ? buildTree<PouFile>(
            project.pous,
            project.pou_folders,
            (p) => p.path,
          )
        : [],
    [project],
  )

  const deviceTree = useMemo(
    () =>
      project
        ? buildTree<Device>(project.devices, project.device_folders, (d) => d.name)
        : [],
    [project],
  )

  const edgeTree = useMemo(
    () =>
      project
        ? buildTree<Edge>(project.edges, project.edge_folders, (e) => e.name)
        : [],
    [project],
  )

  if (!project) return null

  return (
    <div className="py-1 text-sm">
      <SectionHeader
        label="Applications"
        open={appsOpen}
        count={project.pous.length}
        onToggle={() => setAppsOpen(!appsOpen)}
        action={
          <SectionActions
            onAddItem={() => setPouDialog({ parent: "" })}
            onAddFolder={() =>
              setFolderDialog({ section: "applications", parent: "" })
            }
            itemTitle="New POU"
            folderTitle="New folder"
          />
        }
      />
      {appsOpen && (
        <TreeChildren
          nodes={pouTree}
          depth={0}
          renderItem={(node) => (
            <PouItem
              node={node}
              active={view === "app" && currentPou?.path === node.path}
              onOpen={() => selectPou(node.path)}
              onDelete={() => {
                if (confirm(`Delete POU file "${node.path}.st"?`)) {
                  void deletePou(node.path)
                }
              }}
            />
          )}
          folderContextMenu={(folder) => (
            <>
              <ContextMenuItem
                onSelect={() => setPouDialog({ parent: folder.path })}
              >
                New POU here…
              </ContextMenuItem>
              <ContextMenuItem
                onSelect={() =>
                  setFolderDialog({
                    section: "applications",
                    parent: folder.path,
                  })
                }
              >
                New folder here…
              </ContextMenuItem>
            </>
          )}
          openFolders={openFolders}
          toggleFolder={toggleFolder}
          section="apps"
          emptyHint={
            project.pous.length === 0 &&
            project.pou_folders.length === 0
              ? "No POUs yet — click + to create one."
              : null
          }
        />
      )}

      <SectionHeader
        label="Devices"
        open={devicesOpen}
        count={project.devices.length}
        onToggle={() => setDevicesOpen(!devicesOpen)}
        action={
          <SectionActions
            onAddItem={() => setDeviceDialog({ parent: "" })}
            onAddFolder={() =>
              setFolderDialog({ section: "devices", parent: "" })
            }
            itemTitle="New device"
            folderTitle="New folder"
          />
        }
      />
      {devicesOpen && (
        <TreeChildren
          nodes={deviceTree}
          depth={0}
          renderItem={(node) => (
            <DeviceItem
              node={node}
              active={
                view === "device" && currentDevice?.name === node.path
              }
              onOpen={() => selectDevice(node.path)}
              onDelete={() => {
                if (confirm(`Delete device "${node.path}"?`)) {
                  void deleteDevice(node.path)
                }
              }}
            />
          )}
          folderContextMenu={(folder) => (
            <>
              <ContextMenuItem
                onSelect={() => setDeviceDialog({ parent: folder.path })}
              >
                New device here…
              </ContextMenuItem>
              <ContextMenuItem
                onSelect={() =>
                  setFolderDialog({
                    section: "devices",
                    parent: folder.path,
                  })
                }
              >
                New folder here…
              </ContextMenuItem>
            </>
          )}
          openFolders={openFolders}
          toggleFolder={toggleFolder}
          section="devices"
          emptyHint={
            project.devices.length === 0 && project.device_folders.length === 0
              ? "No devices yet — click + to add one."
              : null
          }
        />
      )}

      <SectionHeader
        label="Edges"
        open={edgesOpen}
        count={project.edges.length}
        onToggle={() => setEdgesOpen(!edgesOpen)}
        action={
          <SectionActions
            onAddItem={() => setEdgeDialog({ parent: "" })}
            onAddFolder={() =>
              setFolderDialog({ section: "edges", parent: "" })
            }
            itemTitle="New edge"
            folderTitle="New folder"
          />
        }
      />
      {edgesOpen && (
        <TreeChildren
          nodes={edgeTree}
          depth={0}
          renderItem={(node) => (
            <EdgeItem
              node={node}
              active={view === "edge" && currentEdge?.name === node.path}
              attached={attached?.name === node.path}
              onOpen={() => selectEdge(node.path)}
              onDelete={() => {
                if (confirm(`Delete edge "${node.path}"?`)) {
                  void deleteEdge(node.path)
                }
              }}
            />
          )}
          folderContextMenu={(folder) => (
            <>
              <ContextMenuItem
                onSelect={() => setEdgeDialog({ parent: folder.path })}
              >
                New edge here…
              </ContextMenuItem>
              <ContextMenuItem
                onSelect={() =>
                  setFolderDialog({ section: "edges", parent: folder.path })
                }
              >
                New folder here…
              </ContextMenuItem>
            </>
          )}
          openFolders={openFolders}
          toggleFolder={toggleFolder}
          section="edges"
          emptyHint={
            project.edges.length === 0 && project.edge_folders.length === 0
              ? "No edges yet — click + to add a deploy target."
              : null
          }
        />
      )}

      <button
        type="button"
        onClick={() => void openTasks()}
        className={cn(
          "mt-1 flex w-full items-center gap-1.5 border-t border-border px-2 py-1.5 text-left text-sm transition-colors hover:bg-accent/40",
          view === "tasks" && "bg-accent/60",
        )}
        style={{ paddingLeft: 12 }}
      >
        <Clock className="size-3.5 shrink-0 text-violet-600 dark:text-violet-400" />
        <span className="flex-1 truncate font-medium">Tasks</span>
        <span className="font-mono text-[9px] uppercase text-muted-foreground">
          {project.tasks.tasks.length}/{project.tasks.programs.length}
        </span>
      </button>

      <button
        type="button"
        onClick={() => void openIoMap()}
        className={cn(
          "flex w-full items-center gap-1.5 px-2 py-1.5 text-left text-sm transition-colors hover:bg-accent/40",
          view === "iomap" && "bg-accent/60",
        )}
        style={{ paddingLeft: 12 }}
      >
        <Cable className="size-3.5 shrink-0 text-fuchsia-600 dark:text-fuchsia-400" />
        <span className="flex-1 truncate font-medium">IO Mapping</span>
        <span className="font-mono text-[9px] uppercase text-muted-foreground">
          {project.iomap.mappings.length}
        </span>
      </button>

      {/* Controlled dialogs popped open by folder context menus. */}
      {folderDialog && (
        <NewFolderDialog
          open
          onOpenChange={(o) => {
            if (!o) setFolderDialog(null)
          }}
          section={folderDialog.section}
          parent={folderDialog.parent}
        />
      )}
      {pouDialog && (
        <NewPouDialog
          open
          onOpenChange={(o) => {
            if (!o) setPouDialog(null)
          }}
          parent={pouDialog.parent}
        />
      )}
      {deviceDialog && (
        <NewDeviceDialog
          open
          onOpenChange={(o) => {
            if (!o) setDeviceDialog(null)
          }}
          parent={deviceDialog.parent}
        />
      )}
      {edgeDialog && (
        <NewEdgeDialog
          open
          onOpenChange={(o) => {
            if (!o) setEdgeDialog(null)
          }}
          parent={edgeDialog.parent}
        />
      )}
    </div>
  )
}

// ============================================================
//  Tree internals
// ============================================================

function TreeChildren<T>({
  nodes,
  depth,
  renderItem,
  folderContextMenu,
  openFolders,
  toggleFolder,
  section,
  emptyHint,
}: {
  nodes: TreeNode<T>[]
  depth: number
  renderItem: (node: Extract<TreeNode<T>, { kind: "item" }>) => React.ReactNode
  folderContextMenu: (
    folder: Extract<TreeNode<T>, { kind: "folder" }>,
  ) => React.ReactNode
  openFolders: Record<string, boolean>
  toggleFolder: (key: string) => void
  section: string
  emptyHint?: string | null
}) {
  if (nodes.length === 0) {
    return emptyHint ? (
      <div
        className="py-1 text-[11px] italic text-muted-foreground"
        style={{ paddingLeft: pad(depth + 1) }}
      >
        {emptyHint}
      </div>
    ) : null
  }

  return (
    <>
      {nodes.map((node) => {
        if (node.kind === "folder") {
          const key = `${section}/${node.path}`
          const isOpen = openFolders[key] ?? true
          return (
            <div key={key}>
              <ContextMenu>
                <ContextMenuTrigger asChild>
                  <button
                    type="button"
                    onClick={() => toggleFolder(key)}
                    className="flex w-full items-center gap-1 py-1 text-left text-sm text-muted-foreground transition-colors hover:bg-accent/40 hover:text-foreground"
                    style={{ paddingLeft: pad(depth) }}
                  >
                    {isOpen ? (
                      <ChevronDown className="size-3 shrink-0" />
                    ) : (
                      <ChevronRight className="size-3 shrink-0" />
                    )}
                    {isOpen ? (
                      <FolderOpen className="size-3.5 shrink-0 text-amber-600 dark:text-amber-400" />
                    ) : (
                      <Folder className="size-3.5 shrink-0 text-amber-600 dark:text-amber-400" />
                    )}
                    <span className="flex-1 truncate text-foreground">
                      {node.name}
                    </span>
                  </button>
                </ContextMenuTrigger>
                <ContextMenuContent>
                  {folderContextMenu(node)}
                </ContextMenuContent>
              </ContextMenu>
              {isOpen && (
                <TreeChildren
                  nodes={node.children}
                  depth={depth + 1}
                  renderItem={renderItem}
                  folderContextMenu={folderContextMenu}
                  openFolders={openFolders}
                  toggleFolder={toggleFolder}
                  section={section}
                />
              )}
            </div>
          )
        }
        return (
          <div
            key={`${section}-item-${node.path}`}
            style={{ paddingLeft: pad(depth) }}
          >
            {renderItem(node)}
          </div>
        )
      })}
    </>
  )
}

/** Pixel padding for a given tree depth — keeps icons aligned across levels. */
function pad(depth: number): number {
  return 12 + depth * 14
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

function SectionActions({
  onAddItem,
  onAddFolder,
  itemTitle,
  folderTitle,
}: {
  onAddItem: () => void
  onAddFolder: () => void
  itemTitle: string
  folderTitle: string
}) {
  return (
    <div className="flex items-center">
      <button
        type="button"
        title={folderTitle}
        onClick={(e) => {
          e.stopPropagation()
          onAddFolder()
        }}
        className="flex size-5 items-center justify-center rounded text-muted-foreground hover:bg-accent/40 hover:text-foreground"
      >
        <FolderPlus className="size-3.5" />
      </button>
      <button
        type="button"
        title={itemTitle}
        onClick={(e) => {
          e.stopPropagation()
          onAddItem()
        }}
        className="flex size-5 items-center justify-center rounded text-muted-foreground hover:bg-accent/40 hover:text-foreground"
      >
        <Plus className="size-3.5" />
      </button>
    </div>
  )
}

/// A POU file in the tree. Renders as one row if the file declares
/// exactly one POU and that POU's IEC name matches the file's leaf
/// segment (the typical 1-POU-per-file case); otherwise renders the
/// file as a header row plus one child per declaration. The icon
/// reflects the IEC POU type (PRG / FB / FUN), and a small badge
/// shows the language ("st" for now). All variants click-to-open the
/// same file in the editor.
function PouItem({
  node,
  active,
  onOpen,
  onDelete,
}: {
  node: { name: string; path: string; item: PouFile }
  active: boolean
  onOpen: () => void
  onDelete: () => void
}) {
  const decls = node.item.declarations
  const simple =
    decls.length === 1 && decls[0].name === node.name
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div>
          <button
            type="button"
            onClick={onOpen}
            className={cn(
              "flex w-full items-center gap-1.5 py-1 pl-3 pr-2 text-left transition-colors hover:bg-accent/40",
              active && "bg-accent/60",
            )}
          >
            {simple ? (
              <PouTypeIcon type={decls[0].type} />
            ) : (
              <FileCode2 className="size-3.5 shrink-0 text-muted-foreground" />
            )}
            <span className="flex-1 truncate">{node.name}</span>
            {simple ? (
              <PouTypeBadge type={decls[0].type} language={decls[0].language} />
            ) : decls.length === 0 ? (
              <span className="font-mono text-[9px] uppercase text-amber-700 dark:text-amber-400">
                empty
              </span>
            ) : (
              <span className="font-mono text-[9px] uppercase text-muted-foreground">
                {decls.length} POUs
              </span>
            )}
          </button>
          {/* Multi-POU file: each declaration as a sibling sub-row.
              Indented so it's obvious they live inside the file above. */}
          {!simple && decls.length > 0 && (
            <ul>
              {decls.map((d) => (
                <li key={d.name}>
                  <button
                    type="button"
                    onClick={onOpen}
                    className="flex w-full items-center gap-1.5 py-0.5 pl-9 pr-2 text-left text-[12px] text-muted-foreground transition-colors hover:bg-accent/40 hover:text-foreground"
                  >
                    <PouTypeIcon type={d.type} />
                    <span className="flex-1 truncate">{d.name}</span>
                    <PouTypeBadge type={d.type} language={d.language} />
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem onSelect={onOpen}>Open</ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem variant="destructive" onSelect={onDelete}>
          Delete
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  )
}

function PouTypeIcon({ type }: { type: PouType }) {
  return (
    <FileCode2
      className={cn(
        "size-3.5 shrink-0",
        type === "function_block"
          ? "text-violet-600 dark:text-violet-400"
          : type === "function"
            ? "text-amber-600 dark:text-amber-400"
            : "text-sky-600 dark:text-sky-400",
      )}
    />
  )
}

function PouTypeBadge({
  type,
  language,
}: {
  type: PouType
  language: string
}) {
  const label =
    type === "function_block" ? "fb" : type === "function" ? "fn" : "prg"
  return (
    <span className="flex items-center gap-1 font-mono text-[9px] uppercase tracking-wider text-muted-foreground">
      <span>{label}</span>
      <span className="rounded bg-muted/60 px-1 text-[8px] uppercase">
        {language}
      </span>
    </span>
  )
}

function DeviceItem({
  node,
  active,
  onOpen,
  onDelete,
}: {
  node: { name: string; path: string; item: Device }
  active: boolean
  onOpen: () => void
  onDelete: () => void
}) {
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <button
          type="button"
          onClick={onOpen}
          className={cn(
            "flex w-full items-center gap-1.5 py-1 pl-3 pr-2 text-left transition-colors hover:bg-accent/40",
            active && "bg-accent/60",
          )}
        >
          <ProtocolIcon protocol={node.item.protocol} />
          <span className="flex-1 truncate">{node.name}</span>
          <span className="font-mono text-[9px] uppercase text-muted-foreground">
            {node.item.protocol}
          </span>
        </button>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem onSelect={onOpen}>Open</ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem variant="destructive" onSelect={onDelete}>
          Delete
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  )
}

function EdgeItem({
  node,
  active,
  attached,
  onOpen,
  onDelete,
}: {
  node: { name: string; path: string; item: Edge }
  active: boolean
  attached: boolean
  onOpen: () => void
  onDelete: () => void
}) {
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <button
          type="button"
          onClick={onOpen}
          className={cn(
            "flex w-full items-center gap-1.5 py-1 pl-3 pr-2 text-left transition-colors hover:bg-accent/40",
            active && "bg-accent/60",
          )}
        >
          <Server className="size-3.5 shrink-0 text-rose-600 dark:text-rose-400" />
          <span className="flex-1 truncate">{node.name}</span>
          {attached && (
            <span
              className="font-mono text-[9px] uppercase tracking-wider text-emerald-700 dark:text-emerald-400"
              title="IDE is attached to this edge"
            >
              attached
            </span>
          )}
          <span className="truncate font-mono text-[9px] lowercase text-muted-foreground">
            {node.item.host}
          </span>
        </button>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem onSelect={onOpen}>Open</ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem variant="destructive" onSelect={onDelete}>
          Delete
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  )
}

function ProtocolIcon({ protocol }: { protocol: Protocol }) {
  if (protocol === "modbus") {
    return (
      <Network className="size-3.5 shrink-0 text-amber-600 dark:text-amber-400" />
    )
  }
  return (
    <Cpu className="size-3.5 shrink-0 text-emerald-600 dark:text-emerald-400" />
  )
}

