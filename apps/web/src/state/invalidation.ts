/**
 * Cache-invalidation bus.
 *
 * Bridges server-pushed `Mutation` events (over `/api/events` SSE) to
 * any subscriber that has a refetch function for the affected topic.
 *
 * Why a bus, not a context: invalidations come from one global place
 * (the SSE handler in RuntimeProvider) and fan out to many unrelated
 * subscribers — POU editor, project tree, IO mapping pane. A bus
 * decouples emitter from receivers without each subscriber having to
 * be plumbed through React context.
 *
 * Wire format: backends emit canonical lowercase topic strings, see
 * `crates/server/src/events.rs` `mod topic` for the source of truth.
 * Special wildcards:
 *   "*"  — receives every emit (for debugging / global telemetry)
 */

type Listener = () => void

class InvalidationBus {
  private listeners = new Map<string, Set<Listener>>()

  /** Subscribe to `topic`. Returns an unsubscriber. */
  subscribe(topic: string, listener: Listener): () => void {
    let set = this.listeners.get(topic)
    if (!set) {
      set = new Set()
      this.listeners.set(topic, set)
    }
    set.add(listener)
    return () => {
      const s = this.listeners.get(topic)
      s?.delete(listener)
      if (s && s.size === 0) this.listeners.delete(topic)
    }
  }

  /** Fire all listeners on `topic` plus the global "*" channel. */
  emit(topic: string): void {
    this.listeners.get(topic)?.forEach((fn) => {
      try {
        fn()
      } catch (e) {
        console.error("[invalidation] listener threw on", topic, e)
      }
    })
    this.listeners.get("*")?.forEach((fn) => {
      try {
        fn()
      } catch {}
    })
  }
}

export const invalidationBus = new InvalidationBus()

/** Topic strings — keep in sync with `crates/server/src/events.rs::topic`. */
export const Topic = {
  PROJECT: "project",
  PROJECT_META: "project_meta",
  DEVICES: "devices",
  EDGES: "edges",
  IOMAP: "iomap",
  HMI: "hmi",
  TASKS: "tasks",
  pou: (path: string) => `pou:${path}`,
  device: (name: string) => `device:${name}`,
  edge: (name: string) => `edge:${name}`,
} as const
