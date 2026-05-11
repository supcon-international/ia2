export function AgentsPane() {
  return (
    <aside className="flex min-w-0 flex-col">
      <div className="flex h-9 items-center border-b border-border px-3 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        Agents
      </div>
      <div className="flex-1 space-y-3 overflow-auto p-3 text-xs">
        <div className="text-muted-foreground">No AI agent attached.</div>
        <div className="rounded-md border border-dashed border-border p-2 text-[11px] leading-relaxed text-muted-foreground">
          External agents will connect over MCP — Claude Code, Codex, and
          Cursor as clients. The MCP server endpoint isn't wired up yet.
        </div>
      </div>
    </aside>
  )
}
