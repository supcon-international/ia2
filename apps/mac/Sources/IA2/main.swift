// Entry point. AppKit boilerplate: instantiate NSApplication, wire up
// our AppDelegate, run the event loop. Everything interesting happens
// in AppDelegate / BackendSupervisor / WindowController.

import AppKit
import Darwin

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
// .regular = full-fledged app with Dock icon and menu bar. The
// alternative, .accessory, hides from Dock — appropriate for tray-
// only utilities, which we are not.
app.setActivationPolicy(.regular)
// NB: activate() is a no-op until `app.run()` is called and the run
// loop is processing events. Don't call it here — the actual
// activation happens inside WindowController.onFirstPaint where the
// window also orderFronts.

// AppKit only calls `applicationWillTerminate` when the user quits
// through the UI (menu, Cmd-Q, Dock right-click → Quit). A bare
// SIGTERM from the shell or system shutdown bypasses that path and
// would orphan the Rust subprocess. Install POSIX signal handlers
// that mirror what NSApplication.terminate(_:) does, so any way the
// app dies cleans up the backend.
//
// We use GCD signal sources because POSIX signal handlers can only
// safely call async-signal-safe functions; GCD bridges to the main
// queue where it's safe to invoke arbitrary code (i.e. our
// `supervisor.stop()` chain via `NSApp.terminate(nil)`).

private let installSignalHandlers: () = {
    for sig in [SIGTERM, SIGINT, SIGHUP] {
        // Need to ignore default disposition first so signal() doesn't
        // pre-empt our handler before GCD's source fires.
        signal(sig, SIG_IGN)
        let src = DispatchSource.makeSignalSource(signal: sig, queue: .main)
        src.setEventHandler {
            NSLog("ia2: caught signal \(sig), terminating")
            // Try the polite path first — NSApp.terminate invokes
            // applicationShouldTerminate, applicationWillTerminate,
            // window close handlers etc. Then, AFTER a short grace
            // period, fall back to exit() so we always die. Without
            // this fallback, NSApp.terminate sometimes silently no-
            // ops for processes launched via `open` (the AppKit
            // termination path expects to be driven from the menu or
            // from a Cmd-Q dispatch, not from a signal handler under
            // launchd supervision). The Rust subprocess's stdin-EOF
            // watchdog will reap it when our pipes close on exit.
            NSApp.terminate(nil)
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
                NSLog("ia2: terminate didn't take, exit(0) fallback")
                exit(0)
            }
        }
        src.resume()
        // Keep the source alive for the process lifetime by leaking
        // it on purpose — there's exactly one and it should never be
        // cancelled.
        _ = Unmanaged.passRetained(src as AnyObject)
    }
}()

app.run()
