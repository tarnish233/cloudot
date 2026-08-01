import AppKit

/// 为 18pt 菜单栏专门绘制的小机器人。
///
/// 每一帧都是 template image：源图始终用白色绘制，AppKit 会根据菜单栏背景
/// 自动选择可读的前景色。这样深色菜单栏是用户要的白色，同时浅色菜单栏也不会隐身。
@MainActor
enum RobotAnimator {
    static let barHeight: CGFloat = 18
    static let canvasSize = NSSize(width: 22, height: barHeight)

    private static var memo: [String: RobotAnimationSequence] = [:]

    private static func memoized(
        _ key: String,
        build: @MainActor () -> RobotAnimationSequence
    ) -> RobotAnimationSequence {
        if let cached = memo[key] { return cached }
        let sequence = build()
        memo[key] = sequence
        return sequence
    }

    static func resting(for state: AppModel.IconState) -> RobotAnimationSequence {
        switch state {
        case .healthy: idle
        case .pending: pending
        case .refreshing: refreshing
        case .syncing: working
        case .broken: broken
        case .unavailable: offline
        }
    }

    /// 正常：绝大部分时间安静，仅偶尔自然眨眼，避免持续晃动分散注意力。
    static var idle: RobotAnimationSequence {
        memoized("idle") {
            let neutral = render()
            let frames = Array(repeating: neutral, count: 28)
                + [render(expression: .blink), render(expression: .blink)]
                + Array(repeating: neutral, count: 5)
            return RobotAnimationSequence(frames: frames, interval: 0.12, loops: true)
        }
    }

    /// 待同步：天线发出一圈克制的提示，不让整个图标持续跳动。
    static var pending: RobotAnimationSequence {
        memoized("pending") {
            let pulse = stride(from: CGFloat(0), through: 1, by: 0.2).map {
                render(signalProgress: $0)
            }
            let pause = Array(repeating: render(), count: 18)
            return RobotAnimationSequence(frames: pulse + pause, interval: 0.1, loops: true)
        }
    }

    /// 刷新：眼睛扫描状态，天线同步左右摆动。
    static var refreshing: RobotAnimationSequence {
        memoized("refreshing") {
            let positions: [CGFloat] = [-1.2, -0.8, -0.4, 0, 0.4, 0.8, 1.2, 0.8, 0.4, 0, -0.4, -0.8]
            let frames = positions.map { offset in
                render(eyeOffset: offset, antennaAngle: offset * 0.22)
            }
            return RobotAnimationSequence(frames: frames, interval: 0.08, loops: true)
        }
    }

    /// 同步：机器人轻微起伏，两只机械臂交替工作。
    static var working: RobotAnimationSequence {
        memoized("working") {
            let phases: [CGFloat] = [-1, -0.5, 0, 0.5, 1, 0.5, 0, -0.5]
            let frames = phases.enumerated().map { index, phase in
                render(
                    verticalOffset: index.isMultiple(of: 2) ? 0.25 : 0,
                    armPhase: phase
                )
            }
            return RobotAnimationSequence(frames: frames, interval: 0.09, loops: true)
        }
    }

    /// 配置损坏：叉眼、皱眉，并稍微歪头。
    static var broken: RobotAnimationSequence {
        memoized("broken") {
            RobotAnimationSequence(
                frames: [render(expression: .error, rotation: -0.08)],
                interval: 0.1,
                loops: false
            )
        }
    }

    /// 找不到 CLI：天线垂下、闭眼，并降低不透明度。
    static var offline: RobotAnimationSequence {
        memoized("offline") {
            RobotAnimationSequence(
                frames: [render(expression: .offline, antennaAngle: -0.72, alpha: 0.48)],
                interval: 0.1,
                loops: false
            )
        }
    }

    /// 成功：亮出笑脸并做一次轻快而短促的弹跳。
    static var success: RobotAnimationSequence {
        memoized("success") {
            let frames = [
                render(scaleY: 0.9),
                render(expression: .happy, verticalOffset: 0.8, scaleY: 1.06),
                render(expression: .happy, verticalOffset: 1.5, scaleY: 1.02),
                render(expression: .happy, verticalOffset: 0.7),
                render(expression: .happy, scaleY: 0.93),
                render(expression: .happy),
                render(expression: .happy),
                render(),
            ]
            return RobotAnimationSequence(frames: frames, interval: 0.075, loops: false)
        }
    }

    /// 失败：显示叉眼并摇头两次，随后回到当前常驻状态。
    static var failure: RobotAnimationSequence {
        memoized("failure") {
            let rotations: [CGFloat] = [0, -0.12, 0.12, -0.1, 0.1, -0.06, 0.06, 0, 0]
            let frames = rotations.map { render(expression: .error, rotation: $0) } + [render()]
            return RobotAnimationSequence(frames: frames, interval: 0.07, loops: false)
        }
    }

    static func oneShot(for pulse: AppModel.IconPulse.Kind) -> RobotAnimationSequence {
        switch pulse {
        case .success: success
        case .failure: failure
        }
    }

    // MARK: - Vector rendering

    private static func render(
        expression: RobotExpression = .neutral,
        eyeOffset: CGFloat = 0,
        antennaAngle: CGFloat = 0,
        verticalOffset: CGFloat = 0,
        rotation: CGFloat = 0,
        scaleY: CGFloat = 1,
        armPhase: CGFloat? = nil,
        signalProgress: CGFloat? = nil,
        alpha: CGFloat = 1
    ) -> NSImage {
        let image = NSImage(size: canvasSize, flipped: false) { _ in
            guard let context = NSGraphicsContext.current?.cgContext else { return false }
            context.saveGState()
            defer { context.restoreGState() }

            context.setShouldAntialias(true)
            context.setAllowsAntialiasing(true)
            context.setLineWidth(1.3)
            context.setLineCap(.round)
            context.setLineJoin(.round)
            context.setStrokeColor(NSColor.white.withAlphaComponent(alpha).cgColor)
            context.setFillColor(NSColor.white.withAlphaComponent(alpha).cgColor)

            context.translateBy(x: canvasSize.width / 2, y: 8.15 + verticalOffset)
            context.rotate(by: rotation)
            context.scaleBy(x: 1, y: scaleY)

            drawAntenna(in: context, angle: antennaAngle, signalProgress: signalProgress, alpha: alpha)
            drawBody(in: context, expression: expression, eyeOffset: eyeOffset)
            if let armPhase {
                drawArms(in: context, phase: armPhase)
            }
            return true
        }
        image.isTemplate = true
        image.accessibilityDescription = "Cloudot"
        return image
    }

    private static func drawAntenna(
        in context: CGContext,
        angle: CGFloat,
        signalProgress: CGFloat?,
        alpha: CGFloat
    ) {
        let base = CGPoint(x: 0, y: 4.2)
        let tip = CGPoint(x: sin(angle) * 1.8, y: 6.15 + cos(angle) * 0.25)
        context.move(to: base)
        context.addLine(to: tip)
        context.strokePath()
        context.fillEllipse(in: CGRect(x: tip.x - 0.72, y: tip.y - 0.72, width: 1.44, height: 1.44))

        if let progress = signalProgress, progress > 0 {
            context.saveGState()
            context.setAlpha(alpha * (1 - progress) * 0.75)
            context.setLineWidth(0.8)
            let radius = 0.85 + progress * 2
            context.strokeEllipse(in: CGRect(
                x: tip.x - radius,
                y: tip.y - radius,
                width: radius * 2,
                height: radius * 2
            ))
            context.restoreGState()
        }
    }

    private static func drawBody(
        in context: CGContext,
        expression: RobotExpression,
        eyeOffset: CGFloat
    ) {
        let head = CGRect(x: -6.15, y: -4.05, width: 12.3, height: 8.25)
        context.addPath(CGPath(roundedRect: head, cornerWidth: 2.45, cornerHeight: 2.45, transform: nil))
        context.strokePath()

        // 两侧短耳让 18pt 轮廓仍然一眼能看出是机器人。
        context.move(to: CGPoint(x: -7.15, y: 0.1))
        context.addLine(to: CGPoint(x: -6.15, y: 0.1))
        context.move(to: CGPoint(x: 6.15, y: 0.1))
        context.addLine(to: CGPoint(x: 7.15, y: 0.1))
        context.strokePath()

        switch expression {
        case .neutral:
            drawDotEyes(in: context, offset: eyeOffset)
            drawMouth(in: context, happy: false)
        case .blink, .offline:
            drawClosedEyes(in: context)
            if expression == .blink {
                drawMouth(in: context, happy: false)
            }
        case .happy:
            drawHappyEyes(in: context)
            drawMouth(in: context, happy: true)
        case .error:
            drawErrorEyes(in: context)
            drawFrown(in: context)
        }
    }

    private static func drawDotEyes(in context: CGContext, offset: CGFloat) {
        for x in [-2.15 + offset, 2.15 + offset] {
            context.fillEllipse(in: CGRect(x: x - 0.62, y: 0.2, width: 1.24, height: 1.5))
        }
    }

    private static func drawClosedEyes(in context: CGContext) {
        for x in [-2.15, 2.15] {
            context.move(to: CGPoint(x: x - 0.62, y: 0.72))
            context.addLine(to: CGPoint(x: x + 0.62, y: 0.72))
        }
        context.strokePath()
    }

    private static func drawHappyEyes(in context: CGContext) {
        for x in [-2.15, 2.15] {
            context.move(to: CGPoint(x: x - 0.68, y: 0.48))
            context.addQuadCurve(to: CGPoint(x: x + 0.68, y: 0.48), control: CGPoint(x: x, y: 1.45))
        }
        context.strokePath()
    }

    private static func drawErrorEyes(in context: CGContext) {
        for x in [-2.15, 2.15] {
            context.move(to: CGPoint(x: x - 0.55, y: 0.2))
            context.addLine(to: CGPoint(x: x + 0.55, y: 1.3))
            context.move(to: CGPoint(x: x + 0.55, y: 0.2))
            context.addLine(to: CGPoint(x: x - 0.55, y: 1.3))
        }
        context.strokePath()
    }

    private static func drawMouth(in context: CGContext, happy: Bool) {
        context.move(to: CGPoint(x: -1.25, y: -1.65))
        if happy {
            context.addQuadCurve(to: CGPoint(x: 1.25, y: -1.65), control: CGPoint(x: 0, y: -2.65))
        } else {
            context.addLine(to: CGPoint(x: 1.25, y: -1.65))
        }
        context.strokePath()
    }

    private static func drawFrown(in context: CGContext) {
        context.move(to: CGPoint(x: -1.2, y: -2.15))
        context.addQuadCurve(to: CGPoint(x: 1.2, y: -2.15), control: CGPoint(x: 0, y: -1.05))
        context.strokePath()
    }

    private static func drawArms(in context: CGContext, phase: CGFloat) {
        context.move(to: CGPoint(x: -6.55, y: -1.05))
        context.addLine(to: CGPoint(x: -8, y: -2.15 + phase * 0.65))
        context.move(to: CGPoint(x: 6.55, y: -1.05))
        context.addLine(to: CGPoint(x: 8, y: -2.15 - phase * 0.65))
        context.strokePath()
    }
}
