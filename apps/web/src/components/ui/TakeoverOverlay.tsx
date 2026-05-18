import { useState } from "react"
import { Bot, ChevronDown, ChevronUp, Shield, ShieldCheck, X } from "lucide-react"

import { apiFetch } from "@/lib/api"
import { agentActivityStore, useAgentActivity } from "@/state/agent-activity"
import { cn } from "@/lib/utils"

/**
 * Agent-takeover overlay. Renders three things on top of everything
 * when the server reports an agent is active:
 *
 *   1. A pulsing brand-green border around the whole window —
 *      decorative, pointer-events: none, draws over the entire app.
 *   2. A full-screen scrim that intercepts clicks so the user can't
 *      modify state while the agent's in the middle of a multi-step
 *      sequence. Click → micro-shake animation as a "no, take over
 *      first" cue.
 *   3. A top-center banner with the current command + a recent
 *      activity list + a "Take over" button. This replaces the
 *      bottom-right toast as the primary surface during takeover —
 *      agent actions are far more visible at top-center than
 *      tucked in the corner.
 *
 * Layer/z-index plan (from back to front):
 *   500  scrim       (catches clicks, semi-transparent backdrop)
 *   600  banner      (interactive — Take over, expand log)
 *   700  glow        (decorative; pointer-events: none)
 *
 * Same React code runs in both the Mac desktop WKWebView and the
 * dev `vite` browser — visual behaviour stays consistent across
 * environments because there is no native code path for this.
 */
export function TakeoverOverlay() {
  const agent = useAgentActivity()

  if (!agent.effectivelyActive) {
    // Render nothing — even an empty wrapper would cost a render +
    // a layout pass. The banner can briefly outlast active=false
    // when the user clicks "Take over"; we handle that case below
    // via overrideActive, but `effectivelyActive` already encodes it.
    return null
  }

  return (
    <>
      <Scrim />
      <Banner agent={agent} />
      <BorderGlow />
    </>
  )
}

/**
 * Full-screen click-eater. Clear background so the IDE stays visible;
 * just blocks pointer events on the content beneath the banner.
 *
 * Click triggers a micro-shake to remind the user that input is
 * being held until they click "Take over". The shake is cosmetic —
 * the actual block is `pointer-events: auto` on this layer.
 */
function Scrim() {
  const [shake, setShake] = useState(0)
  return (
    <div
      className={cn(
        "fixed inset-0 z-[500] cursor-not-allowed bg-background/30 backdrop-blur-[0.5px]",
        shake > 0 && "animate-pulse",
      )}
      onPointerDown={() => setShake((n) => n + 1)}
      role="presentation"
    />
  )
}

/**
 * Two-layer border treatment — see `apps/web/src/styles.css`
 * `.ia2-takeover-breath` / `.ia2-takeover-comet` for the keyframes.
 *
 * `breath` is the always-visible thick green ring + soft halo.
 * `comet` is a slowly-rotating bright spot that travels around the
 * perimeter to make "the agent is actively doing things" feel
 * physically present rather than just a static accent.
 */
function BorderGlow() {
  return (
    <>
      <div className="ia2-takeover-comet" />
      <div className="ia2-takeover-breath" />
    </>
  )
}

/**
 * Top-center banner. Sticky to the window top with a small inset so
 * the macOS traffic lights remain visible.
 */
function Banner({ agent }: { agent: ReturnType<typeof useAgentActivity> }) {
  const [expanded, setExpanded] = useState(false)

  // When an explicit session is open, its label is the authoritative
  // banner text — it persists across many commands so the user sees
  // "rebuilding tank controller" not the per-command flicker.
  // Otherwise we fall back to the most-recent transient command.
  const inSession = agent.sessionLabel != null
  const primaryLabel =
    agent.sessionLabel ??
    (agent.command ? `cs ${agent.command}` : agent.recent[0]?.command ?? "running")

  return (
    <div
      className={cn(
        "fixed left-1/2 top-3 z-[600] -translate-x-1/2",
        "flex w-[480px] max-w-[calc(100vw-160px)] flex-col gap-0",
        "rounded-xl border-2 shadow-2xl",
        "border-emerald-500/70 bg-popover/95 backdrop-blur",
        "animate-in slide-in-from-top-4 fade-in",
      )}
    >
      <div className="flex items-center gap-3 px-4 py-2.5">
        <div className="relative">
          <Bot className="size-5 text-emerald-500" />
          {/* tiny pulsing dot to communicate "live" */}
          <span className="absolute -right-0.5 -top-0.5 flex size-2">
            <span className="absolute inline-flex size-full animate-ping rounded-full bg-emerald-400 opacity-75" />
            <span className="relative inline-flex size-2 rounded-full bg-emerald-500" />
          </span>
        </div>
        <div className="flex flex-1 flex-col min-w-0">
          <div className="text-[11px] font-semibold uppercase tracking-wider text-emerald-600 dark:text-emerald-400">
            {inSession ? "Agent session" : "Agent in control"}
          </div>
          <div
            className={cn(
              "truncate text-[13px] text-foreground",
              inSession ? "font-medium" : "font-mono",
            )}
          >
            {primaryLabel}
          </div>
        </div>
        {agent.recent.length > 1 && (
          <button
            type="button"
            onClick={() => setExpanded((v) => !v)}
            className="rounded-md px-2 py-1 text-[11px] text-muted-foreground hover:bg-accent/40 hover:text-foreground"
            aria-expanded={expanded}
          >
            {agent.recent.length} {agent.recent.length === 1 ? "action" : "actions"}
            {expanded ? (
              <ChevronUp className="ml-1 inline size-3" />
            ) : (
              <ChevronDown className="ml-1 inline size-3" />
            )}
          </button>
        )}
        {inSession ? (
          // For an explicit session, the visible action is "End"
          // — POSTs to /api/agent/session/end with no id to
          // force-close the session server-side. The overlay drops
          // immediately when the SSE AgentActivity { active:false }
          // event arrives, so the user gets sub-second feedback.
          <button
            type="button"
            onClick={() => {
              void apiFetch("/api/agent/session/end", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: "{}",
              })
            }}
            className={cn(
              "inline-flex items-center gap-1.5 rounded-md px-2.5 py-1",
              "border border-foreground/10 bg-background text-[12px] font-medium",
              "hover:bg-destructive/10 hover:text-destructive",
            )}
            title="End the agent session and return control to you."
          >
            <X className="size-3.5" />
            End session
          </button>
        ) : (
          // For a transient heartbeat (no session), use the local
          // override — heartbeats keep coming for a few seconds and
          // re-end-sessioning them server-side would be noisy.
          <button
            type="button"
            onClick={() => agentActivityStore.requestUserOverride()}
            className={cn(
              "inline-flex items-center gap-1.5 rounded-md px-2.5 py-1",
              "border border-foreground/10 bg-background text-[12px] font-medium",
              "hover:bg-accent/40 hover:text-foreground",
            )}
            title="Suppress the agent overlay for 8 seconds and regain pointer control."
          >
            <Shield className="size-3.5" />
            Take over
          </button>
        )}
      </div>

      {expanded && agent.recent.length > 0 && (
        <div className="border-t border-border/60 px-4 py-2">
          <div className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground mb-1">
            Recent
          </div>
          <ul className="space-y-0.5">
            {agent.recent.map((entry, i) => (
              <li
                key={`${entry.ts}-${i}`}
                className="flex items-center justify-between gap-2 font-mono text-[12px]"
              >
                <span className="truncate">cs {entry.command}</span>
                <span className="shrink-0 text-[10px] text-muted-foreground">
                  {ageLabel(entry.ts)}
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  )
}

/**
 * "2s ago" / "12s ago" — short relative time. We re-render only when
 * the banner re-renders (on state changes), so the labels can drift
 * a little. That's fine; precision isn't useful here.
 */
function ageLabel(ts: number): string {
  const seconds = Math.max(0, Math.round((Date.now() - ts) / 1000))
  if (seconds < 2) return "just now"
  if (seconds < 60) return `${seconds}s ago`
  return `${Math.round(seconds / 60)}m ago`
}

/**
 * Indicator shown when the user has manually claimed back control via
 * "Take over" — small green check at the top corner, fading out after
 * the override window expires.
 */
export function UserControlIndicator() {
  const agent = useAgentActivity()
  if (!agent.overrideActive) return null
  return (
    <div
      className={cn(
        "pointer-events-none fixed right-4 top-4 z-[650]",
        "flex items-center gap-1.5 rounded-full bg-popover/95 px-3 py-1",
        "border border-emerald-500/40 text-[11px] font-medium text-emerald-700 dark:text-emerald-300",
        "animate-in fade-in slide-in-from-top-1",
      )}
    >
      <ShieldCheck className="size-3.5" />
      You're in control
    </div>
  )
}
