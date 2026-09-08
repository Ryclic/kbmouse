// macOS SVG export helper. Usage: swift scripts/render-logo.swift INPUT.svg OUTPUT.png SIZE
import AppKit
guard CommandLine.arguments.count == 4,
      let requestedSize = Int(CommandLine.arguments[3]), requestedSize > 0 else {
    fatalError("Usage: swift scripts/render-logo.swift INPUT.svg OUTPUT.png SIZE")
}
let source = CommandLine.arguments[1]
let output = CommandLine.arguments[2]
let size = requestedSize
guard let image = NSImage(contentsOfFile: source) else { fatalError("Cannot load SVG") }
let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
image.draw(in: NSRect(x: 0, y: 0, width: size, height: size))
NSGraphicsContext.restoreGraphicsState()
try bitmap.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: output))
