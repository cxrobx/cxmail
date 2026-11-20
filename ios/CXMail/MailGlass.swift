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

// Light mode uses warm off-whites (the #F8F7F5 canvas from T3 Code) instead of
// pure white. Dark mode resolves to the exact system colours it used before.
extension Color {
    /// Full-screen paper behind writing surfaces (composer). Dark: systemBackground.
    static let mailPaper = Color(uiColor: UIColor { $0.userInterfaceStyle == .dark
        ? UIColor.systemBackground.resolvedColor(with: $0)
        : UIColor(red: 0.973, green: 0.969, blue: 0.961, alpha: 1) })
    /// Cards and grouped rows, one step above the canvas. Dark: secondarySystemGroupedBackground.
    static let mailSurface = Color(uiColor: UIColor { $0.userInterfaceStyle == .dark
        ? UIColor.secondarySystemGroupedBackground.resolvedColor(with: $0)
        : UIColor(red: 0.988, green: 0.984, blue: 0.976, alpha: 1) })
}

/// A Form on the MailCanvas with off-white rows, so light-mode settings sheets
/// don't fall back to iOS's grey canvas and pure-white rows.
struct MailForm<Content: View>: View {
    @ViewBuilder var content: Content
    var body: some View {
        Form { Group { content }.listRowBackground(Color.mailSurface) }
            .scrollContentBackground(.hidden)
            .background(Color("MailCanvas"))
    }
}
