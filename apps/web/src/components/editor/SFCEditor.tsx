/**
 * Sequential Function Chart (SFC) — read-only renderer.
 *
 * Parses `.sfc.json` source, draws steps as vertical-stacked boxes
 * with transition bars between them, lists actions and outbound
 * transition conditions inline. Online-mode colouring uses the
 * `__sfc_step` variable from the runtime snapshot to highlight the
 * currently active step.
 *
 * Authoring is JSON-only at this phase. Drag-to-place is deferred to
 * a later phase if it's ever requested — SFC has a strict canonical
 * vertical layout, so authoring-via-mouse adds less value than for
 * FBD (where placement is a real choice).
 */

import { useEffect, useMemo, useState } from "react"

import { checkProgram } from "@/lib/api"
import { cn } from "@/lib/utils"
import { useRuntime } from "@/state/runtime"
import type { CheckDiagnostic } from "@/types/generated/CheckDiagnostic"
import type { SfcLocation } from "@/types/generated/SfcLocation"
import type { SfcProgram } from "@/types/generated/SfcProgram"
import type { SfcTransition } from "@/types/generated/SfcTransition"

// =================================================================
//   Component
// =================================================================

export function SFCEditor({
  value,
  onChange: _onChange,
  className,
}: {
  value: string
  /** Reserved for future authoring phase. Currently unused — the
   *  source of truth is the JSON file; the viewer doesn't mutate. */
  onChange: (next: string) => void
  className?: string
}) {
  const parsed = useMemo(() => safeParse(value), [value])

  // Online-mode: the current step is just a STRING variable named
  // `__sfc_step` in the runtime snapshot. If it's there + matches a
  // declared step, that step renders highlighted. Otherwise the
  // diagram is static.
  const { lastSnapshot, isRunning } = useRuntime()
  const activeStep = useMemo<string | null>(() => {
    if (!isRunning || !lastSnapshot) return null
    const v = lastSnapshot.vars.find((x) => x.name === "__sfc_step")
    if (!v) return null
    // Runtime stringifies STRING values; strip surrounding quotes if any.
    return v.value.replace(/^'/, "").replace(/'$/, "")
  }, [lastSnapshot, isRunning])

  // Diagnostics — same 350 ms debounced poll as LD / FBD.
  const [diagnostics, setDiagnostics] = useState<CheckDiagnostic[]>([])
  useEffect(() => {
    if (parsed.kind === "error") {
      setDiagnostics([])
      return
    }
    const handle = setTimeout(async () => {
      try {
        const diags = await checkProgram(value, "sfc")
        setDiagnostics(diags)
      } catch (e) {
        console.warn("SFC diagnostics fetch failed:", e)
      }
    }, 350)
    return () => clearTimeout(handle)
  }, [value, parsed.kind])

  if (parsed.kind === "error") {
    return (
      <div className={cn("flex h-full min-h-0 flex-col", className)}>
        <div className="border-b border-destructive/40 bg-destructive/5 px-3 py-2 text-xs text-destructive">
          SFC JSON parse error: {parsed.message}
        </div>
        <pre className="flex-1 overflow-auto bg-muted/20 px-4 py-3 font-mono text-xs leading-relaxed text-foreground">
          {value}
        </pre>
      </div>
    )
  }

  const prog = parsed.program
  const diagIndex = useMemo(() => indexDiagnostics(diagnostics), [diagnostics])

  // Group outbound transitions per step so we can render them
  // attached to the step they leave from.
  const outboundByStep = useMemo(() => {
    const m = new Map<string, SfcTransition[]>()
    prog.transitions.forEach((t) => {
      const list = m.get(t.from) ?? []
      list.push(t)
      m.set(t.from, list)
    })
    return m
  }, [prog.transitions])

  return (
    <div className={cn("flex h-full min-h-0 flex-col", className)}>
      <Header prog={prog} activeStep={activeStep} />
      {diagnostics.length > 0 && (
        <DiagnosticsBanner diagnostics={diagnostics} />
      )}
      <div className="flex-1 overflow-auto bg-background">
        <VariablePanel prog={prog} diagIndex={diagIndex} />
        <div className="flex justify-center px-4 py-6">
          <div className="flex w-full max-w-xl flex-col items-stretch gap-0">
            {prog.steps.map((step, i) => {
              const isActive = activeStep === step.name
              const isInitial = step.name === prog.initial_step
              const outbound = outboundByStep.get(step.name) ?? []
              const hasError =
                diagIndex.byStep.has(step.name) ||
                outbound.some((t) =>
                  diagIndex.byTransitionFrom.has(`${t.from}→${t.to}`),
                )
              return (
                <div key={step.name} className="flex flex-col items-stretch">
                  {/* Step box */}
                  <div
                    className={cn(
                      "rounded-md border bg-card shadow-sm",
                      hasError
                        ? "border-destructive"
                        : isActive
                          ? "border-highlight ring-2 ring-highlight/40"
                          : "border-border",
                    )}
                  >
                    <div
                      className={cn(
                        "flex items-center justify-between border-b px-3 py-1.5 font-mono text-xs",
                        isActive
                          ? "border-highlight/40 bg-highlight/10 text-foreground"
                          : "border-border bg-muted/30 text-foreground",
                      )}
                    >
                      <span>
                        <strong>{step.name}</strong>
                        {isInitial && (
                          <span className="ml-2 rounded border border-border bg-muted/50 px-1 py-0.5 text-[9px] uppercase text-muted-foreground">
                            initial
                          </span>
                        )}
                      </span>
                      {isActive && (
                        <span className="rounded bg-highlight/30 px-1.5 py-0.5 text-[9px] font-medium uppercase text-foreground">
                          active
                        </span>
                      )}
                    </div>
                    {step.actions.length > 0 ? (
                      <ul className="space-y-0.5 px-3 py-2 font-mono text-[11px]">
                        {step.actions.map((a, ai) => (
                          <li key={ai} className="flex items-start gap-2">
                            <span
                              className="rounded bg-muted px-1 text-[10px] font-medium text-foreground"
                              title={qualifierTooltip(a.qualifier)}
                            >
                              {a.qualifier}
                            </span>
                            <pre className="flex-1 whitespace-pre-wrap text-foreground">
                              {a.body}
                            </pre>
                          </li>
                        ))}
                      </ul>
                    ) : (
                      <div className="px-3 py-2 font-mono text-[11px] italic text-muted-foreground">
                        (no actions)
                      </div>
                    )}
                  </div>

                  {/* Outbound transitions */}
                  {outbound.length > 0 && (
                    <div className="my-1 flex flex-col items-stretch gap-0.5">
                      {outbound.map((t) => (
                        <TransitionBar
                          key={`${t.from}→${t.to}-${t.condition}`}
                          transition={t}
                          active={activeStep === t.from}
                        />
                      ))}
                    </div>
                  )}

                  {/* Connector line down to next step (visual hint) */}
                  {i < prog.steps.length - 1 && outbound.length === 0 && (
                    <div className="mx-auto h-4 w-px bg-border" />
                  )}
                </div>
              )
            })}
          </div>
        </div>
        <div className="px-4 pb-4 text-[11px] text-muted-foreground">
          <p>
            SFC viewer is read-only — author by editing the JSON
            directly. Transitions list outbound rules attached to each
            step; the cascade is evaluated top-to-bottom in author
            order, so the first satisfied condition wins.
          </p>
        </div>
      </div>
    </div>
  )
}

// =================================================================
//   Header
// =================================================================

function Header({
  prog,
  activeStep,
}: {
  prog: SfcProgram
  activeStep: string | null
}) {
  return (
    <div className="border-b border-border bg-muted/30 px-3 py-1.5 text-[11px] uppercase tracking-wider text-muted-foreground">
      <span className="font-mono normal-case tracking-normal text-foreground">
        {prog.name}
      </span>
      <span className="ml-2 rounded border border-border bg-muted/50 px-1.5 py-0.5 font-mono text-[9px] text-muted-foreground">
        sfc
      </span>
      <span className="ml-2 rounded border border-border bg-muted/50 px-1.5 py-0.5 font-mono text-[9px] text-muted-foreground">
        {prog.pou_type === "function_block" ? "fb" : "prg"}
      </span>
      <span className="ml-3">
        {prog.steps.length} step{prog.steps.length === 1 ? "" : "s"} ·{" "}
        {prog.transitions.length} transition
        {prog.transitions.length === 1 ? "" : "s"}
      </span>
      {activeStep && (
        <span className="ml-3 rounded bg-highlight/15 px-1.5 py-0.5 font-mono text-[9px] normal-case text-foreground">
          → {activeStep}
        </span>
      )}
    </div>
  )
}

// =================================================================
//   Variable panel (reuses the LD/FBD layout)
// =================================================================

function VariablePanel({
  prog,
  diagIndex,
}: {
  prog: SfcProgram
  diagIndex: DiagIndex
}) {
  const groups: Array<{ label: string; section: "input" | "output" | "internal" }> = [
    { label: "VAR_INPUT", section: "input" },
    { label: "VAR_OUTPUT", section: "output" },
    { label: "VAR", section: "internal" },
  ]
  return (
    <div className="grid grid-cols-3 gap-3 border-b border-border bg-muted/10 px-4 py-2 text-[11px]">
      {groups.map((g) => {
        const vs = prog.variables.filter((v) => v.section === g.section)
        return (
          <div key={g.section}>
            <div className="mb-1 font-mono text-[9px] uppercase tracking-wider text-muted-foreground">
              {g.label}
            </div>
            <ul className="space-y-0.5">
              {vs.length === 0 && (
                <li className="text-muted-foreground italic">—</li>
              )}
              {vs.map((v) => (
                <li
                  key={v.name}
                  className={cn(
                    "flex items-center gap-1 rounded px-1 font-mono",
                    diagIndex.byVariable.has(v.name) &&
                      "ring-1 ring-destructive/60",
                  )}
                  title={diagIndex.byVariable.get(v.name)?.[0]?.message ?? undefined}
                >
                  <span className="text-foreground">{v.name}</span>
                  <span className="text-muted-foreground">{v.type}</span>
                  {v.init !== null && v.init !== undefined && (
                    <span className="text-muted-foreground">:= {v.init}</span>
                  )}
                </li>
              ))}
            </ul>
          </div>
        )
      })}
    </div>
  )
}

// =================================================================
//   Transition bar
// =================================================================

function TransitionBar({
  transition,
  active,
}: {
  transition: SfcTransition
  active: boolean
}) {
  return (
    <div
      className={cn(
        "mx-auto flex w-[90%] items-center gap-2 rounded border bg-card px-2 py-0.5 font-mono text-[10px]",
        active
          ? "border-highlight text-foreground"
          : "border-border text-muted-foreground",
      )}
      title={`${transition.from} → ${transition.to}\nwhen ${transition.condition}`}
    >
      <span className="rounded bg-muted px-1 text-foreground">→</span>
      <span className="font-medium text-foreground">{transition.to}</span>
      <span className="ml-auto rounded bg-muted/60 px-1 text-foreground/80">
        {transition.condition}
      </span>
    </div>
  )
}

// =================================================================
//   Diagnostics
// =================================================================

interface DiagIndex {
  byStep: Map<string, CheckDiagnostic[]>
  byTransitionFrom: Map<string, CheckDiagnostic[]>
  byVariable: Map<string, CheckDiagnostic[]>
}

function indexDiagnostics(diags: CheckDiagnostic[]): DiagIndex {
  const byStep = new Map<string, CheckDiagnostic[]>()
  const byTransitionFrom = new Map<string, CheckDiagnostic[]>()
  const byVariable = new Map<string, CheckDiagnostic[]>()
  for (const d of diags) {
    const loc = d.sfc_location
    if (!loc) continue
    switch (loc.kind) {
      case "step":
      case "action": {
        const step = loc.kind === "step" ? loc.name : loc.step
        const list = byStep.get(step) ?? []
        list.push(d)
        byStep.set(step, list)
        break
      }
      case "transition":
        // Without resolving the index back to a (from, to) pair we
        // can't easily group by edge; the banner shows the message
        // and the diag itself carries `index`. We still record it
        // for the banner count.
        break
      case "variable": {
        const list = byVariable.get(loc.name) ?? []
        list.push(d)
        byVariable.set(loc.name, list)
        break
      }
    }
  }
  return { byStep, byTransitionFrom, byVariable }
}

function describeLocation(loc: SfcLocation | null | undefined): string {
  if (!loc) return "—"
  switch (loc.kind) {
    case "variable":
      return `var ${loc.name}`
    case "step":
      return `step ${loc.name}`
    case "action":
      return `step ${loc.step} · action ${loc.action_index}`
    case "transition":
      return `transition #${loc.index}`
  }
}

function DiagnosticsBanner({ diagnostics }: { diagnostics: CheckDiagnostic[] }) {
  return (
    <div className="border-b border-destructive/30 bg-destructive/5 text-xs">
      <div className="flex items-center gap-2 px-3 py-1.5">
        <span className="font-mono font-medium text-destructive">
          {diagnostics.length} {diagnostics.length === 1 ? "error" : "errors"}
        </span>
      </div>
      <ul className="divide-y divide-destructive/15">
        {diagnostics.slice(0, 8).map((d, i) => (
          <li key={i} className="flex items-start gap-2 px-3 py-1">
            <span className="font-mono text-[10px] text-destructive">
              {d.code}
            </span>
            <span className="flex-1 text-foreground">{d.message}</span>
            <span className="font-mono text-[10px] text-muted-foreground">
              {describeLocation(d.sfc_location)}
            </span>
          </li>
        ))}
        {diagnostics.length > 8 && (
          <li className="px-3 py-1 text-muted-foreground">
            +{diagnostics.length - 8} more…
          </li>
        )}
      </ul>
    </div>
  )
}

// =================================================================
//   Helpers
// =================================================================

function qualifierTooltip(q: string): string {
  switch (q) {
    case "N":
      return "N (Non-stored): fires every scan while the step is active"
    case "S":
      return "S (Set): fires once on entry — typically asserts an output"
    case "R":
      return "R (Reset): fires once on entry — typically deasserts an output"
    default:
      return q
  }
}

type Parsed =
  | { kind: "ok"; program: SfcProgram }
  | { kind: "error"; message: string }

function safeParse(source: string): Parsed {
  try {
    const obj = JSON.parse(source)
    if (
      !obj ||
      typeof obj !== "object" ||
      !Array.isArray(obj.steps) ||
      !Array.isArray(obj.transitions)
    ) {
      return { kind: "error", message: "missing `steps` or `transitions` array" }
    }
    return { kind: "ok", program: obj as SfcProgram }
  } catch (e) {
    return { kind: "error", message: String(e) }
  }
}
