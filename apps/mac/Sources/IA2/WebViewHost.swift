// WebViewHost — owns the WKWebView and applies the survival-guide
// fixes from references/03-webview-survival.md. Without these, the
// WebView feels like a browser embed, which is exactly the "feels
// like a web app" tell the whole architecture is set up to avoid.
//
// Each suppression / private-API call is documented inline with the
// section reference. If anything looks magical, that file is where to
// look first.

import AppKit
import WebKit

final class WebViewHost: NSObject {
    let webView: WKWebView
    private var titleObservation: NSKeyValueObservation?

    /// Called once the WebView reports its first paint, so the host
    /// window can `orderFront` without showing an empty backing
    /// surface (cf. survival guide A.2 — startup flicker).
    var onFirstPaint: (() -> Void)?

    /// Called whenever the WebView's `title` property changes, so the
    /// host window can update `NSWindow.title`. The HTML `<title>` is
    /// the source of truth for the window title — keeps brand updates
    /// in one place.
    var onTitleChange: ((String) -> Void)?

    override init() {
        NSLog("WebViewHost: init begin")
        let config = WKWebViewConfiguration()

        if #available(macOS 13.3, *) {
            config.preferences.isElementFullscreenEnabled = false
        }
        config.preferences.setValue(true, forKey: "developerExtrasEnabled")
        NSLog("WebViewHost: prefs basic done")

        // RequestIdleCallback is off by default in WKWebView; flip
        // it via the private prefs API. Wrap in catch-all because the
        // key/selector are private and can disappear across macOS
        // versions.
        let prefsAny: AnyObject = config.preferences
        let setBoolSel = Selector(("_setBoolValue:forKey:"))
        if prefsAny.responds(to: setBoolSel) {
            _ = prefsAny.perform(
                setBoolSel,
                with: NSNumber(value: true),
                with: "RequestIdleCallbackEnabled" as NSString
            )
            NSLog("WebViewHost: idle callback flag set")
        } else {
            NSLog("WebViewHost: private prefs API unavailable, skipping")
        }

        webView = CustomWebView(frame: .zero, configuration: config)
        NSLog("WebViewHost: WKWebView constructed")
        super.init()

        webView.translatesAutoresizingMaskIntoConstraints = false
        webView.navigationDelegate = self
        webView.uiDelegate = self
        NSLog("WebViewHost: delegates set")

        webView.wantsLayer = true
        // Disable the WebView's own opaque background so the
        // NSVisualEffectView underlay we install in WindowController
        // can show through wherever the page leaves a transparent gap
        // (right now: the 28px titlebar safe area at the top of
        // Workbench). The rest of the page sets `bg-background`
        // explicitly so vibrancy only leaks in where we want it.
        //
        // `drawsBackground` is a private KVC key on WKWebView (the
        // public API didn't ship until macOS 13.3 as a non-private
        // property). The KVC path has been stable since macOS 10.10
        // — guarded with responds(to:) so a future WebKit shuffle
        // can't crash us.
        let drawsBackgroundKey = "drawsBackground"
        if (webView as NSObject).responds(to: Selector((drawsBackgroundKey))) {
            webView.setValue(false, forKey: drawsBackgroundKey)
            NSLog("WebViewHost: drawsBackground = false (vibrancy enabled)")
        } else {
            NSLog("WebViewHost: drawsBackground key absent; titlebar vibrancy disabled")
        }

        // Forward console.log from the page to Console.app via a
        // userContentController message handler. Cheap way to debug
        // why React might not be rendering.
        let userContent = config.userContentController
        userContent.add(ConsoleBridge(), name: "console")
        let consoleJS = """
            (function() {
              const orig = { log: console.log, warn: console.warn, error: console.error };
              for (const k of ['log','warn','error']) {
                console[k] = function(...args) {
                  orig[k].apply(console, args);
                  try {
                    window.webkit.messageHandlers.console.postMessage(
                      k + ': ' + args.map(a => String(a)).join(' ')
                    );
                  } catch(e) {}
                };
              }
              window.addEventListener('error', e => {
                window.webkit.messageHandlers.console.postMessage(
                  'uncaught: ' + e.message + ' @ ' + e.filename + ':' + e.lineno
                );
              });
            })();
            """
        userContent.addUserScript(
            WKUserScript(
                source: consoleJS,
                injectionTime: .atDocumentStart,
                forMainFrameOnly: true
            )
        )

        // Observe `<title>` so the window title tracks the HTML.
        titleObservation = webView.observe(\.title, options: [.new]) {
            [weak self] _, change in
            guard let title = change.newValue ?? nil, !title.isEmpty else { return }
            DispatchQueue.main.async {
                self?.onTitleChange?(title)
            }
        }
        NSLog("WebViewHost: init done")
    }

    /// Load the given URL. We use `_doAfterNextPresentationUpdate:`
    /// (A.2) inside the navigation delegate to synchronize first-paint
    /// with the window's orderFront so the user never sees a white
    /// frame.
    func load(url: URL) {
        NSLog("WebView load(\(url.absoluteString))")
        webView.load(URLRequest(url: url))
    }
}

extension WebViewHost: WKNavigationDelegate {
    func webView(
        _ webView: WKWebView,
        didStartProvisionalNavigation navigation: WKNavigation!
    ) {
        NSLog("WebView didStart")
    }

    func webView(_ webView: WKWebView, didCommit navigation: WKNavigation!) {
        NSLog("WebView didCommit")
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        NSLog("WebView didFinish navigation: %@",
              webView.url?.absoluteString ?? "<no url>")
        // Run after the next presentation update so the window
        // orderFront happens AFTER the WebView has actually painted
        // its first frame, not just decided what to paint.
        let cb: @convention(block) () -> Void = { [weak self] in
            NSLog("WebView _doAfterNextPresentationUpdate fired")
            DispatchQueue.main.async { self?.onFirstPaint?() }
        }
        let selector = Selector(("_doAfterNextPresentationUpdate:"))
        if webView.responds(to: selector) {
            webView.perform(selector, with: cb)
        } else {
            NSLog("WebView does not respond to _doAfterNextPresentationUpdate, falling back")
            DispatchQueue.main.async { [weak self] in self?.onFirstPaint?() }
        }
        // Belt-and-suspenders: if the private API silently no-ops (the
        // selector EXISTS but the callback never fires — observed on
        // some macOS builds when the page is HTTPS-with-warnings, or
        // when WebKit is mid-process-restart), fall through to a
        // deadline-based show. 800 ms is long enough that the normal
        // path beats it in practice but short enough that a stuck
        // private API doesn't strand the window forever.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.8) { [weak self] in
            NSLog("WebView fallback timer firing")
            self?.onFirstPaint?()
        }
    }

    func webView(
        _ webView: WKWebView,
        didFailProvisionalNavigation navigation: WKNavigation!,
        withError error: Error
    ) {
        NSLog("WebView failed provisional navigation: \(error.localizedDescription)")
        // If the server died mid-load, surface SOMETHING to the user
        // rather than an indefinitely-blank window. The supervisor
        // owns full lifecycle so it should already be writing to
        // stderr; this is a last-resort signal.
        let html = """
            <html><body style='font:13px -apple-system;color:#444;padding:32px'>
            <h3>Backend unreachable</h3>
            <p>The IA2 backend exited before the UI could connect.</p>
            <p style='color:#888'>Check Console.app for details, then relaunch.</p>
            </body></html>
            """
        webView.loadHTMLString(html, baseURL: nil)
    }
}

extension WebViewHost: WKUIDelegate {
    // Open `target="_blank"` links in the user's default browser
    // (Safari), not as new WebView windows.
    func webView(
        _ webView: WKWebView,
        createWebViewWith configuration: WKWebViewConfiguration,
        for navigationAction: WKNavigationAction,
        windowFeatures: WKWindowFeatures
    ) -> WKWebView? {
        if let url = navigationAction.request.url {
            NSWorkspace.shared.open(url)
        }
        return nil
    }
}

/// A.6 — filter WebKit's browser context menu down to the editing
/// affordances a Mac user expects: Cut / Copy / Paste / Select All
/// and "Look Up" on selected text. Browser-only items (Reload,
/// "Open in Safari", "View Page Source", Inspect Element on release
/// builds, etc.) are stripped. The previous version removed everything
/// — which broke text fields and selectable surfaces (Monitor pane
/// values, output logs) because users couldn't right-click → Copy.
///
/// We identify items by their `WKMenuItemIdentifier` so this survives
/// Apple's occasional menu reshuffling. Anything without a stable
/// identifier (legacy items) is dropped to keep the menu uncluttered.
private final class CustomWebView: WKWebView {
    /// Identifiers we keep. WKMenuItemIdentifier values are stable
    /// across macOS versions; the raw strings are documented in the
    /// WebKit headers (cf. `WKMenuItemIdentifiers.h`).
    private static let allowedIdentifiers: Set<String> = [
        "WKMenuItemIdentifierCut",
        "WKMenuItemIdentifierCopy",
        "WKMenuItemIdentifierPaste",
        "WKMenuItemIdentifierSelectAll",
        "WKMenuItemIdentifierLookUp",
        "WKMenuItemIdentifierTranslate",
        // Image / link items the user explicitly initiated by right-
        // clicking a specific element. Less noisy than the full set.
        "WKMenuItemIdentifierCopyLink",
        "WKMenuItemIdentifierCopyImage",
    ]

    override func willOpenMenu(_ menu: NSMenu, with event: NSEvent) {
        // Two passes: collect items to keep (matched by identifier),
        // then rebuild the menu from those. This avoids index-shift
        // bugs from removing items while iterating.
        let keep = menu.items.filter { item in
            guard let id = item.identifier?.rawValue else { return false }
            return Self.allowedIdentifiers.contains(id)
        }
        menu.removeAllItems()
        for item in keep {
            menu.addItem(item)
        }
        // If we end up with nothing (right-click on empty surface),
        // suppress the menu entirely rather than show a blank popup.
        if menu.items.isEmpty {
            menu.cancelTracking()
        }
    }
}

/// Bridge for `window.webkit.messageHandlers.console.postMessage(...)`
/// from the injected userscript above. Routes browser console output
/// into our own NSLog stream so we can see what React is doing without
/// opening the Web Inspector.
private final class ConsoleBridge: NSObject, WKScriptMessageHandler {
    func userContentController(
        _ uc: WKUserContentController,
        didReceive msg: WKScriptMessage
    ) {
        if let s = msg.body as? String {
            NSLog("WebViewConsole: %@", s)
        }
    }
}
