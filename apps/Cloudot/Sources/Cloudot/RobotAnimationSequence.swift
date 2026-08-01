import AppKit

/// 一段菜单栏机器人逐帧动画。
struct RobotAnimationSequence {
    let frames: [NSImage]
    let interval: TimeInterval
    let loops: Bool
}
