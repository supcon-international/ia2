// AppDelegate — top-level lifecycle. Spin up the backend, wait for
// the URL handshake, open the main window. The supervisor is owned
// here so it lives as long as the app; killing it on terminate is
// the closest thing this app has to "save state on quit."

import AppKit

final class AppDelegate: NSObject, NSApplicationDelegate {
    private let supervisor = BackendSupervisor()
    /// Live windows. Each one wraps its own `WebViewHost` so they
    /// don't share JavaScript / DOM state — the only thing in common
    /// is the URL origin (one server, many windows).
    private var windows: [WindowController] = []
    /// URL of the running backend server, captured at startup so
    /// Cmd+N and other "open new window" paths can spawn a window
    /// without re-running the supervisor.
    private var serverURL: URL?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSLog("AppDelegate: applicationDidFinishLaunching")
        installMenuBar()

        do {
            NSLog("AppDelegate: starting supervisor")
            let url = try supervisor.start()
            NSLog("AppDelegate: supervisor returned URL %@", url.absoluteString)
            self.serverURL = url
            spawnWindow(url: url)
        } catch {
            NSLog("AppDelegate: supervisor failed: %@", String(describing: error))
            presentFatalAlert(
                title: "Couldn't start backend",
                detail: String(describing: error)
            )
        }
    }

    /// Open a new IDE window pointing at `url`. The web side reads
    /// the URL's `?project=<name>` search param to decide which
    /// project this window owns; if absent, the server-side active
    /// fallback takes over. Called from both the initial launch and
    /// the picker's "Open in new window" flow.
    func spawnWindow(url: URL) {
        let host = WebViewHost()
        // Same-origin `window.open()` from the web side spawns a new
        // IA2 window via this callback. Cross-origin opens are still
        // routed to the user's default browser (see WebViewHost's
        // WKUIDelegate). Capture `self` weakly to avoid retain cycles.
        host.onSameOriginOpen = { [weak self] target in
            self?.spawnWindow(url: target)
        }
        let controller = WindowController(host: host)
        windows.append(controller)
        // Drop the controller from our registry when the window
        // actually closes. We listen to NSWindow's willClose because
        // NSWindowController owns the window's lifecycle but doesn't
        // expose a "did close" hook itself.
        if let window = controller.window {
            NotificationCenter.default.addObserver(
                forName: NSWindow.willCloseNotification,
                object: window,
                queue: .main
            ) { [weak self] _ in
                self?.windows.removeAll { $0 === controller }
            }
        }
        host.load(url: url)
    }

    /// "File → New Window" — opens a fresh window pinned to the same
    /// server but starting with no `?project=` filter, so the picker
    /// prompts the user to choose.
    @objc func newWindowAction(_ sender: Any?) {
        guard let url = serverURL else {
            NSLog("AppDelegate: new-window requested before server URL ready")
            return
        }
        // Strip any query string — fresh window starts at the picker.
        var base = url
        if var comps = URLComponents(url: url, resolvingAgainstBaseURL: false) {
            comps.query = nil
            if let stripped = comps.url {
                base = stripped
            }
        }
        spawnWindow(url: base)
    }

    func applicationShouldTerminateAfterLastWindowClosed(
        _ sender: NSApplication
    ) -> Bool {
        // Conventional Mac apps stay running with no windows; for a
        // single-window IDE that's surprising. Mirror VS Code /
        // Xcode behaviour: closing the LAST workbench window
        // terminates. Earlier windows can close independently.
        return windows.isEmpty
    }

    func applicationWillTerminate(_ notification: Notification) {
        supervisor.stop()
    }

    // MARK: - Menu bar
    //
    // Minimal but native: App / File / Edit / View / Window / Help
    // with the standard shortcuts. The React UI handles its own
    // commands; this menu exists for muscle memory only (T7) — Cmd+Q,
    // Cmd+W, Cmd+M, copy/paste in text fields, etc.

    private func installMenuBar() {
        let main = NSMenu()

        let appMenu = NSMenu()
        appMenu.addItem(
            NSMenuItem(
                title: "About IA2",
                action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)),
                keyEquivalent: ""
            )
        )
        appMenu.addItem(NSMenuItem.separator())
        appMenu.addItem(
            NSMenuItem(
                title: "Hide IA2",
                action: #selector(NSApplication.hide(_:)),
                keyEquivalent: "h"
            )
        )
        let hideOthers = NSMenuItem(
            title: "Hide Others",
            action: #selector(NSApplication.hideOtherApplications(_:)),
            keyEquivalent: "h"
        )
        hideOthers.keyEquivalentModifierMask = [.command, .option]
        appMenu.addItem(hideOthers)
        appMenu.addItem(
            NSMenuItem(
                title: "Show All",
                action: #selector(NSApplication.unhideAllApplications(_:)),
                keyEquivalent: ""
            )
        )
        appMenu.addItem(NSMenuItem.separator())
        appMenu.addItem(
            NSMenuItem(
                title: "Quit IA2",
                action: #selector(NSApplication.terminate(_:)),
                keyEquivalent: "q"
            )
        )
        let appItem = NSMenuItem()
        appItem.submenu = appMenu
        main.addItem(appItem)

        // File menu — currently only "New Window" so users can spawn
        // a second workbench window via Cmd+N (mirrors Safari /
        // Chrome / Linear / Xcode). Each window then picks its own
        // project from the picker in the title bar.
        let fileMenu = NSMenu(title: "File")
        let newWindow = NSMenuItem(
            title: "New Window",
            action: #selector(AppDelegate.newWindowAction(_:)),
            keyEquivalent: "n"
        )
        newWindow.target = self
        fileMenu.addItem(newWindow)
        fileMenu.addItem(NSMenuItem.separator())
        // Close Window — wired to the standard window close action so
        // it inherits the normal NSWindow lifecycle (which is what
        // our willClose observer in `spawnWindow` listens for).
        fileMenu.addItem(
            NSMenuItem(
                title: "Close Window",
                action: #selector(NSWindow.performClose(_:)),
                keyEquivalent: "w"
            )
        )
        let fileItem = NSMenuItem()
        fileItem.submenu = fileMenu
        main.addItem(fileItem)

        // Edit menu — gives us system Cut/Copy/Paste/Undo/Select-All
        // that the WebView's native input fields rely on.
        let editMenu = NSMenu(title: "Edit")
        editMenu.addItem(
            NSMenuItem(
                title: "Undo",
                action: Selector(("undo:")),
                keyEquivalent: "z"
            )
        )
        let redo = NSMenuItem(
            title: "Redo",
            action: Selector(("redo:")),
            keyEquivalent: "z"
        )
        redo.keyEquivalentModifierMask = [.command, .shift]
        editMenu.addItem(redo)
        editMenu.addItem(NSMenuItem.separator())
        editMenu.addItem(
            NSMenuItem(
                title: "Cut",
                action: #selector(NSText.cut(_:)),
                keyEquivalent: "x"
            )
        )
        editMenu.addItem(
            NSMenuItem(
                title: "Copy",
                action: #selector(NSText.copy(_:)),
                keyEquivalent: "c"
            )
        )
        editMenu.addItem(
            NSMenuItem(
                title: "Paste",
                action: #selector(NSText.paste(_:)),
                keyEquivalent: "v"
            )
        )
        editMenu.addItem(
            NSMenuItem(
                title: "Select All",
                action: #selector(NSText.selectAll(_:)),
                keyEquivalent: "a"
            )
        )
        let editItem = NSMenuItem()
        editItem.submenu = editMenu
        main.addItem(editItem)

        // Window menu — system Minimize / Zoom.
        let windowMenu = NSMenu(title: "Window")
        windowMenu.addItem(
            NSMenuItem(
                title: "Minimize",
                action: #selector(NSWindow.performMiniaturize(_:)),
                keyEquivalent: "m"
            )
        )
        windowMenu.addItem(
            NSMenuItem(
                title: "Zoom",
                action: #selector(NSWindow.performZoom(_:)),
                keyEquivalent: ""
            )
        )
        let windowItem = NSMenuItem()
        windowItem.submenu = windowMenu
        main.addItem(windowItem)
        NSApp.windowsMenu = windowMenu

        NSApp.mainMenu = main
    }

    private func presentFatalAlert(title: String, detail: String) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = detail
        alert.alertStyle = .critical
        alert.addButton(withTitle: "Quit")
        alert.runModal()
        NSApplication.shared.terminate(nil)
    }
}
