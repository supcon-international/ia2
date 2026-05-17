// make-icon.swift — render the .icns app icon from code, no .xcassets.
//
// Run:
//   swift apps/mac/Resources/make-icon.swift apps/mac/build/AppIcon.iconset
//   iconutil -c icns apps/mac/build/AppIcon.iconset -o apps/mac/build/AppIcon.icns
//
// build.sh handles this automatically.
//
// Design rationale:
//   The product mark is the wordmark "IA2" on a brand-green
//   squircle. The function-block motif from the v0 icon ties to the
//   FBD editor specifically; the wordmark generalizes — it identifies
//   the whole product, including future surfaces (CLI banner, web
//   footer, docs). One bold geometric form scales cleanly from
//   1024 px down to 32 px (16 px is read by the OS at lower fidelity
//   in modern macOS Docks, so we let it blur into a coloured tile).

import AppKit
import CoreGraphics
import Foundation

let outDir = CommandLine.arguments.count > 1
    ? CommandLine.arguments[1]
    : "./AppIcon.iconset"

// macOS .icns expects this exact filename set; iconutil rejects extras
// or omissions. (`size@2x.png` is "size at retina" — physical pixels
// are size × 2.)
let sizes: [(name: String, px: Int)] = [
    ("icon_16x16.png", 16),
    ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32),
    ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128),
    ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256),
    ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512),
    ("icon_512x512@2x.png", 1024),
]

/// Brand colours. The green is roughly the sRGB rendering of the
/// IDE's `--highlight: oklch(0.7 0.17 152)` from
/// apps/web/src/styles.css. Keep them in sync — when the IDE
/// re-tunes the highlight hue, this should follow.
let brandGreen = NSColor(srgbRed: 0.36, green: 0.71, blue: 0.42, alpha: 1.0)
let inkWhite = NSColor.white

/// Pick the bundled font that gives us the best heavy sans-serif at
/// any rendered size. SF Pro Display Black > SF Pro Display Bold >
/// system default. Falls back gracefully on older macOS.
func brandFont(size: CGFloat) -> NSFont {
    if let f = NSFont(name: "SF Pro Display", size: size) {
        // SF Pro Display Black weight via descriptor.
        let descriptor = f.fontDescriptor.addingAttributes([
            .traits: [
                NSFontDescriptor.TraitKey.weight: NSFont.Weight.black,
            ]
        ])
        if let bold = NSFont(descriptor: descriptor, size: size) {
            return bold
        }
        return f
    }
    return NSFont.systemFont(ofSize: size, weight: .black)
}

func renderIcon(px: Int) -> Data {
    let size = CGFloat(px)
    let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil,
        pixelsWide: px,
        pixelsHigh: px,
        bitsPerSample: 8,
        samplesPerPixel: 4,
        hasAlpha: true,
        isPlanar: false,
        colorSpaceName: .deviceRGB,
        bytesPerRow: 0,
        bitsPerPixel: 0
    )!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)

    let ctx = NSGraphicsContext.current!.cgContext
    ctx.setAllowsAntialiasing(true)
    ctx.setShouldAntialias(true)

    // === Squircle background ===
    //
    // Core Graphics has no native squircle, only rounded rectangles.
    // 22.4% corner radius is the well-known approximation of the
    // continuous corner Apple uses on macOS Big Sur+ icons.
    let inset = size * 0.06
    let bg = CGRect(x: inset, y: inset, width: size - 2 * inset, height: size - 2 * inset)
    let radius = bg.width * 0.224

    let bgPath = CGPath(roundedRect: bg, cornerWidth: radius, cornerHeight: radius, transform: nil)
    ctx.addPath(bgPath)
    ctx.setFillColor(brandGreen.cgColor)
    ctx.fillPath()

    // === Wordmark "IA2" ===
    //
    // Approach: pick a font size that fills ~58% of the canvas
    // height in cap-height. We measure the rendered bounds and
    // center within the background rect, using cap-height (not
    // line-height) as the vertical reference so the optical center
    // is correct — string drawing centers by line box by default,
    // which floats text high.
    let text: NSString = "IA2"
    let fontSize = bg.height * 0.62
    let font = brandFont(size: fontSize)
    let paragraph = NSMutableParagraphStyle()
    paragraph.alignment = .center
    paragraph.lineBreakMode = .byClipping

    let attrs: [NSAttributedString.Key: Any] = [
        .font: font,
        .foregroundColor: inkWhite,
        .paragraphStyle: paragraph,
        // Tighten the tracking a hair — system "Black" weights have
        // generous defaults that look airy at small sizes. Negative
        // is fine for ALL-CAPS short marks.
        .kern: -fontSize * 0.04,
    ]

    let measured = text.size(withAttributes: attrs)
    // capHeight is the *visual* top → baseline distance for caps.
    // It's what we want to centre, not the full ascender+descender
    // metric (which would push caps up).
    let visualHeight = font.capHeight
    let yShift = (font.ascender - visualHeight) / 2  // re-centre cap
    let drawRect = CGRect(
        x: bg.midX - measured.width / 2,
        y: bg.midY - visualHeight / 2 - yShift,
        width: measured.width,
        height: measured.height
    )

    text.draw(in: drawRect, withAttributes: attrs)

    NSGraphicsContext.restoreGraphicsState()

    return rep.representation(using: .png, properties: [:])!
}

let fm = FileManager.default
try? fm.createDirectory(atPath: outDir, withIntermediateDirectories: true)

for (name, px) in sizes {
    let data = renderIcon(px: px)
    let path = "\(outDir)/\(name)"
    fm.createFile(atPath: path, contents: data)
    print("wrote \(name) \(px)px")
}
