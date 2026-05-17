import { FolderPlus, FolderOpen, Box } from "lucide-react"

import { Button } from "@/components/ui/button"
import { useRuntime } from "@/state/runtime"
import { NewProjectDialog } from "./NewProjectDialog"
import { OpenProjectDialog } from "./OpenProjectDialog"

export function ProjectEmptyState() {
  const { availableProjects, openProject } = useRuntime()

  return (
    <div className="flex h-screen flex-col text-foreground">
      {/* macOS titlebar drag-region — transparent so vibrancy shows
       * through behind the traffic lights. See Workbench for the full
       * explanation. */}
      <div aria-hidden className="ia2-mac-drag-region h-7 shrink-0" />
      <div className="ia2-no-drag grid flex-1 place-items-center bg-background">
      <div className="w-full max-w-md space-y-6 p-8">
        <div className="space-y-1">
          <div className="flex items-center gap-2 text-xs uppercase tracking-widest text-muted-foreground">
            <Box className="size-4" />
            IA2
          </div>
          <h1 className="text-2xl font-semibold tracking-tight">
            No project open
          </h1>
          <p className="text-sm text-muted-foreground">
            Create a new project or open one to start writing IEC 61131-3
            programs and wiring devices.
          </p>
        </div>

        <div className="flex gap-2">
          <NewProjectDialog
            trigger={
              <Button>
                <FolderPlus className="mr-2 size-4" />
                New project
              </Button>
            }
          />
          <OpenProjectDialog
            trigger={
              <Button variant="outline">
                <FolderOpen className="mr-2 size-4" />
                Open project
              </Button>
            }
          />
        </div>

        {availableProjects.length > 0 && (
          <div className="space-y-2">
            <div className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
              Recent
            </div>
            <ul className="divide-y divide-border rounded-md border border-border">
              {availableProjects.map((p) => (
                <li key={p.path}>
                  <button
                    type="button"
                    onClick={() => openProject(p.path)}
                    className="flex w-full items-center gap-3 px-3 py-2 text-left text-sm hover:bg-accent/40"
                  >
                    <FolderOpen className="size-4 shrink-0 text-muted-foreground" />
                    <span className="flex-1 truncate font-medium">
                      {p.name}
                    </span>
                    <span className="truncate font-mono text-[10px] text-muted-foreground">
                      {p.path.replace(/^.*\//, "")}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>
      </div>
    </div>
  )
}
