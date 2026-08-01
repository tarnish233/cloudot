// cloudot 应用图标 —— 原创设计，不派生自任何第三方素材。
//
// 意象：cloudot 的本质是「软链」—— 家目录里的配置指向一个共享的 store，
// 多台机器读同一份内容。所以画两个节点由一条链路相连，而不是画一台备份机。
//
// 构图：
//   · 圆角方底（macOS 的超椭圆，n=5），系统蓝到青的对角渐变
//   · 中央一条环形链路，两端各一个节点 —— 一台机器、一份共享配置
//   · 链路留一个缺口并用箭头收尾，表示单向落地（store → 本机）
//
// 全部用 CoreGraphics 画，没有外部素材，因此授权干净。
//
// 用法：swift make-icon.swift   → 输出 AppIcon.png（1024×1024）
import AppKit
import Foundation

/// macOS 的图标圆角是超椭圆而不是圆弧，小尺寸下看得出差别。
func squircle(in rect: CGRect, n: CGFloat = 5) -> CGPath {
    let path = CGMutablePath()
    let a = rect.width / 2, b = rect.height / 2
    let cx = rect.midX, cy = rect.midY
    let steps = 720
    for i in 0...steps {
        let t = CGFloat(i) / CGFloat(steps) * 2 * .pi
        let ct = cos(t), st = sin(t)
        // 超椭圆参数式：|x/a|^n + |y/b|^n = 1
        let x = cx + a * pow(abs(ct), 2 / n) * (ct < 0 ? -1 : 1)
        let y = cy + b * pow(abs(st), 2 / n) * (st < 0 ? -1 : 1)
        if i == 0 { path.move(to: CGPoint(x: x, y: y)) } else { path.addLine(to: CGPoint(x: x, y: y)) }
    }
    path.closeSubpath()
    return path
}

func rgb(_ hex: UInt32, _ alpha: CGFloat = 1) -> CGColor {
    CGColor(
        red: CGFloat((hex >> 16) & 0xFF) / 255,
        green: CGFloat((hex >> 8) & 0xFF) / 255,
        blue: CGFloat(hex & 0xFF) / 255,
        alpha: alpha
    )
}

func render(size S: CGFloat) -> NSImage {
    let image = NSImage(size: NSSize(width: S, height: S))
    image.lockFocus()
    defer { image.unlockFocus() }
    guard let ctx = NSGraphicsContext.current?.cgContext else { return image }

    ctx.setShouldAntialias(true)
    ctx.interpolationQuality = .high

    // macOS 图标不占满画布，四周留白约 10%
    let inset = S * 0.098
    let body = CGRect(x: inset, y: inset, width: S - inset * 2, height: S - inset * 2)
    let shape = squircle(in: body)

    // 底：系统蓝 → 青的对角渐变
    ctx.saveGState()
    ctx.addPath(shape)
    ctx.clip()
    let bg = CGGradient(
        colorsSpace: CGColorSpaceCreateDeviceRGB(),
        colors: [rgb(0x2E9BF5), rgb(0x0B62D6)] as CFArray,
        locations: [0, 1]
    )!
    ctx.drawLinearGradient(
        bg,
        start: CGPoint(x: body.minX, y: body.maxY),
        end: CGPoint(x: body.maxX, y: body.minY),
        options: []
    )

    // 顶部一层柔光，让平面渐变有点体积感
    let sheen = CGGradient(
        colorsSpace: CGColorSpaceCreateDeviceRGB(),
        colors: [rgb(0xFFFFFF, 0.26), rgb(0xFFFFFF, 0)] as CFArray,
        locations: [0, 1]
    )!
    ctx.drawRadialGradient(
        sheen,
        startCenter: CGPoint(x: body.midX, y: body.maxY - body.height * 0.06),
        startRadius: 0,
        endCenter: CGPoint(x: body.midX, y: body.maxY - body.height * 0.06),
        endRadius: body.width * 0.72,
        options: []
    )
    ctx.restoreGState()

    // 内描边，贴合系统图标的边缘处理
    ctx.saveGState()
    ctx.addPath(shape)
    ctx.setStrokeColor(rgb(0xFFFFFF, 0.22))
    ctx.setLineWidth(S * 0.008)
    ctx.strokePath()
    ctx.restoreGState()

    // MARK: 链路
    //
    // 一条留缺口的圆环，两端各一个节点，缺口处用箭头收尾表示方向（store → 本机）。
    // 节点必须落在环的两个**端点**上，否则会盖住环线或箭头。
    let cx = body.midX, cy = body.midY
    let ringR = body.width * 0.255
    let lw = S * 0.058
    let nodeR = S * 0.058

    // 环：从 40° 逆时针画到 300°（缺口在右下方约 -20°…40°）
    let startDeg: CGFloat = 40, endDeg: CGFloat = 300
    let startRad = startDeg * .pi / 180, endRad = endDeg * .pi / 180

    ctx.saveGState()
    ctx.setLineCap(.round)
    ctx.setLineWidth(lw)
    ctx.setStrokeColor(rgb(0xFFFFFF, 0.95))
    ctx.addArc(
        center: CGPoint(x: cx, y: cy),
        radius: ringR,
        startAngle: startRad,
        endAngle: endRad,
        clockwise: false
    )
    ctx.strokePath()
    ctx.restoreGState()

    // 箭头：画在环的终点（300°）处，沿逆时针切线指出去
    let tip = CGPoint(x: cx + cos(endRad) * ringR, y: cy + sin(endRad) * ringR)
    let head = S * 0.062
    ctx.saveGState()
    ctx.translateBy(x: tip.x, y: tip.y)
    // 逆时针行进方向的切线 = 角度 + 90°
    ctx.rotate(by: endRad + .pi / 2)
    ctx.beginPath()
    ctx.move(to: CGPoint(x: 0, y: head * 0.95))
    ctx.addLine(to: CGPoint(x: -head * 0.9, y: -head * 0.6))
    ctx.addLine(to: CGPoint(x: head * 0.9, y: -head * 0.6))
    ctx.closePath()
    ctx.setFillColor(rgb(0xFFFFFF, 0.95))
    ctx.fillPath()
    ctx.restoreGState()

    // 两个节点：实心 = 本机，空心 = 共享的 store。
    // 放在环的中段（120° 与 220°），不与端点/箭头打架。
    for (deg, filled) in [(CGFloat(122), true), (CGFloat(218), false)] {
        let rad = deg * .pi / 180
        let p = CGPoint(x: cx + cos(rad) * ringR, y: cy + sin(rad) * ringR)
        let box = CGRect(x: p.x - nodeR, y: p.y - nodeR, width: nodeR * 2, height: nodeR * 2)
        // 先用背景色垫一圈，把环线挖断，节点才不像串在线上的珠子
        ctx.setFillColor(rgb(0x1F7AE0))
        ctx.fillEllipse(in: box.insetBy(dx: -lw * 0.42, dy: -lw * 0.42))
        if filled {
            ctx.setFillColor(rgb(0xFFFFFF))
            ctx.fillEllipse(in: box)
        } else {
            ctx.setStrokeColor(rgb(0xFFFFFF))
            ctx.setLineWidth(S * 0.028)
            ctx.strokeEllipse(in: box.insetBy(dx: S * 0.014, dy: S * 0.014))
        }
    }

    return image
}

// MARK: - 输出

let out = CommandLine.arguments.count > 1
    ? CommandLine.arguments[1]
    : FileManager.default.currentDirectoryPath + "/AppIcon.png"

let icon = render(size: 1024)
guard let tiff = icon.tiffRepresentation,
      let rep = NSBitmapImageRep(data: tiff),
      let png = rep.representation(using: .png, properties: [:])
else {
    FileHandle.standardError.write(Data("渲染失败\n".utf8))
    exit(1)
}
try png.write(to: URL(filePath: out))
print("已写出 \(out)（1024×1024）")
