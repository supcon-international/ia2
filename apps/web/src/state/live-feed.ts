/**
 * High-frequency runtime feed — the last VarSnapshot and the SSE link
 * state — kept OUTSIDE the RuntimeProvider context on purpose.
 *
 * Snapshots arrive at up to tens of Hz while a program runs. Anything
 * carried in the runtime context re-renders all of its consumers on
 * every change, and only four components actually read the snapshot
 * (Monitor + the three graphical editors' live overlays). A plain
 * singleton with `useSyncExternalStore` gives those four a cheap,
 * precise subscription and leaves every dialog, tree and toolbar
 * untouched by the firehose — same pattern as `agentActivityStore`.
 *
 * RuntimeProvider owns the SSE plumbing and WRITES here; components
 * READ via the two hooks. Nothing else should import the store
 * directly for writes.
 */

import { useSyncExternalStore } from "react"

import type { VarSnapshot } from "@/types/generated/VarSnapshot"

type Listener = () => void

class LiveFeedStore {
  private snapshot: VarSnapshot | null = null
  private connected = false
  private snapshotListeners = new Set<Listener>()
  private connectedListeners = new Set<Listener>()

  // Two listener sets so a connectivity flip doesn't wake snapshot
  // subscribers and vice versa — they change at wildly different rates.

  getSnapshot = (): VarSnapshot | null => this.snapshot
  getConnected = (): boolean => this.connected

  subscribeSnapshot = (l: Listener): (() => void) => {
    this.snapshotListeners.add(l)
    return () => {
      this.snapshotListeners.delete(l)
    }
  }

  subscribeConnected = (l: Listener): (() => void) => {
    this.connectedListeners.add(l)
    return () => {
      this.connectedListeners.delete(l)
    }
  }

  setSnapshot(snap: VarSnapshot | null): void {
    this.snapshot = snap
    this.snapshotListeners.forEach((l) => l())
  }

  setConnected(up: boolean): void {
    if (this.connected === up) return
    this.connected = up
    this.connectedListeners.forEach((l) => l())
  }
}

export const liveFeedStore = new LiveFeedStore()

/** Last VarSnapshot from the running program (server or attached edge),
 *  `null` when nothing has run yet or the project was closed. */
export function useLastSnapshot(): VarSnapshot | null {
  return useSyncExternalStore(
    liveFeedStore.subscribeSnapshot,
    liveFeedStore.getSnapshot,
  )
}

/** Is the SSE event stream to the server currently up? */
export function useConnected(): boolean {
  return useSyncExternalStore(
    liveFeedStore.subscribeConnected,
    liveFeedStore.getConnected,
  )
}
