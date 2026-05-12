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
import type { Edge } from "@/types/generated/Edge"
import type { IoMap } from "@/types/generated/IoMap"
import type { MigrationResponse } from "@/types/generated/MigrationResponse"
import type { ProjectListing } from "@/types/generated/ProjectListing"
import type { ProjectTree } from "@/types/generated/ProjectTree"
import type { Protocol } from "@/types/generated/Protocol"
import type { Tasks } from "@/types/generated/Tasks"
import type { VarSnapshot } from "@/types/generated/VarSnapshot"
import {
  attachEdge as apiAttachEdge,
  closeProject as apiCloseProject,
  createApplication as apiCreateApplication,
  createApplicationFolder as apiCreateApplicationFolder,
  createDevice as apiCreateDevice,
  createDeviceFolder as apiCreateDeviceFolder,
  createEdge as apiCreateEdge,
  createEdgeFolder as apiCreateEdgeFolder,
  createProject as apiCreateProject,
  deleteApplication as apiDeleteApplication,
  deleteDevice as apiDeleteDevice,
  deleteEdge as apiDeleteEdge,
  detachEdge as apiDetachEdge,
  eventsUrl,
  fetchApplication,
  fetchDevice,
  fetchEdge,
  fetchIomap,
  fetchProject,
  fetchProjects as apiFetchProjects,
  migrateTasks as apiMigrateTasks,
  openProject as apiOpenProject,
  runProgram,
  saveApplication,
  stopProgram,
  updateDevice as apiUpdateDevice,
  updateEdge as apiUpdateEdge,
  updateIomap as apiUpdateIomap,
  updateTasks as apiUpdateTasks,
} from "@/lib/api"
import { LspClient } from "@/lib/lsp-client"

export type View = "app" | "device" | "iomap" | "edge" | "tasks"

/** Which edge (if any) the IDE is currently attached to. When attached,
 * the SSE source switches from the local bridge to the edge's runtime
 * via the SSH-forwarded port. */
export type AttachedEdge = { name: string; localPort: number } | null

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
  currentEdge: Edge | null
  iomap: IoMap
  tasks: Tasks

  /** Live attachment to an edge runtime (or null when running locally). */
  attached: AttachedEdge

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
  selectEdge: (name: string) => Promise<void>
  openIoMap: () => Promise<void>
  openTasks: () => Promise<void>

  // App/Device mutations
  saveCurrentApp: () => Promise<void>
  createApp: (name: string, kind: ApplicationKind) => Promise<void>
  deleteApp: (name: string) => Promise<void>
  createDevice: (name: string, protocol: Protocol) => Promise<void>
  deleteDevice: (name: string) => Promise<void>
  createAppFolder: (path: string) => Promise<void>
  createDeviceFolder: (path: string) => Promise<void>
  createEdge: (name: string, host: string) => Promise<void>
  deleteEdge: (name: string) => Promise<void>
  saveEdge: (edge: Edge) => Promise<void>
  createEdgeFolder: (path: string) => Promise<void>

  /** Open an SSH tunnel to the edge's runtime port and switch the SSE
   * stream over to it. Updates `attached`. */
  attachEdge: (name: string) => Promise<void>
  /** Close the SSH tunnel and switch back to the local bridge SSE. */
  detachEdge: () => Promise<void>

  saveTasks: (tasks: Tasks) => Promise<void>
  /** One-shot migration: extract inline CONFIGURATION blocks from POUs
   * into tasks.toml, strip them from the POU source files. */
  migrateTasks: () => Promise<MigrationResponse>
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
  const [currentEdge, setCurrentEdge] = useState<Edge | null>(null)
  const [attached, setAttached] = useState<AttachedEdge>(null)
  const [iomap, setIomap] = useState<IoMap>({ mappings: [] })
  const [tasks, setTasks] = useState<Tasks>({ tasks: [], programs: [] })

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

  // Keep the iomap + tasks state in sync with the tree.
  useEffect(() => {
    if (project) {
      setIomap(project.iomap)
      setTasks(project.tasks)
    }
  }, [project])

  // ---------------- SSE source ----------------
  //
  // Two modes:
  //  (a) Local — subscribe to /api/events, which carries AppEvent JSON.
  //  (b) Attached to an edge — subscribe to the SSH-forwarded edge runtime
  //      at http://127.0.0.1:<localPort>/events, which streams bare
  //      VarSnapshot JSON. We synthesize "started" / "stopped" from the
  //      tunnel coming up and going down.
  //
  // The effect tears down and rebuilds the EventSource whenever the
  // attachment state changes, so existing MonitorPane / VariablesPanel
  // consumers of `lastSnapshot` keep working without knowing about edges.

  useEffect(() => {
    const url = attached
      ? `http://127.0.0.1:${attached.localPort}/events`
      : eventsUrl()
    const es = new EventSource(url)
    esRef.current = es
    es.onopen = () => {
      setConnected(true)
      if (attached) {
        // Edge runtime has no "started" event — the tunnel coming up is
        // good enough; the program has been running on the edge for a
        // while already.
        setIsRunning(true)
        setError(null)
      }
    }
    es.onerror = () => setConnected(false)
    es.onmessage = (msg) => {
      try {
        if (attached) {
          // Edge runtime: payload is a VarSnapshot directly.
          const snap = JSON.parse(msg.data) as VarSnapshot
          setLastSnapshot(snap)
        } else {
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
              break
            case "error":
              setError(ev.data)
              break
          }
        }
      } catch {
        /* ignore */
      }
    }
    return () => {
      es.close()
      esRef.current = null
    }
  }, [attached])

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

  const selectEdge = useCallback(async (name: string) => {
    setError(null)
    try {
      const edge = await fetchEdge(name)
      setCurrentEdge(edge)
      setView("edge")
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

  const openTasks = useCallback(async () => {
    setError(null)
    setView("tasks")
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

  const saveTasks = useCallback(
    async (next: Tasks) => {
      setError(null)
      try {
        await apiUpdateTasks(next)
        setTasks(next)
        await refreshProject()
      } catch (e) {
        setError(String(e))
      }
    },
    [refreshProject],
  )

  const migrateTasks = useCallback(async (): Promise<MigrationResponse> => {
    setError(null)
    try {
      const report = await apiMigrateTasks()
      await refreshProject()
      return report
    } catch (e) {
      setError(String(e))
      return {
        migrated: false,
        tasks_count: 0,
        programs_count: 0,
        pous_modified: [],
      }
    }
  }, [refreshProject])

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

  const createAppFolder = useCallback(
    async (path: string) => {
      setError(null)
      try {
        await apiCreateApplicationFolder(path)
        await refreshProject()
      } catch (e) {
        setError(String(e))
      }
    },
    [refreshProject],
  )

  const createDeviceFolderCb = useCallback(
    async (path: string) => {
      setError(null)
      try {
        await apiCreateDeviceFolder(path)
        await refreshProject()
      } catch (e) {
        setError(String(e))
      }
    },
    [refreshProject],
  )

  // ---------------- Edge actions ----------------

  const createEdgeAction = useCallback(
    async (name: string, host: string) => {
      setError(null)
      try {
        await apiCreateEdge(name, host)
        await refreshProject()
      } catch (e) {
        setError(String(e))
      }
    },
    [refreshProject],
  )

  const deleteEdgeAction = useCallback(
    async (name: string) => {
      setError(null)
      try {
        await apiDeleteEdge(name)
        if (currentEdge?.name === name) setCurrentEdge(null)
        if (attached?.name === name) setAttached(null)
        await refreshProject()
      } catch (e) {
        setError(String(e))
      }
    },
    [currentEdge, attached, refreshProject],
  )

  const saveEdge = useCallback(
    async (edge: Edge) => {
      setError(null)
      try {
        await apiUpdateEdge(edge.name, edge)
        setCurrentEdge(edge)
        await refreshProject()
      } catch (e) {
        setError(String(e))
      }
    },
    [refreshProject],
  )

  const createEdgeFolderAction = useCallback(
    async (path: string) => {
      setError(null)
      try {
        await apiCreateEdgeFolder(path)
        await refreshProject()
      } catch (e) {
        setError(String(e))
      }
    },
    [refreshProject],
  )

  const attachEdgeAction = useCallback(async (name: string) => {
    setError(null)
    try {
      // If something's already attached, detach it first so we don't leak
      // tunnels server-side.
      await apiDetachEdge(name).catch(() => {})
      const info = await apiAttachEdge(name)
      setAttached({ name, localPort: info.local_port })
    } catch (e) {
      setError(String(e))
    }
  }, [])

  const detachEdgeAction = useCallback(async () => {
    if (!attached) return
    try {
      await apiDetachEdge(attached.name)
    } catch (e) {
      setError(String(e))
    }
    setAttached(null)
  }, [attached])

  // ---------------- Run / Stop ----------------

  const run = useCallback(async () => {
    setError(null)
    try {
      // Save the currently-open POU first if it's dirty; the runtime
      // re-reads the project from disk on compile, so unsaved edits would
      // otherwise be silently ignored.
      if (currentApp && source !== currentApp.source) {
        await saveApplication(currentApp.name, source)
        setCurrentApp({ ...currentApp, source })
      }
      await runProgram()
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
        currentEdge,
        attached,
        iomap,
        tasks,
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
        selectEdge,
        openIoMap,
        openTasks,
        saveCurrentApp,
        createApp,
        deleteApp,
        createDevice,
        deleteDevice,
        createAppFolder,
        createDeviceFolder: createDeviceFolderCb,
        createEdge: createEdgeAction,
        deleteEdge: deleteEdgeAction,
        saveEdge,
        createEdgeFolder: createEdgeFolderAction,
        attachEdge: attachEdgeAction,
        detachEdge: detachEdgeAction,
        saveTasks,
        migrateTasks,
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
