export function ConnectionsPane() {
  return (
    <aside className="flex min-w-0 flex-col">
      <div className="flex h-9 items-center border-b border-border px-3 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        Connections
      </div>
      <div className="flex-1 space-y-3 p-3 text-xs">
        <div className="text-muted-foreground">No MCP clients connected.</div>
        <div className="rounded-md border border-dashed border-border p-2 text-[11px] leading-relaxed text-muted-foreground">
          Point Claude Code / Codex / Cursor at this workspace's MCP endpoint
          to drive project, compile, and runtime operations from an external agent.
        </div>
      </div>
    </aside>
  )
}
