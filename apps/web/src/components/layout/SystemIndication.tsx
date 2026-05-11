export function SystemIndication() {
  return (
    <div className="border-t border-border px-3 py-2">
      <div className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
        System indication
      </div>
      <div className="mt-1 flex items-center gap-2 text-xs">
        <span className="size-2 rounded-full bg-muted-foreground/40" />
        <span className="text-muted-foreground">Runtime not connected</span>
      </div>
    </div>
  )
}
