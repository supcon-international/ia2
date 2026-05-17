// swift-tools-version:5.9
//
// Mac shell for IA2. Single executable that spawns the bundled
// `ia2-server` binary (Rust, built separately by cargo), reads the
// localhost URL it prints on stdout, then hosts a WKWebView pointing
// at that URL inside an AppKit window.
//
// We do not vendor an Xcode project — `swift build` + a thin
// `build.sh` is enough to produce a `.app` bundle and run it. Keeps
// the source tree minimal and makes the build reproducible on CI
// without needing Xcode installed (only the command-line tools).

import PackageDescription

let package = Package(
    name: "IA2",
    platforms: [
        // 13.0 is the WKWebView floor that gives us all the survival-
        // guide private APIs (`_doAfterNextPresentationUpdate:`,
        // `_setBoolValue:forKey:`, occlusion detection toggle). Bump
        // to 26 when we want to opt in to `NSGlassEffectView` for the
        // Tahoe / Liquid Glass material; for now, fall back to
        // `NSVisualEffectView` which works everywhere ≥ 10.10.
        .macOS(.v13),
    ],
    targets: [
        .executableTarget(
            name: "IA2",
            path: "Sources/IA2"
        ),
    ]
)
