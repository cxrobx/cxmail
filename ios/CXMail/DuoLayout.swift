import SwiftUI

struct DuoLayout: Equatable {
    var split = false
    var leading: CGFloat = 340
    var gap: CGFloat = 1
    static func resolve(width: CGFloat, height: CGFloat, fold: CGRect?) -> Self {
        // A landscape closed Duo has only 432 usable points of height.
        guard width >= 760, height >= 500 else { return .init() }
        if let fold, fold.height > fold.width, fold.minX >= 300, width - fold.maxX >= 300 {
            return .init(split: true, leading: fold.minX, gap: fold.width)
        }
        return .init(split: true, leading: min(340, max(300, width * 0.38)), gap: 1)
    }
}

extension GeometryProxy {
    var mailLayout: DuoLayout {
        var fold: CGRect?
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("--uitesting") && ProcessInfo.processInfo.arguments.contains("--duo-wide") {
            fold = CGRect(x: 455.5, y: 0, width: 40, height: 635)
        }
        #endif
        if #available(iOS 27.1, *), fold == nil { fold = reservedRegions(kind: .division).first?.frame }
        return .resolve(width: size.width, height: size.height, fold: fold)
    }
}
