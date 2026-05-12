import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react"
import type { AppEvent } from "@/types/generated/AppEvent"
import type { Application } from "@/types/generated/Application"
import type { ApplicationKind } from "@/types/generated/ApplicationKind"
import type { CheckDiagnostic } from "@/types/generated/CheckDiagnostic"
import type { Device } from "@/types/generated/Device"
import type { IoMap } from "@/types/generated/IoMap"
import type { ProjectListing } from "@/types/generated/ProjectListing"
import type { ProjectTree } from "@/types/generated/ProjectTree"
import type { Protocol } from "@/types/generated/Protocol"
import type { VarSnapshot } from "@/types/generated/VarSnapshot"
import {
  closeProject as apiCloseProject,
  createApplication as apiCreateApplication,
  createDevice as apiCreateDevice,
  createProject as apiCreateProject,
  deleteApplication as apiDeleteApplication,
  deleteDevice as apiDeleteDevice,
  eventsUrl,
  fetchApplication,
  fetchDevice,
  fetchIomap,
  fetchProject,
  fetchProjects as apiFetchProjects,
  openProject as apiOpenProject,
  runProgram,
  saveApplication,
  stopProgram,
  updateDevice as apiUpdateDevice,
  updateIomap as apiUpdateIomap,
} from "@/lib/api"
import { LspClient } from "@/lib/lsp-client"

export type View = "app" | "device" | "iomap"

type AppState = {
  // Project
  project: ProjectTree | null
  projectLoading: boolean
  availableProjects: ProjectListing[]

  // Center-pane focus
  view: View | null
  currentApp: Application | null
  source: string
  setSource: (s: string) => void
  isDirty: boolean
  diagnostics: CheckDiagnostic[]

  currentDevice: Device | null
  iomap: IoMap

  // Runtime
  isRunning: boolean
  connected: boolean
  lastSnapshot: VarSnapshot | null

  // Errors
  error: string | null

  // Project actions
  createProject: (name: string) => Promise<void>
  openProject: (path: string) => Promise<void>
  closeProject: () => Promise<void>
  refreshProjects: () => Promise<void>
  refreshProject: () => Promise<void>

  // Selection actions
  selectApp: (name: string) => Promise<void>
  selectDevice: (name: string) => Promise<void>
  openIoMap: () => Promise<void>

  // App/Device mutations
  saveCurrentApp: () => Promise<void>
  createApp: (name: string, kind: ApplicationKind) => Promise<void>
  deleteApp: (name: string) => Promise<void>
  createDevice: (name: string, protocol: Protocol) => Promise<void>
  deleteDevice: (name: string) => Promise<void>
  saveDevice: (device: Device) => Promise<void>
  saveIomap: (iomap: IoMap) => Promise<void>

  // Runtime actions
  run: () => Promise<void>
  stop: () => Promise<void>
}

const Ctx = createContext<AppState | null>(null)

export function RuntimeProvider({ children }: { children: ReactNode }) {
  const [project, setProject] = useState<ProjectTree | null>(null)
  const [projectLoading, setProjectLoading] = useState(true)
  const [availableProjects, setAvailableProjects] = useState<ProjectListing[]>(
    [],
  )

  const [view, setView] = useState<View | null>(null)
  const [currentApp, setCurrentApp] = useState<Application | null>(null)
  const [source, setSource] = useState("")
  const [diagnostics, setDiagnostics] = useState<CheckDiagnostic[]>([])
  const [currentDevice, setCurrentDevice] = useState<Device | null>(null)
  const [iomap, setIomap] = useState<IoMap>({ mappings: [] })

  const [isRunning, setIsRunning] = useState(false)
  const [connected, setConnected] = useState(false)
  const [lastSnapshot, setLastSnapshot] = useState<VarSnapshot | null>(null)
  const [error, setError] = useState<string | null>(null)

  const esRef = useRef<EventSource | null>(null)

  // ---------------- Bootstrap ----------------

  const refreshProject = useCallback(async () => {
    try {
      const tree = await fetchProject()
      setProject(tree)
      // Drop currentApp if the project lost it (e.g. deleted).
      if (
        tree &&
        currentApp &&
        !tree.applications.some((a) => a.name === currentApp.name)
      ) {
        setCurrentApp(null)
        setSource("")
      }
    } catch (e) {
      setError(String(e))
    }
  }, [currentApp])

  const refreshProjects = useCallback(async () => {
    try {
      setAvailableProjects(await apiFetchProjects())
    } catch (e) {
      setError(String(e))
    }
  }, [])

  // Initial fetch.
  useEffect(() => {
    ;(async () => {
      try {
        const tree = await fetchProject()
        setProject(tree)
        if (!tree) {
          setAvailableProjects(await apiFetchProjects())
        }
      } catch (e) {
        setError(String(e))
      } finally {
        setProjectLoading(false)
      }
    })()
  }, [])

  // Auto-select the first POU when a project loads and nothing is open yet.
  useEffect(() => {
    if (!project || view !== null) return
    if (project.applications.length === 0) return
    void selectApp(project.applications[0].name)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project])

  // Keep the iomap state in sync with the tree.
  useEffect(() => {
    if (project) setIomap(project.iomap)
  }, [project])

  // ---------------- SSE ----------------

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
            // Don't wipe the previous last-known snapshot on start — the
            // Monitor pane keeps showing values until the first new
            // snapshot arrives, avoiding a one-frame flicker to "—".
            break
          case "stopped":
            setIsRunning(false)
            // Keep `lastSnapshot` so the Monitor pane displays the final
            // state of the last run as "stale". History buffers also
            // survive so trend charts remain interpretable.
            break
          case "error":
            setError(ev.data)
            break
        }
      } catch {
        /* ignore */
      }
    }
    return () => {
      es.close()
      esRef.current = null
    }
  }, [])

  // ---------------- LSP-driven diagnostics ----------------

  // One client per opened POU. Tears down on app switch / project close;
  // publishDiagnostics from the ironplc LSP land in `diagnostics` and
  // flow to Monaco markers + the ProgramPane header badge.
  const lspRef = useRef<LspClient | null>(null)
  useEffect(() => {
    if (!currentApp) {
      setDiagnostics([])
      lspRef.current?.dispose()
      lspRef.current = null
      return
    }
    const client = new LspClient({
      uri: `file:///${currentApp.name}.st`,
      languageId: "iec61131",
      onDiagnostics: setDiagnostics,
    })
    lspRef.current = client
    return () => {
      client.dispose()
      lspRef.current = null
    }
  }, [currentApp?.name])

  useEffect(() => {
    lspRef.current?.setSource(source)
  }, [source])

  // ---------------- Project actions ----------------

  const createProject = useCallback(async (name: string) => {
    setError(null)
    try {
      await apiCreateProject(name)
      const tree = await fetchProject()
      setProject(tree)
      setCurrentApp(null)
      setSource("")
    } catch (e) {
      setError(String(e))
    }
  }, [])

  const openProject = useCallback(async (path: string) => {
    setError(null)
    try {
      await apiOpenProject(path)
      const tree = await fetchProject()
      setProject(tree)
      setCurrentApp(null)
      setSource("")
    } catch (e) {
      setError(String(e))
    }
  }, [])

  const closeProject = useCallback(async () => {
    setError(null)
    try {
      await apiCloseProject()
      setProject(null)
      setCurrentApp(null)
      setSource("")
      setIsRunning(false)
      setLastSnapshot(null)
      setAvailableProjects(await apiFetchProjects())
    } catch (e) {
      setError(String(e))
    }
  }, [])

  // ---------------- App / Device actions ----------------

  const selectApp = useCallback(async (name: string) => {
    setError(null)
    try {
      const app = await fetchApplication(name)
      setCurrentApp(app)
      setSource(app.source)
      setView("app")
    } catch (e) {
      setError(String(e))
    }
  }, [])

  const selectDevice = useCallback(async (name: string) => {
    setError(null)
    try {
      const device = await fetchDevice(name)
      setCurrentDevice(device)
      setView("device")
    } catch (e) {
      setError(String(e))
    }
  }, [])

  const openIoMap = useCallback(async () => {
    setError(null)
    try {
      const m = await fetchIomap()
      setIomap(m)
      setView("iomap")
    } catch (e) {
      setError(String(e))
    }
  }, [])

  const saveDevice = useCallback(
    async (device: Device) => {
      setError(null)
      try {
        await apiUpdateDevice(device.name, device)
        setCurrentDevice(device)
        await refreshProject()
      } catch (e) {
        setError(String(e))
      }
    },
    [refreshProject],
  )

  const saveIomap = useCallback(
    async (next: IoMap) => {
      setError(null)
      try {
        await apiUpdateIomap(next)
        setIomap(next)
        await refreshProject()
      } catch (e) {
        setError(String(e))
      }
    },
    [refreshProject],
  )

  const saveCurrentApp = useCallback(async () => {
    if (!currentApp) return
    try {
      await saveApplication(currentApp.name, source)
      setCurrentApp({ ...currentApp, source })
    } catch (e) {
      setError(String(e))
    }
  }, [currentApp, source])

  const createApp = useCallback(
    async (name: string, kind: ApplicationKind) => {
      setError(null)
      try {
        const app = await apiCreateApplication(name, kind)
        await refreshProject()
        setCurrentApp(app)
        setSource(app.source)
      } catch (e) {
        setError(String(e))
      }
    },
    [refreshProject],
  )

  const deleteApp = useCallback(
    async (name: string) => {
      setError(null)
      try {
        await apiDeleteApplication(name)
        if (currentApp?.name === name) {
          setCurrentApp(null)
          setSource("")
        }
        await refreshProject()
      } catch (e) {
        setError(String(e))
      }
    },
    [currentApp, refreshProject],
  )

  const createDevice = useCallback(
    async (name: string, protocol: Protocol) => {
      setError(null)
      try {
        await apiCreateDevice(name, protocol)
        await refreshProject()
      } catch (e) {
        setError(String(e))
      }
    },
    [refreshProject],
  )

  const deleteDevice = useCallback(
    async (name: string) => {
      setError(null)
      try {
        await apiDeleteDevice(name)
        await refreshProject()
      } catch (e) {
        setError(String(e))
      }
    },
    [refreshProject],
  )

  // ---------------- Run / Stop ----------------

  const run = useCallback(async () => {
    setError(null)
    try {
      if (currentApp && source !== currentApp.source) {
        await saveApplication(currentApp.name, source)
        setCurrentApp({ ...currentApp, source })
      }
      await runProgram(currentApp?.name)
    } catch (e) {
      setError(String(e))
    }
  }, [currentApp, source])

  const stop = useCallback(async () => {
    try {
      await stopProgram()
    } catch (e) {
      setError(String(e))
    }
  }, [])

  const isDirty = !!currentApp && currentApp.source !== source

  return (
    <Ctx.Provider
      value={{
        project,
        projectLoading,
        availableProjects,
        view,
        currentApp,
        source,
        setSource,
        isDirty,
        diagnostics,
        currentDevice,
        iomap,
        isRunning,
        connected,
        lastSnapshot,
        error,
        createProject,
        openProject,
        closeProject,
        refreshProjects,
        refreshProject,
        selectApp,
        selectDevice,
        openIoMap,
        saveCurrentApp,
        createApp,
        deleteApp,
        createDevice,
        deleteDevice,
        saveDevice,
        saveIomap,
        run,
        stop,
      }}
    >
      {children}
    </Ctx.Provider>
  )
}

export function useRuntime() {
  const ctx = useContext(Ctx)
  if (!ctx) throw new Error("useRuntime must be used inside RuntimeProvider")
  return ctx
}
