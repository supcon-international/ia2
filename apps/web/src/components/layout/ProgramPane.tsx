export function ProgramPane() {
  return (
    <main className="flex min-w-0 flex-col border-r border-border">
      <div className="flex h-9 items-center border-b border-border px-3 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        Program
      </div>
      <div className="grid flex-1 place-items-center text-sm text-muted-foreground">
        Select a POU from the project tree
      </div>
    </main>
  )
}
