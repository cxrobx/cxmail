import SwiftUI

extension View {
    func mailGlass(capsule: Bool = false) -> some View { modifier(MailGlass(capsule: capsule)) }
}

private struct MailGlass: ViewModifier {
    var capsule: Bool
    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            if capsule { content.glassEffect(.regular, in: Capsule()) }
            else { content.glassEffect(.regular, in: RoundedRectangle(cornerRadius: 16, style: .continuous)) }
        } else {
            if capsule { content.background(.regularMaterial, in: Capsule()) }
            else { content.background(.regularMaterial, in: RoundedRectangle(cornerRadius: 16, style: .continuous)) }
        }
    }
}
