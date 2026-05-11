import { RuntimeProvider } from "@/state/runtime"
import { MonitorPane } from "./MonitorPane"
import { ProgramPane } from "./ProgramPane"
import { ProjectPane } from "./ProjectPane"

export function Workbench() {
  return (
    <RuntimeProvider>
      <div className="grid h-screen grid-cols-[280px_1fr_340px] bg-background text-foreground">
        <ProjectPane />
        <ProgramPane />
        <MonitorPane />
      </div>
    </RuntimeProvider>
  )
}
