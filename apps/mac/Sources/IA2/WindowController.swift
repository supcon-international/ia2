// WindowController — wires the WebViewHost into an NSWindow with the
// native materials and chrome we want. One window for now (the IDE
// workbench); future windows (settings, theme studio) get their own
// controllers + entry points.
//
// Survival-guide items implemented here:
//   A.1 — disable WebKit occlusion throttling so a hidden / occluded
//         window doesn't drop animation rate.
//   A.5 — translucent backing material via NSVisualEffectView.
//   A.4 not implemented yet — we don't animate window size today.

import AppKit
import WebKit

/// Persisted-window-frame autosave name. NSWindow uses this as the key
/// in `defaults` for `setFrame:autosaveName:` — picking a stable string
/// gives users "the window opens where I left it" across launches.
private let kFrameAutosaveName = "IA2MainWindow"

final class WindowController: NSWindowController {
    private let webViewHost: WebViewHost
    private var firstPaintHandled = false
    /// Tracks the most recently observed effective appearance. When
    /// macOS dark-mode toggles, the WebView's `prefers-color-scheme`
    /// updates automatically, but the React app reads localStorage —
    /// so we forward the change as a custom JS event the web side can
    /// (optionally) listen to.
    private var appearanceObservation: NSKeyValueObservation?

    init(host: WebViewHost) {
        NSLog("WindowController: init begin")
        self.webViewHost = host

        // Native-feel pass: take over the titlebar.
        //   .fullSizeContentView — the WebView extends *under* the
        //   titlebar all the way to the top of the window.
        //   titlebarAppearsTransparent — no separate titlebar color;
        //   the page's `bg-background` shows through.
        //   titleVisibility = .hidden — no centered window-title text
        //   competing with whatever the React side puts up there.
        // The traffic lights (close / min / zoom) stay because the
        // window is still `.titled`. Together this removes the
        // high-contrast seam between OS chrome and web content that
        // people noticed on dark mode.
        let rect = NSRect(x: 0, y: 0, width: 1280, height: 800)
        let style: NSWindow.StyleMask = [
            .titled, .closable, .miniaturizable, .resizable, .fullSizeContentView,
        ]
        let window = NSWindow(
            contentRect: rect,
            styleMask: style,
            backing: .buffered,
            defer: false
        )
        window.title = "IA2"
        window.titleVisibility = .hidden
        window.titlebarAppearsTransparent = true
        // Restore the user's last-known window frame. Off-screen
        // recovery (in case they unplugged a monitor between launches)
        // happens in the first-paint hook below, after we know which
        // screens are actually attached now.
        let restored = window.setFrameUsingName(kFrameAutosaveName)
        window.setFrameAutosaveName(kFrameAutosaveName)
        if !restored {
            // First-ever launch: center on the main screen.
            if let screen = NSScreen.main {
                let f = screen.visibleFrame
                let originX = f.origin.x + (f.size.width - rect.size.width) / 2
                let originY = f.origin.y + (f.size.height - rect.size.height) / 2
                window.setFrame(
                    NSRect(x: originX, y: originY, width: rect.size.width, height: rect.size.height),
                    display: true
                )
            } else {
                window.center()
            }
        }
        // Make the window appear on whichever Space the user is on,
        // rather than its "home" Space — otherwise launching while
        // on a different Space puts the window out of sight.
        window.collectionBehavior = [.moveToActiveSpace]

        // A.1 — turn off WebKit's "the window is occluded so I'll
        // throttle the WebView" heuristic. KVC-only and undocumented;
        // KVC-uncompliant if the key disappears between macOS
        // versions, so guard with try/catch via Objective-C
        // exception bridge.
        NSLog("WindowController: about to set occlusion off")
        let kvcKey = "windowOcclusionDetectionEnabled"
        if (window as NSObject).responds(to: Selector((kvcKey))) {
            window.setValue(false, forKey: kvcKey)
            NSLog("WindowController: occlusion off")
        } else {
            NSLog("WindowController: occlusion key absent, skipping")
        }

        super.init(window: window)

        // For v0 keep it simple: WebView is the entire content view,
        // no NSVisualEffectView underlay. The React app already
        // paints an opaque background. The vibrancy material is a
        // polish item — turning it on requires careful CSS work on
        // the React side (body { background: transparent } plus
        // sidebar tints that look right against system blur), and
        // shipping it without that work yields a window whose
        // content is half-readable.
        webViewHost.webView.frame = window.contentView?.bounds ?? .zero
        webViewHost.webView.autoresizingMask = [.width, .height]
        webViewHost.webView.translatesAutoresizingMaskIntoConstraints = true
        window.contentView = webViewHost.webView

        // Title KVO bridge — when the page sets `document.title`,
        // forward to the window menu / Dock label.
        webViewHost.onTitleChange = { [weak window] title in
            window?.title = title
        }

        // A.2 — gate orderFront on first paint to avoid the white
        // flash. Until then the window stays hidden.
        webViewHost.onFirstPaint = { [weak self] in
            guard let self, !self.firstPaintHandled, let window = self.window else {
                return
            }
            self.firstPaintHandled = true
            NSLog(
                "WindowController: first paint; frame=%@ visible=%d screen=%@",
                NSStringFromRect(window.frame),
                window.isVisible ? 1 : 0,
                window.screen?.localizedName ?? "<no screen>"
            )

            // If the autosaved frame put the window entirely off
            // every screen (happens after monitor unplug, or first
            // run before center() takes), recenter on the main
            // screen.
            let screens = NSScreen.screens
            let onAnyScreen = screens.contains {
                $0.visibleFrame.intersects(window.frame)
            }
            if !onAnyScreen {
                NSLog("WindowController: frame is off-screen, recentering")
                window.center()
            }

            // Bring to absolute front + activate the app. Also bounce
            // the Dock icon so the user has somewhere to look if the
            // window itself is hard to spot.
            window.makeKeyAndOrderFront(nil)
            window.orderFrontRegardless()
            NSApp.activate(ignoringOtherApps: true)
            NSApp.requestUserAttention(.criticalRequest)
            NSLog(
                "WindowController: shown; frame=%@ visible=%d screen=%@",
                NSStringFromRect(window.frame),
                window.isVisible ? 1 : 0,
                window.screen?.localizedName ?? "<no screen>"
            )
        }

        // Forward macOS dark-mode toggles into the WebView. WebKit
        // already updates `prefers-color-scheme` automatically, but
        // the React app reads localStorage — so we dispatch a custom
        // event the web side can opt into to mirror the OS choice.
        // Observation is on the NSApp, not the window, because system
        // appearance is process-wide.
        appearanceObservation = NSApp.observe(\.effectiveAppearance, options: [.new]) {
            [weak self] _, change in
            guard let self, let appearance = change.newValue else { return }
            let isDark = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            DispatchQueue.main.async {
                let js = "window.dispatchEvent(new CustomEvent('ia2:os-appearance', { detail: { dark: \(isDark) } }))"
                self.webViewHost.webView.evaluateJavaScript(js, completionHandler: nil)
            }
        }
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) not implemented")
    }
}
