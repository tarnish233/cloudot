import SwiftUI

enum SystemTheme {
    /// “系统设置 → 外观 → 强调色”。
    ///
    /// 使用 AppKit 的动态系统颜色，而不是读取 UserDefaults 中未公开的键；这样颜色会
    /// 正确处理多彩模式、深浅外观、高对比度，以及用户运行时修改强调色的情况。
    static var accentColor: Color {
        Color(nsColor: .controlAccentColor)
    }
}
