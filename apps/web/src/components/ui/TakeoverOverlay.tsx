import { ShieldCheck } from "lucide-react"

import { useAgentActivity } from "@/state/agent-activity"
import { cn } from "@/lib/utils"

/**
 * The window-frame half of the agent-takeover chrome. Purely decorative
 * and `pointer-events: none` — takeover NEVER blocks the user; human and
 * agent drive the app at the same time.
 *
 * Per the design, "the agent is in control" is expressed by exactly two
 * things, and they read as one continuous piece of chrome:
 *
 *   1. this solid acid-green frame around the whole window, and
 *   2. the acid-green status bar docked at the bottom edge
 *      (`AgentStatusBar`, laid out in normal flow by the Shell).
 *
 * Everything the old floating banner carried — what the agent is doing,
 * the scan state, the way out — now lives in that bar, where it can't
 * cover the code the agent is editing. The recent-actions log went with
 * it: the bar shows the current action, and the CLI's own history is the
 * log of record.
 */
export function TakeoverOverlay() {
  const agent = useAgentActivity()

  if (!agent.effectivelyActive) {
    // Render nothing — even an empty wrapper would cost a render + a
    // layout pass. `effectivelyActive` already encodes the brief window
    // where the chrome outlives active=false after "Take over".
    return null
  }

  return <div aria-hidden className="ia2-takeover-frame" />
}

/**
 * Indicator shown when the user has manually claimed back control via
 * "Take over" — small check at the top corner, fading out after the
 * override window expires. Deliberately uses `--highlight` (the
 * conventional green), never the acid green: acid green means "the
 * machine has it", and this says the opposite.
 */
export function UserControlIndicator() {
  const agent = useAgentActivity()
  if (!agent.overrideActive) return null
  return (
    <div
      className={cn(
        "pointer-events-none fixed right-4 top-10 z-[650]",
        "flex items-center gap-1.5 rounded-full bg-popover/95 px-3 py-1",
        "border border-highlight/40 text-[11px] font-medium text-highlight",
        "animate-in fade-in slide-in-from-top-1",
      )}
    >
      <ShieldCheck className="size-3.5" />
      You're in control
    </div>
  )
}
