import { useRuntime } from "@/state/runtime"
import { cn } from "@/lib/utils"

export function SystemIndication() {
  const { connected, isRunning } = useRuntime()

  const { label, dot, text } = connected
    ? isRunning
      ? {
          label: "Running",
          dot: "bg-emerald-500",
          text: "text-foreground",
        }
      : { label: "Idle", dot: "bg-sky-500", text: "text-muted-foreground" }
    : {
        label: "Server unreachable",
        dot: "bg-muted-foreground/40",
        text: "text-muted-foreground",
      }

  return (
    <div className="border-t border-border px-3 py-2">
      <div className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
        System indication
      </div>
      <div className="mt-1 flex items-center gap-2 text-xs">
        <span className={cn("size-2 rounded-full", dot)} />
        <span className={text}>{label}</span>
      </div>
    </div>
  )
}
