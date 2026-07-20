/**
 * The canvas's seam between "where the screen lives" and "how it renders".
 * The same HmiCanvas serves two hosts:
 *
 *   - the IDE (HmiPane): documents come from the project server, writes go
 *     through `/api/runtime/...`, nav switches the workbench view;
 *   - the standalone edge panel (HmiStandalone): documents come read-only
 *     from the edge runtime's `/api/hmi`, writes go to its `/write`, nav
 *     swaps the panel URL.
 *
 * Everything the canvas needs from its surroundings passes through this
 * interface, so the edge bundle never imports the IDE's state layer.
 */

import { createContext, useContext } from "react"

import type { HmiDoc } from "@/types/generated/HmiDoc"
import type { RuntimeMode } from "@/types/generated/RuntimeMode"

export type HmiRuntimeState = {
  running: boolean
  /** Active fault / last error to show in the alarm bar; null when calm. */
  alarm: string | null
  /** Scan-loop mode when the host's status carries it — paused/step
   *  must not present as a healthy green run. */
  mode?: RuntimeMode["kind"]
  /** Devices whose transport is down (their input variables are frozen
   *  at last-known values). Empty/absent = fieldbus healthy. */
  unhealthyDevices?: string[]
}

export type HmiHost = {
  fetchDoc(path: string): Promise<HmiDoc>
  /** Persist a whole document (Arrange-mode drag). Absent → layout is
   *  read-only, which is the edge panel's case. */
  saveDoc?(path: string, doc: HmiDoc): Promise<unknown>
  /** Write one variable. `typeName` comes from the live snapshot so the
   *  host can bit-pack REALs correctly. */
  write(name: string, value: number, typeName: string): Promise<void>
  /** Navigate to another screen (a `nav` action). */
  nav(target: string): void
  /** Polled by the alarm bar (~2 s cadence). */
  runtimeState(): Promise<HmiRuntimeState>
}

const HmiHostContext = createContext<HmiHost | null>(null)

export const HmiHostProvider = HmiHostContext.Provider

export function useHmiHost(): HmiHost {
  const host = useContext(HmiHostContext)
  if (!host) throw new Error("HmiCanvas must be rendered inside HmiHostProvider")
  return host
}
