import { ProjectPane } from "./ProjectPane"
import { ProgramPane } from "./ProgramPane"
import { ConnectionsPane } from "./ConnectionsPane"

export function Workbench() {
  return (
    <div className="grid h-screen grid-cols-[280px_1fr_320px] bg-background text-foreground">
      <ProjectPane />
      <ProgramPane />
      <ConnectionsPane />
    </div>
  )
}
