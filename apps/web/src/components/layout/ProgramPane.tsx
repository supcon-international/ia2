import { PanelRight, Play, Plus, RotateCcw, Save, Square } from "lucide-react"
import { useState } from "react"

import { LDEditor } from "@/components/editor/LDEditor"
import { STEditor } from "@/components/editor/STEditor"
import { cn } from "@/lib/utils"
import { useRuntime } from "@/state/runtime"
import { VariablesPanel } from "./VariablesPanel"

export function ProgramPane() {
  const {
    currentPou,
    source,
    setSource,
    isDirty,
    saveCurrentPou,
    isRunning,
    run,
    stop,
    diagnostics,
    error,
    tasks,
    saveTasks,
  } = useRuntime()

  // Right-side Variables panel — defaults open so users discover the
  // binding picker without hunting. Persists across POU switches but not
  // across reloads (keeping the state ephemeral keeps the toolbar simple).
  const [varsOpen, setVarsOpen] = useState(true)

  if (!currentPou) {
    return (
      <main className="flex h-full min-h-0 min-w-0 flex-col">
        <div className="flex h-9 items-center border-b border-border px-3 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
          Program
        </div>
        <div className="grid flex-1 place-items-center p-6 text-center text-sm text-muted-foreground">
          Select a POU from the project tree, or create one with the&nbsp;
          <span className="font-mono">+</span> button next to "Applications".
        </div>
      </main>
    )
  }

  return (
    <main className="flex h-full min-h-0 min-w-0 flex-col">
      <div className="flex h-9 items-center justify-between border-b border-border pl-3 pr-2 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        <span className="flex items-center gap-2 truncate normal-case tracking-normal text-foreground">
          <span className="truncate font-mono">{currentPou.path}</span>
          {/* Declaration summary: 1-POU files show their type, multi-POU
              files show a count. The actual icon is in the tree; this is
              just a hint above the editor. */}
          {currentPou.declarations.length === 1 ? (
            <span className="font-mono text-[9px] uppercase tracking-wider text-muted-foreground">
              {currentPou.declarations[0].type === "function_block"
                ? "fb"
                : currentPou.declarations[0].type === "function"
                  ? "fn"
                  : "prg"}
            </span>
          ) : currentPou.declarations.length > 1 ? (
            <span className="font-mono text-[9px] uppercase tracking-wider text-muted-foreground">
              {currentPou.declarations.length} POUs
            </span>
          ) : null}
          {isDirty && (
            <span className="rounded bg-amber-500/15 px-1.5 py-0.5 font-mono text-[9px] uppercase tracking-wider text-amber-700 dark:text-amber-400">
              modified
            </span>
          )}
          {diagnostics.length > 0 && (
            <span className="rounded bg-red-500/15 px-1.5 py-0.5 font-mono text-[9px] uppercase tracking-wider text-red-700 dark:text-red-400">
              {diagnostics.length}{" "}
              {diagnostics.length === 1 ? "issue" : "issues"}
            </span>
          )}
          <ScheduleHint
            currentPou={currentPou}
            tasks={tasks}
            onSchedule={async (programName) => {
              // Append to the first existing task, or create a default
              // `plc_task` (100 ms / priority 1) if none exists.
              const taskName = tasks.tasks[0]?.name ?? "plc_task"
              const nextTasks =
                tasks.tasks.length === 0
                  ? [
                      ...tasks.tasks,
                      { name: taskName, interval_ms: 100, priority: 1 },
                    ]
                  : tasks.tasks
              // Pick an instance name that doesn't collide.
              const taken = new Set(tasks.programs.map((p) => p.instance))
              let cand = `${programName}_inst`
              let n = 1
              while (taken.has(cand)) cand = `${programName}_inst_${n++}`
              await saveTasks({
                tasks: nextTasks,
                programs: [
                  ...tasks.programs,
                  { instance: cand, program: programName, task: taskName },
                ],
              })
            }}
          />
        </span>
        <div className="flex items-center gap-1">
          {isDirty && (
            <>
              <button
                type="button"
                onClick={() => setSource(currentPou.source)}
                title="Revert"
                className="flex items-center gap-1 rounded-md px-2 py-1 text-xs font-medium tracking-normal text-muted-foreground normal-case hover:bg-accent/40"
              >
                <RotateCcw className="size-3" />
                Revert
              </button>
              <button
                type="button"
                onClick={() => void saveCurrentPou()}
                title="Save (Cmd/Ctrl+S)"
                className="flex items-center gap-1 rounded-md px-2 py-1 text-xs font-medium tracking-normal text-foreground normal-case hover:bg-accent/40"
              >
                <Save className="size-3" />
                Save
              </button>
            </>
          )}
          {isRunning ? (
            <button
              type="button"
              onClick={stop}
              className="flex items-center gap-1 rounded-md px-2 py-1 text-xs font-medium tracking-normal text-destructive normal-case hover:bg-destructive/10"
            >
              <Square className="size-3 fill-current" />
              Stop
            </button>
          ) : (
            (() => {
              // Run this file's PROGRAM ad-hoc (not whatever's in
              // tasks.toml). The first declared PROGRAM in the file wins
              // — typical case is one PROGRAM per file. If the file has
              // none (FB-only or Function-only), Run is disabled with a
              // tooltip steering the user to the Tasks pane.
              const target = currentPou.declarations.find(
                (d) => d.type === "program",
              )
              return target ? (
                <button
                  type="button"
                  onClick={() => void run(target.name, currentPou.path)}
                  title={`Compile and run PROGRAM ${target.name} in isolation (just this file's source — Monitor shows only its variables)`}
                  className="flex items-center gap-1 rounded-md px-2 py-1 text-xs font-medium tracking-normal text-highlight normal-case hover:bg-highlight/10"
                >
                  <Play className="size-3 fill-current" />
                  Run {target.name}
                </button>
              ) : (
                <button
                  type="button"
                  disabled
                  title="No PROGRAM in this file to run. Use the Tasks pane to run the whole project."
                  className="flex cursor-not-allowed items-center gap-1 rounded-md px-2 py-1 text-xs font-medium tracking-normal text-muted-foreground/50 normal-case"
                >
                  <Play className="size-3 fill-current" />
                  Run
                </button>
              )
            })()
          )}
          <button
            type="button"
            onClick={() => setVarsOpen((v) => !v)}
            title={varsOpen ? "Hide Variables panel" : "Show Variables panel"}
            className={cn(
              "flex items-center gap-1 rounded-md px-2 py-1 text-xs font-medium tracking-normal normal-case hover:bg-accent/40",
              varsOpen ? "text-foreground" : "text-muted-foreground",
            )}
          >
            <PanelRight className="size-3.5" />
          </button>
        </div>
      </div>
      {error && (
        <div className="border-b border-red-200 bg-red-50 px-3 py-2 text-xs text-red-700 dark:border-red-900 dark:bg-red-950/40 dark:text-red-400">
          {error}
        </div>
      )}
      <div className="flex min-h-0 flex-1">
        <div className="min-h-0 min-w-0 flex-1">
          {/* Editor dispatch by POU language. ST keeps the Monaco
              editor; LD renders a read-only SVG ladder (authoring
              still goes through the JSON, via the in-editor fallback
              that LDEditor surfaces when the source can't parse —
              and via direct file edits / agent writes for now). */}
          {currentPou.declarations[0]?.language === "ld" ? (
            <LDEditor value={source} onChange={setSource} />
          ) : (
            <STEditor
              value={source}
              onChange={setSource}
              diagnostics={diagnostics}
            />
          )}
        </div>
        {varsOpen && (
          <div className="hidden min-h-0 w-[240px] shrink-0 md:block">
            <VariablesPanel />
          </div>
        )}
      </div>
    </main>
  )
}

import type { Pou } from "@/types/generated/Pou"
import type { Tasks } from "@/types/generated/Tasks"

/**
 * Header-area indicator that tells the user, before they click Run, what
 * will actually run:
 *
 *  - 0 PROGRAMs declared in this file → no schedule action; just a hint.
 *  - 1+ PROGRAMs, all already scheduled → a small ✓ "Scheduled" badge.
 *  - 1+ PROGRAMs, some unscheduled → a "Schedule" button that one-clicks
 *    the first unscheduled PROGRAM into the project's first task.
 *
 * Run is project-level (it compiles every PROGRAM instance in tasks.toml
 * — `currentPou` doesn't affect what runs), so without this surface the
 * user is otherwise blind to whether their file is even hooked up.
 */
function ScheduleHint({
  currentPou,
  tasks,
  onSchedule,
}: {
  currentPou: Pou
  tasks: Tasks
  onSchedule: (programName: string) => Promise<void>
}) {
  const programs = currentPou.declarations.filter((d) => d.type === "program")
  if (programs.length === 0) return null
  const scheduled = new Set(tasks.programs.map((p) => p.program))
  const unscheduled = programs.filter((p) => !scheduled.has(p.name))
  if (unscheduled.length === 0) {
    return (
      <span
        className="inline-flex items-center gap-1 rounded bg-highlight/15 px-1.5 py-0.5 font-mono text-[9px] uppercase tracking-wider text-highlight"
        title="Every PROGRAM in this file is bound to a task and will run on Run."
      >
        scheduled
      </span>
    )
  }
  const target = unscheduled[0]
  return (
    <button
      type="button"
      onClick={() => void onSchedule(target.name)}
      title={`Add PROGRAM ${target.name} to a task so Run actually schedules it`}
      className="inline-flex items-center gap-1 rounded-md border border-dashed border-amber-500/50 px-1.5 py-0.5 font-mono text-[9px] uppercase tracking-wider text-amber-800 hover:bg-amber-500/15 dark:text-amber-300"
    >
      <Plus className="size-2.5" />
      schedule {target.name}
    </button>
  )
}
