// cloudot 应用图标 —— 原创设计，不派生自任何第三方素材。
//
// 意象直接取 cloudot 在做的事：**同步**。glyph 用的就是菜单栏那个
// `arrow.triangle.2.circlepath`，同一个 SF Symbol —— 不是「风格相似」，
// 是同一份矢量数据，所以 App 图标和菜单栏图标永远不会走形。
//
// 底面是渐变 squircle，材质靠六层叠出来：外投影、多段底色渐变、球面高光、
// 底部冷色反射、内侧倒角、符号自身的投影与柔光。全部 CoreGraphics 绘制，
// 没有外部素材，授权干净。
//
// 用法：swift make-icon.swift [输出路径]   → AppIcon.png（1024×1024）
import AppKit
import Foundation

// MARK: - 基础工具

/// macOS 的图标圆角是超椭圆而不是圆弧，小尺寸下看得出差别。
func squircle(in rect: CGRect, n: CGFloat = 5) -> CGPath {
    let path = CGMutablePath()
    let a = rect.width / 2, b = rect.height / 2
    let cx = rect.midX, cy = rect.midY
    let steps = 1440
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

func bitmap(_ width: Int, _ height: Int) -> NSBitmapImageRep {
    NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height,
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
    )!
}

func draw(into rep: NSBitmapImageRep, _ body: (CGContext) -> Void) {
    let context = NSGraphicsContext(bitmapImageRep: rep)!
    NSGraphicsContext.saveGraphicsState()
    defer { NSGraphicsContext.restoreGraphicsState() }
    NSGraphicsContext.current = context
    context.cgContext.setShouldAntialias(true)
    context.cgContext.interpolationQuality = .high
    body(context.cgContext)
}

// MARK: - glyph

/// SF Symbol 渲染成纯白蒙版，后面所有对符号的处理（投影、柔光、渐变填充）
/// 都拿这张蒙版当形状用。
func glyphMask(_ name: String, side: CGFloat, weight: NSFont.Weight) -> (CGImage, NSSize) {
    let symbol = NSImage(systemSymbolName: name, accessibilityDescription: nil)!
        .withSymbolConfiguration(.init(pointSize: side, weight: weight))!
    let size = symbol.size
    let rep = bitmap(Int(size.width.rounded()), Int(size.height.rounded()))
    draw(into: rep) { _ in
        let rect = NSRect(origin: .zero, size: size)
        symbol.draw(in: rect)
        NSColor.white.set()
        rect.fill(using: .sourceAtop)
    }
    return (rep.cgImage!, size)
}

/// 给蒙版填上垂直渐变，做出「有厚度的白色材质」而不是一片死白。
func shadeGlyph(_ mask: CGImage, _ size: NSSize, top: CGColor, bottom: CGColor) -> CGImage {
    let rep = bitmap(Int(size.width.rounded()), Int(size.height.rounded()))
    draw(into: rep) { ctx in
        let gradient = CGGradient(
            colorsSpace: CGColorSpaceCreateDeviceRGB(),
            colors: [top, bottom] as CFArray,
            locations: [0, 1]
        )!
        ctx.drawLinearGradient(
            gradient,
            start: CGPoint(x: 0, y: size.height),
            end: CGPoint(x: 0, y: 0),
            options: []
        )
        ctx.setBlendMode(.destinationIn)
        ctx.draw(mask, in: CGRect(origin: .zero, size: size))
    }
    return rep.cgImage!
}

// MARK: - 渲染

func render(size S: CGFloat) -> NSBitmapImageRep {
    let rep = bitmap(Int(S), Int(S))
    draw(into: rep) { ctx in
        // macOS 图标不占满画布，四周留白约 10%
        let inset = S * 0.098
        let body = CGRect(x: inset, y: inset, width: S - inset * 2, height: S - inset * 2)
        let shape = squircle(in: body)

        // ── 1. 外投影，让图标从背景上浮起来。
        //
        // 投影和底色必须**分两层**画：同一层里填色 + 投影时，抗锯齿边缘会漏出填充色，
        // 看着像描歪了一圈边框。这里先单独把投影打到画布上，底色渐变随后整片盖住同一个
        // 形状，边缘就只剩渐变自己的颜色。
        // 投影色还必须是**中性黑** —— 试过用深蓝，会在图标外沿留一圈肉眼可见的蓝晕，
        // 效果跟描边一样糟。
        ctx.saveGState()
        ctx.setShadow(
            offset: CGSize(width: 0, height: -S * 0.008),
            blur: S * 0.022,
            color: rgb(0x000000, 0.20)
        )
        ctx.addPath(shape)
        ctx.setFillColor(rgb(0x000000))
        ctx.fillPath()
        ctx.restoreGState()

        ctx.saveGState()
        ctx.addPath(shape)
        ctx.clip()

        let space = CGColorSpaceCreateDeviceRGB()

        // ── 2. 底色：三段渐变。中间提亮一档，两点线性渐变会显得很平。
        let background = CGGradient(
            colorsSpace: space,
            colors: [rgb(0x5CC3FA), rgb(0x2A93F0), rgb(0x0B57C9)] as CFArray,
            locations: [0, 0.48, 1]
        )!
        ctx.drawLinearGradient(
            background,
            start: CGPoint(x: body.minX, y: body.maxY),
            end: CGPoint(x: body.maxX, y: body.minY),
            options: []
        )

        // ── 3. 球面高光：径向，光源在左上偏上
        let sheen = CGGradient(
            colorsSpace: space,
            colors: [rgb(0xFFFFFF, 0.30), rgb(0xFFFFFF, 0.05), rgb(0xFFFFFF, 0)] as CFArray,
            locations: [0, 0.45, 1]
        )!
        let sheenCenter = CGPoint(x: body.midX - body.width * 0.14, y: body.maxY - body.height * 0.10)
        ctx.drawRadialGradient(
            sheen,
            startCenter: sheenCenter, startRadius: 0,
            endCenter: sheenCenter, endRadius: body.width * 0.78,
            options: []
        )

        // ── 4. 底部反射光：冷色回弹，玻璃质感主要靠这一层
        let bounce = CGGradient(
            colorsSpace: space,
            colors: [rgb(0x7FE9FF, 0), rgb(0x7FE9FF, 0.22)] as CFArray,
            locations: [0, 1]
        )!
        ctx.drawLinearGradient(
            bounce,
            start: CGPoint(x: body.midX, y: body.minY + body.height * 0.42),
            end: CGPoint(x: body.midX, y: body.minY),
            options: []
        )

        // ── 5. 内侧倒角：顶缘提亮 + 底缘压暗，做出边框厚度。
        // 手法是拿「画布挖掉 shape」的反向路径投影到内侧。
        ctx.saveGState()
        let outer = CGMutablePath()
        outer.addRect(body.insetBy(dx: -S, dy: -S))
        outer.addPath(shape)
        ctx.setShadow(
            offset: CGSize(width: 0, height: -S * 0.010),
            blur: S * 0.020,
            color: rgb(0xFFFFFF, 0.55)
        )
        ctx.addPath(outer)
        ctx.setFillColor(rgb(0x000000))
        ctx.fillPath(using: .evenOdd)
        ctx.setShadow(
            offset: CGSize(width: 0, height: S * 0.012),
            blur: S * 0.026,
            color: rgb(0x063A78, 0.50)
        )
        ctx.addPath(outer)
        ctx.setFillColor(rgb(0x000000))
        ctx.fillPath(using: .evenOdd)
        ctx.restoreGState()

        ctx.restoreGState()

        // ── 6. 外描边：收住边缘
        ctx.saveGState()
        ctx.addPath(shape)
        ctx.setStrokeColor(rgb(0xFFFFFF, 0.14))
        ctx.setLineWidth(S * 0.005)
        ctx.strokePath()
        ctx.restoreGState()

        // ── 7. glyph —— 和菜单栏「同步中」用的是同一个 SF Symbol。
        // 改这里之前先看 IconState+Symbol.swift，两边要一起改。
        let (mask, glyphSize) = glyphMask(
            "arrow.triangle.2.circlepath",
            side: body.width * 0.54,
            weight: .semibold
        )
        let glyphRect = CGRect(
            x: body.midX - glyphSize.width / 2,
            y: body.midY - glyphSize.height / 2,
            width: glyphSize.width,
            height: glyphSize.height
        )

        // 符号自己的投影，让它浮在底面之上
        ctx.saveGState()
        ctx.setShadow(
            offset: CGSize(width: 0, height: -S * 0.008),
            blur: S * 0.022,
            color: rgb(0x04305F, 0.45)
        )
        ctx.draw(mask, in: glyphRect)
        ctx.restoreGState()

        // ── 8. 柔光：零偏移的阴影垫在符号后面，做出一圈很淡的晕。
        //
        // 两层而不是一层：紧的那层（blur 3%）负责「亮起来」，散的那层（blur 7%）负责
        // 「晕开」。单层做不到 —— blur 小了贴着符号看不出来，blur 大了散成一片白雾把
        // 底色洗掉。
        //
        // 混合模式用普通混合，**不要用 `.plusLighter`**：加法混合在蓝底上会把绿蓝通道
        // 推到 255 饱和，出来是霓虹灯描边而不是光晕（实测紧贴符号处从 40,131,192 直接
        // 跳到 194,255,255）。颜色用冷白，跟底色同色系，看着才像底面被照亮。
        //
        // alpha 刻意压得很低：这是气氛，不是第二个图形。一旦能单独看出光晕的边界就过了。
        ctx.saveGState()
        ctx.setShadow(offset: .zero, blur: S * 0.030, color: rgb(0xE8F6FF, 0.19))
        ctx.draw(mask, in: glyphRect)
        ctx.setShadow(offset: .zero, blur: S * 0.070, color: rgb(0xBFE4FF, 0.14))
        ctx.draw(mask, in: glyphRect)
        ctx.restoreGState()

        // ── 9. 符号本体：上端纯白、下端微微带蓝
        let shaded = shadeGlyph(mask, glyphSize, top: rgb(0xFFFFFF), bottom: rgb(0xE3F1FF))
        ctx.draw(shaded, in: glyphRect)
    }
    return rep
}

// MARK: - 输出

let out = CommandLine.arguments.count > 1
    ? CommandLine.arguments[1]
    : FileManager.default.currentDirectoryPath + "/AppIcon.png"

guard let png = render(size: 1024).representation(using: .png, properties: [:]) else {
    FileHandle.standardError.write(Data("渲染失败\n".utf8))
    exit(1)
}
try png.write(to: URL(filePath: out))
print("已写出 \(out)（1024×1024）")
