import { ChevronDown } from "lucide-react"

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { useRuntime } from "@/state/runtime"
import { ProjectTree } from "./ProjectTree"
import { SystemIndication } from "./SystemIndication"
import { NewProjectDialog } from "@/components/dialogs/NewProjectDialog"
import { OpenProjectDialog } from "@/components/dialogs/OpenProjectDialog"

export function ProjectPane() {
  const { project, closeProject } = useRuntime()

  return (
    <aside className="flex h-full min-w-0 flex-col">
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            className="flex h-9 items-center justify-between gap-1 border-b border-border px-3 text-left text-[11px] font-medium uppercase tracking-wider text-muted-foreground hover:bg-accent/40 hover:text-foreground"
          >
            <span className="truncate normal-case tracking-normal text-foreground">
              {project?.name ?? "Project"}
            </span>
            <ChevronDown className="size-3 shrink-0" />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-56">
          <NewProjectDialog
            trigger={
              <DropdownMenuItem onSelect={(e) => e.preventDefault()}>
                New project…
              </DropdownMenuItem>
            }
          />
          <OpenProjectDialog
            trigger={
              <DropdownMenuItem onSelect={(e) => e.preventDefault()}>
                Open project…
              </DropdownMenuItem>
            }
          />
          {project && (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuItem
                onSelect={() => {
                  void closeProject()
                }}
              >
                Close project
              </DropdownMenuItem>
            </>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
      <div className="flex-1 overflow-auto">
        <ProjectTree />
      </div>
      <SystemIndication />
    </aside>
  )
}
