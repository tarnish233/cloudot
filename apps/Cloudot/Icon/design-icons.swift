// cloudot 应用图标渲染器。
//
// 构图取自 macosicons 上 @Luca K 的「Smart Backup」：石墨机身 + 蓝色灯带 +
// 前面板绿色 LED + 中央循环箭头。深浅两个变体都在这里重建，用同一套渲染 ——
// 如果一个用原图、一个手画，作为一对自适应图标风格会打架。
//
// 箭头用 SF Symbol arrow.triangle.2.circlepath，正是菜单栏「待同步/同步中」
// 那个符号，应用图标和菜单栏图标因此共用同一套符号语汇。
//
// 用法：swift design-icons.swift   → 输出到 /tmp/cloudot-icon/
import AppKit
import Foundation

// macOS 的图标圆角是超椭圆而不是圆弧，小尺寸下看得出差别
func squircle(in rect: CGRect, n: CGFloat = 5) -> CGPath {
    let path = CGMutablePath()
    let a = rect.width / 2, b = rect.height / 2
    let cx = rect.midX, cy = rect.midY
    for i in 0...720 {
        let t = CGFloat(i) / 720 * 2 * .pi
        let ct = cos(t), st = sin(t)
        let x = cx + a * pow(abs(ct), 2 / n) * (ct < 0 ? -1 : 1)
        let y = cy + b * pow(abs(st), 2 / n) * (st < 0 ? -1 : 1)
        i == 0 ? path.move(to: CGPoint(x: x, y: y)) : path.addLine(to: CGPoint(x: x, y: y))
    }
    path.closeSubpath()
    return path
}

func rgb(_ hex: UInt32) -> NSColor {
    NSColor(srgbRed: CGFloat((hex >> 16) & 0xFF) / 255,
            green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue: CGFloat(hex & 0xFF) / 255, alpha: 1)
}

struct Palette {
    let name: String
    let shellTop: UInt32      // 机身上缘
    let shellBottom: UInt32   // 机身下缘
    let shellGlow: UInt32     // 中央径向高光
    let bandTop: UInt32       // 灯带
    let bandBottom: UInt32
    let faceTop: UInt32       // 前面板
    let faceBottom: UInt32
    let led: UInt32
    let arrow: UInt32
    let arrowGlow: CGFloat    // 箭头外发光强度
    let rimAlpha: CGFloat     // 内描边强度
}

let dark = Palette(name: "dark",
    shellTop: 0x5C5D60, shellBottom: 0x3A3B3D, shellGlow: 0x74757A,
    bandTop: 0x36B4F5, bandBottom: 0x1E8ED8,
    faceTop: 0x4A4B4E, faceBottom: 0x353638,
    led: 0x4CE052, arrow: 0xDCEAF7, arrowGlow: 0.55, rimAlpha: 0.16)

let light = Palette(name: "light",
    shellTop: 0xFAFBFC, shellBottom: 0xDCE0E5, shellGlow: 0xFFFFFF,
    bandTop: 0x2AA3EE, bandBottom: 0x1580CC,
    faceTop: 0xEDEFF2, faceBottom: 0xD6DAE0,
    led: 0x2FBF4F, arrow: 0x1F6FA8, arrowGlow: 0.0, rimAlpha: 0.55)

func render(_ p: Palette, size: CGFloat) -> NSImage {
    let img = NSImage(size: NSSize(width: size, height: size))
    img.lockFocus()
    guard let ctx = NSGraphicsContext.current?.cgContext else { img.unlockFocus(); return img }
    ctx.setShouldAntialias(true)
    let space = CGColorSpaceCreateDeviceRGB()

    // macOS 11+ 图标网格：824/1024 的内框，四周留白给投影
    let inset = size * 0.098
    let body = CGRect(x: inset, y: inset, width: size - inset * 2, height: size - inset * 2)
    let shape = squircle(in: body)

    // 落地投影
    ctx.saveGState()
    ctx.setShadow(offset: CGSize(width: 0, height: -size * 0.014), blur: size * 0.04,
                  color: NSColor.black.withAlphaComponent(p.name == "light" ? 0.22 : 0.38).cgColor)
    ctx.addPath(shape); ctx.setFillColor(NSColor.black.cgColor); ctx.fillPath()
    ctx.restoreGState()

    ctx.saveGState()
    ctx.addPath(shape); ctx.clip()

    // 机身
    let shell = CGGradient(colorsSpace: space,
        colors: [rgb(p.shellTop).cgColor, rgb(p.shellBottom).cgColor] as CFArray,
        locations: [0, 1])!
    ctx.drawLinearGradient(shell, start: CGPoint(x: body.midX, y: body.maxY),
                           end: CGPoint(x: body.midX, y: body.minY), options: [])

    // 中央径向高光 —— 原图的体积感主要来自这个
    let glow = CGGradient(colorsSpace: space,
        colors: [rgb(p.shellGlow).withAlphaComponent(p.name == "light" ? 0.9 : 0.5).cgColor,
                 rgb(p.shellGlow).withAlphaComponent(0).cgColor] as CFArray,
        locations: [0, 1])!
    ctx.drawRadialGradient(glow,
        startCenter: CGPoint(x: body.midX, y: body.minY + body.height * 0.68), startRadius: 0,
        endCenter: CGPoint(x: body.midX, y: body.minY + body.height * 0.68),
        endRadius: body.width * 0.52, options: [])

    // 前面板（下三分之一）
    let faceH = body.height * 0.235
    let faceRect = CGRect(x: body.minX, y: body.minY, width: body.width, height: faceH)
    let face = CGGradient(colorsSpace: space,
        colors: [rgb(p.faceTop).cgColor, rgb(p.faceBottom).cgColor] as CFArray,
        locations: [0, 1])!
    ctx.saveGState(); ctx.clip(to: faceRect)
    ctx.drawLinearGradient(face, start: CGPoint(x: 0, y: faceRect.maxY),
                           end: CGPoint(x: 0, y: faceRect.minY), options: [])
    ctx.restoreGState()

    // 蓝色灯带
    let bandH = body.height * 0.072
    let bandRect = CGRect(x: body.minX, y: faceRect.maxY, width: body.width, height: bandH)
    let band = CGGradient(colorsSpace: space,
        colors: [rgb(p.bandTop).cgColor, rgb(p.bandBottom).cgColor] as CFArray,
        locations: [0, 1])!
    ctx.saveGState(); ctx.clip(to: bandRect)
    ctx.drawLinearGradient(band, start: CGPoint(x: 0, y: bandRect.maxY),
                           end: CGPoint(x: 0, y: bandRect.minY), options: [])
    ctx.restoreGState()

    // 绿色 LED
    let ledW = body.width * 0.062, ledH = body.height * 0.030
    let ledRect = CGRect(x: body.maxX - body.width * 0.235,
                         y: faceRect.minY + faceH * 0.30, width: ledW, height: ledH)
    ctx.saveGState()
    ctx.setShadow(offset: .zero, blur: size * 0.018,
                  color: rgb(p.led).withAlphaComponent(0.85).cgColor)
    ctx.addPath(CGPath(roundedRect: ledRect, cornerWidth: ledH * 0.28,
                       cornerHeight: ledH * 0.28, transform: nil))
    ctx.setFillColor(rgb(p.led).cgColor); ctx.fillPath()
    ctx.restoreGState()

    ctx.restoreGState()  // 解除机身裁剪

    // 内描边
    ctx.saveGState()
    ctx.addPath(shape)
    ctx.setStrokeColor(NSColor.white.withAlphaComponent(p.rimAlpha).cgColor)
    ctx.setLineWidth(size * 0.005)
    ctx.strokePath()
    ctx.restoreGState()

    // 循环箭头 —— 与菜单栏「待同步/同步中」同一个 SF Symbol
    let cfg = NSImage.SymbolConfiguration(pointSize: size * 0.5, weight: .medium)
    if let sym = NSImage(systemSymbolName: "arrow.triangle.2.circlepath",
                         accessibilityDescription: nil)?.withSymbolConfiguration(cfg) {
        let w = body.width * 0.545
        let h = w * (sym.size.height / max(sym.size.width, 1))
        let box = NSRect(x: body.midX - w / 2,
                         y: body.minY + body.height * 0.665 - h / 2, width: w, height: h)
        let tinted = NSImage(size: sym.size, flipped: false) { r in
            rgb(p.arrow).set(); r.fill()
            sym.draw(in: r, from: .zero, operation: .destinationIn, fraction: 1)
            return true
        }
        if p.arrowGlow > 0 {
            NSGraphicsContext.current?.cgContext.setShadow(
                offset: .zero, blur: size * 0.045,
                color: NSColor.white.withAlphaComponent(p.arrowGlow).cgColor)
        }
        tinted.draw(in: box)
    }

    img.unlockFocus()
    return img
}

func write(_ img: NSImage, _ path: String) {
    let tiff = img.tiffRepresentation!
    let png = NSBitmapImageRep(data: tiff)!.representation(using: .png, properties: [:])!
    try? png.write(to: URL(filePath: path))
}

let out = "/tmp/cloudot-icon"
try? FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
for p in [dark, light] {
    write(render(p, size: 1024), "\(out)/AppIcon-\(p.name).png")
    for s in [128, 48, 32, 16] { write(render(p, size: CGFloat(s)), "\(out)/\(p.name)-\(s).png") }
    print("\(p.name) → \(out)/AppIcon-\(p.name).png")
}
