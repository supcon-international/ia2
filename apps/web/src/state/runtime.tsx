import {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react"
import type { AppEvent } from "@/types/generated/AppEvent"
import type { ProgramInfo } from "@/types/generated/ProgramInfo"
import type { VarSnapshot } from "@/types/generated/VarSnapshot"
import { eventsUrl, fetchProgram, runProgram, stopProgram } from "@/lib/api"

type RuntimeState = {
  programInfo: ProgramInfo | null
  source: string
  setSource: (s: string) => void
  isDirty: boolean
  isRunning: boolean
  connected: boolean
  lastSnapshot: VarSnapshot | null
  error: string | null
  run: () => Promise<void>
  stop: () => Promise<void>
}

const RuntimeCtx = createContext<RuntimeState | null>(null)

export function RuntimeProvider({ children }: { children: ReactNode }) {
  const [programInfo, setProgramInfo] = useState<ProgramInfo | null>(null)
  const [source, setSource] = useState("")
  const [isRunning, setIsRunning] = useState(false)
  const [connected, setConnected] = useState(false)
  const [lastSnapshot, setLastSnapshot] = useState<VarSnapshot | null>(null)
  const [error, setError] = useState<string | null>(null)
  const esRef = useRef<EventSource | null>(null)

  useEffect(() => {
    fetchProgram()
      .then((p) => {
        setProgramInfo(p)
        setSource(p.source)
      })
      .catch((e) => setError(String(e)))
  }, [])

  useEffect(() => {
    const es = new EventSource(eventsUrl())
    esRef.current = es
    es.onopen = () => setConnected(true)
    es.onerror = () => setConnected(false)
    es.onmessage = (msg) => {
      try {
        const ev = JSON.parse(msg.data) as AppEvent
        switch (ev.type) {
          case "snapshot":
            setLastSnapshot(ev.data)
            break
          case "started":
            setIsRunning(true)
            setError(null)
            break
          case "stopped":
            setIsRunning(false)
            setLastSnapshot(null)
            break
          case "error":
            setError(ev.data)
            break
        }
      } catch {
        /* ignore malformed payloads */
      }
    }
    return () => {
      es.close()
      esRef.current = null
    }
  }, [])

  const run = async () => {
    setError(null)
    try {
      await runProgram(source)
    } catch (e) {
      setError(String(e))
    }
  }

  const stop = async () => {
    try {
      await stopProgram()
    } catch (e) {
      setError(String(e))
    }
  }

  const isDirty = programInfo !== null && source !== programInfo.source

  return (
    <RuntimeCtx.Provider
      value={{
        programInfo,
        source,
        setSource,
        isDirty,
        isRunning,
        connected,
        lastSnapshot,
        error,
        run,
        stop,
      }}
    >
      {children}
    </RuntimeCtx.Provider>
  )
}

export function useRuntime() {
  const ctx = useContext(RuntimeCtx)
  if (!ctx) throw new Error("useRuntime must be used inside RuntimeProvider")
  return ctx
}
