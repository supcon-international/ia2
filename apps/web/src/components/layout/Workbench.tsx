import { ProjectEmptyState } from "@/components/dialogs/ProjectEmptyState"
import { useDarkMode } from "@/lib/dark-mode"
import { RuntimeProvider, useRuntime } from "@/state/runtime"
import { AgentsPane } from "./AgentsPane"
import { DevicePane } from "./DevicePane"
import { IoMapPane } from "./IoMapPane"
import { MonitorPane } from "./MonitorPane"
import { ProgramPane } from "./ProgramPane"
import { ProjectPane } from "./ProjectPane"

export function Workbench() {
  useDarkMode()
  return (
    <RuntimeProvider>
      <Shell />
    </RuntimeProvider>
  )
}

function Shell() {
  const { project, projectLoading, view } = useRuntime()

  if (projectLoading) {
    return (
      <div className="grid h-screen place-items-center bg-background text-sm text-muted-foreground">
        Loading…
      </div>
    )
  }

  if (!project) {
    return <ProjectEmptyState />
  }

  const center =
    view === "device" ? (
      <DevicePane />
    ) : view === "iomap" ? (
      <IoMapPane />
    ) : (
      <ProgramPane />
    )

  return (
    <div className="grid h-screen grid-cols-[260px_1fr_320px] bg-background text-foreground">
      <ProjectPane />
      <div className="grid min-h-0 min-w-0 grid-rows-[1fr_minmax(180px,32%)] border-x border-border">
        {center}
        <MonitorPane />
      </div>
      <AgentsPane />
    </div>
  )
}
