/**
 * Agent-takeover state bus.
 *
 * Sourced from the server's `AgentActivity` SSE events. Holds:
 *   - `active`     — is at least one agent session currently mid-flight
 *   - `command`    — what the latest agent announced ("pou create")
 *   - `session`    — stable id for one CLI process; used to tell
 *                    apart "one agent doing many commands" vs many
 *   - `recent`     — small ring of recent commands for the activity log
 *   - `localOverride` — when the user clicks "Take over", we ignore
 *                       agent activity for `OVERRIDE_MS` so the UI
 *                       doesn't keep snapping back. Best-effort, local
 *                       only — agents that genuinely care should
 *                       throttle themselves.
 *
 * Why a custom store and not React state in a provider: the takeover
 * overlay lives at app root and the per-pane consumers each want a
 * cheap subscription. A plain singleton with `useSyncExternalStore`
 * gives us O(1) reads in any component without prop-drilling and
 * without re-render storms.
 */

import { useSyncExternalStore } from "react"

export type AgentActivityState = {
  active: boolean
  command: string | null
  session: string | null
  /** Wall-clock ms at which we last received an event. */
  lastEventAt: number
  /**
   * Most recent commands the agent ran, newest first. We keep a small
   * tail so the user can read "what happened" without scrolling. Cap
   * at 8 — anything older shows up in the toast bus stream anyway.
   */
  recent: Array<{ command: string; ts: number }>
  /** ms-since-epoch until which we suppress the takeover overlay. */
  overrideUntil: number
}

const OVERRIDE_MS = 8_000

const initialState: AgentActivityState = {
  active: false,
  command: null,
  session: null,
  lastEventAt: 0,
  recent: [],
  overrideUntil: 0,
}

type Listener = () => void

class AgentActivityStore {
  private state: AgentActivityState = initialState
  private listeners = new Set<Listener>()

  getSnapshot = (): AgentActivityState => this.state

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  ingest(event: { active: boolean; command?: string | null; session?: string | null }): void {
    const now = Date.now()
    const command = event.command ?? null
    const session = event.session ?? null
    const recent =
      event.active && command
        ? this.appendRecent(command, now)
        : this.state.recent
    this.state = {
      ...this.state,
      active: event.active,
      command,
      session,
      lastEventAt: now,
      recent,
    }
    this.fire()
  }

  /** User clicked "Take over" — hide the overlay for OVERRIDE_MS. */
  requestUserOverride(): void {
    this.state = {
      ...this.state,
      overrideUntil: Date.now() + OVERRIDE_MS,
    }
    this.fire()
  }

  private appendRecent(command: string, ts: number): AgentActivityState["recent"] {
    // De-dupe consecutive identical commands so a burst of one
    // command doesn't fill the ring.
    const head = this.state.recent[0]
    if (head?.command === command && ts - head.ts < 800) {
      return this.state.recent
    }
    return [{ command, ts }, ...this.state.recent].slice(0, 8)
  }

  private fire(): void {
    this.listeners.forEach((l) => l())
  }
}

export const agentActivityStore = new AgentActivityStore()

/**
 * React hook. Returns the effective takeover state — `active` is
 * `true` only when the server says so AND the user's override window
 * has expired. The raw `agentActive` is also exposed for the banner
 * which wants to show even during override (the user shouldn't lose
 * sight of the fact an agent is running).
 */
export function useAgentActivity(): AgentActivityState & {
  effectivelyActive: boolean
  overrideActive: boolean
} {
  const state = useSyncExternalStore(
    agentActivityStore.subscribe,
    agentActivityStore.getSnapshot,
    agentActivityStore.getSnapshot,
  )
  const overrideActive = state.overrideUntil > Date.now()
  return {
    ...state,
    overrideActive,
    effectivelyActive: state.active && !overrideActive,
  }
}
