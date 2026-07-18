/**
 * HMI mutation feed — which screen changed and which node ids a mutation
 * touched. RuntimeProvider's SSE handler writes here on every `hmi`
 * Mutation event; the canvas subscribes to reload the document and to
 * spawn-animate exactly the touched elements (Pencil-style incremental
 * rendering). Same `useSyncExternalStore` singleton pattern as
 * `live-feed.ts` — a screen edit must not re-render the whole context.
 */

import { useSyncExternalStore } from "react"

export type HmiMutation = {
  /** Screen slug the mutation targeted. */
  path: string
  /** Node ids created/modified by the batch (empty = whole-doc save). */
  touched: string[]
  /** Monotonic sequence so equal-looking mutations still notify. */
  seq: number
  /** True when the screen was deleted rather than upserted. */
  deleted: boolean
}

type Listener = () => void

class HmiLiveStore {
  private last: HmiMutation | null = null
  private seq = 0
  private listeners = new Set<Listener>()

  getSnapshot = (): HmiMutation | null => this.last

  subscribe = (l: Listener): (() => void) => {
    this.listeners.add(l)
    return () => {
      this.listeners.delete(l)
    }
  }

  upserted(path: string, touched: string[]): void {
    this.last = { path, touched, seq: ++this.seq, deleted: false }
    this.listeners.forEach((l) => l())
  }

  deleted(path: string): void {
    this.last = { path, touched: [], seq: ++this.seq, deleted: true }
    this.listeners.forEach((l) => l())
  }
}

export const hmiLiveStore = new HmiLiveStore()

/** Latest HMI mutation (null until the first one this session). */
export function useHmiMutation(): HmiMutation | null {
  return useSyncExternalStore(hmiLiveStore.subscribe, hmiLiveStore.getSnapshot)
}
