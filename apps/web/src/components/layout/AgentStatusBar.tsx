import { useEffect } from "react"
import { Bot } from "lucide-react"

import { apiFetch } from "@/lib/api"
import { agentActivityStore, useAgentActivity } from "@/state/agent-activity"
import { useRuntime } from "@/state/runtime"

/**
 * The acid-green agent status bar — the design's primary "machine is in
 * control" surface. Docks to the very bottom of the shell IN NORMAL FLOW
 * (the app content shrinks by 26px rather than being overlapped), pairing
 * with the solid window frame from TakeoverOverlay so the two read as one
 * piece of chrome clamping the app.
 *
 * Layout (from the Figma bar spec):
 *   left : ● dot · bot glyph · AGENT IN CONTROL · <activity> | <scan state> ▍
 *   right: [Take over / End session] ⌘.
 *
 * ⌘. (or Ctrl+. off-mac) triggers the same action as the button — the
 * keyboard hint printed at the far right is real, not decoration.
 */
export function AgentStatusBar() {
  const agent = useAgentActivity()
  const { isRunning, tasks } = useRuntime()

  const inSession = agent.sessionLabel != null
  const active = agent.effectivelyActive

  // Human-takeover action: end an explicit session server-side, or
  // locally suppress transient heartbeats.
  const takeOver = () => {
    if (inSession) {
      void apiFetch("/api/agent/session/end", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: "{}",
      })
    } else {
      agentActivityStore.requestUserOverride()
    }
  }

  // ⌘. — the shortcut the bar advertises. Bound only while the bar is
  // visible so it can't swallow the key when no agent is active.
  useEffect(() => {
    if (!active) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "." && (e.metaKey || e.ctrlKey)) {
        e.preventDefault()
        takeOver()
      }
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, inSession])

  if (!active) return null

  const activity =
    agent.sessionLabel ??
    (agent.command ? `cs ${agent.command}` : (agent.recent[0]?.command ?? "working"))
  // Scan period comes from the first configured task — the same number
  // the Monitor pill shows, so the two never disagree.
  const interval = tasks.tasks[0]?.interval_ms
  const scanText = isRunning
    ? `scan running${interval ? ` · ${interval}ms` : ""}`
    : "scan stopped"

  return (
    <div
      data-testid="agent-status-bar"
      className="relative z-[710] flex h-[26px] shrink-0 items-center gap-2 bg-agent px-3 font-mono text-[11px] text-agent-foreground"
    >
      <span aria-hidden className="size-[6px] rounded-full bg-agent-foreground" />
      {/* Bot glyph in a subtle darker-green chip, per the Figma bar. */}
      <span
        aria-hidden
        className="flex size-[18px] items-center justify-center rounded-[5px] bg-agent-foreground/10"
      >
        <Bot className="size-3.5" strokeWidth={2.25} />
      </span>
      <span className="font-bold tracking-wide">AGENT IN CONTROL</span>
      <span className="min-w-0 truncate font-semibold">{activity}</span>
      <span aria-hidden className="opacity-50">|</span>
      <span className="whitespace-nowrap">{scanText}</span>
      <span aria-hidden className="ia2-agent-caret" />
      <span className="flex-1" />
      {/* One label — "Take over" — regardless of whether an explicit
       * session is open. The design frames this as the HUMAN reclaiming
       * control; whether that ends a server session or just suppresses a
       * heartbeat is an implementation detail the operator shouldn't have
       * to reason about. Cream chip, near-black text: the one light
       * object on the acid bar, mirroring the Save button's role in the
       * toolbar. */}
      <button
        type="button"
        onClick={takeOver}
        className="rounded-[4px] bg-[#f9fafb] px-2 py-[2px] text-[11px] font-semibold text-[#1a1a19] hover:bg-white"
        title={
          inSession
            ? "Take control back from the agent — ends its session (⌘.)"
            : "Take control back from the agent (⌘.)"
        }
      >
        Take over
      </button>
      <span aria-hidden className="text-[11px] font-medium opacity-70">
        ⌘.
      </span>
    </div>
  )
}
