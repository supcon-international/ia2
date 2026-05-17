// AppDelegate — top-level lifecycle. Spin up the backend, wait for
// the URL handshake, open the main window. The supervisor is owned
// here so it lives as long as the app; killing it on terminate is
// the closest thing this app has to "save state on quit."

import AppKit

final class AppDelegate: NSObject, NSApplicationDelegate {
    private let supervisor = BackendSupervisor()
    private var windowController: WindowController?
    private var webViewHost: WebViewHost?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSLog("AppDelegate: applicationDidFinishLaunching")
        installMenuBar()

        do {
            NSLog("AppDelegate: starting supervisor")
            let url = try supervisor.start()
            NSLog("AppDelegate: supervisor returned URL %@", url.absoluteString)
            let host = WebViewHost()
            self.webViewHost = host
            let controller = WindowController(host: host)
            self.windowController = controller
            NSLog("AppDelegate: window+host created, calling load")
            host.load(url: url)
        } catch {
            NSLog("AppDelegate: supervisor failed: %@", String(describing: error))
            presentFatalAlert(
                title: "Couldn't start backend",
                detail: String(describing: error)
            )
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(
        _ sender: NSApplication
    ) -> Bool {
        // Conventional Mac apps stay running with no windows; for a
        // single-window IDE that's surprising. Mirror VS Code /
        // Xcode behaviour: closing the workbench window terminates.
        return true
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
