import { ProjectTree } from "./ProjectTree"
import { SystemIndication } from "./SystemIndication"

export function ProjectPane() {
  return (
    <aside className="flex min-w-0 flex-col">
      <div className="flex h-9 items-center border-b border-border px-3 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        Project
      </div>
      <div className="flex-1 overflow-auto">
        <ProjectTree />
      </div>
      <SystemIndication />
    </aside>
  )
}
