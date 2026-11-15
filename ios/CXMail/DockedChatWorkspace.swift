import SwiftUI

/// Mail and chat use native column containment, including the Duo's asymmetric
/// status area. Only the visible divider controls their relative widths.
struct DockedChatWorkspace<MailContent: View>: View {
    @Bindable var store: MailStore
    let chat: ChatStore
    @ViewBuilder let mailContent: () -> MailContent
    @State private var dragStart: CGFloat?

    var body: some View {
        GeometryReader { geometry in
            let fold = geometry.mailFold
            let folded = fold.map { $0.height > $0.width && $0.minX >= 300 && geometry.size.width - $0.maxX >= 300 } ?? false
            let width = folded ? fold!.minX : Self.mailWidth(geometry.size.width, fraction: store.chatMailFraction)
            let gap = folded ? fold!.width : 1
            NavigationSplitView(columnVisibility: .constant(.all)) {
                NavigationStack { mailContent() }
                    .padding(.trailing, gap - 1)
                    .toolbar(removing: .sidebarToggle)
                    .navigationSplitViewColumnWidth(min: width + gap - 1, ideal: width + gap - 1, max: width + gap - 1)
            } detail: {
                ChatView(mail: store, chat: chat, docked: true, canDock: true,
                    onPresentationChange: { store.chatDocked = false; store.chatPresented = true },
                    onClose: { store.chatDocked = false; chat.pause() })
                    .navigationSplitViewColumnWidth(min: 300, ideal: 300, max: .infinity)
            }
            .navigationSplitViewStyle(.balanced)
            .environment(\.horizontalSizeClass, .regular)
            .overlay(alignment: .leading) {
                if !folded {
                    ZStack {
                        Rectangle().fill(Color(uiColor: .separator)).frame(width: 1)
                        Capsule().fill(.secondary).frame(width: 4, height: 44)
                    }
                    .frame(width: 24).contentShape(Rectangle()).offset(x: width - 12)
                    .gesture(DragGesture(minimumDistance: 2)
                        .onChanged { value in
                            if dragStart == nil { dragStart = width }
                            store.chatMailFraction = Self.mailWidth(geometry.size.width, fraction: ((dragStart ?? width) + value.translation.width) / geometry.size.width) / geometry.size.width
                        }
                        .onEnded { _ in dragStart = nil })
                    .accessibilityElement().accessibilityLabel("Mail and chat divider")
                    .accessibilityValue("\(Int(width / geometry.size.width * 100)) percent mail")
                    .accessibilityAdjustableAction { direction in
                        store.chatMailFraction = Self.mailWidth(geometry.size.width, fraction: store.chatMailFraction + (direction == .increment ? 0.05 : -0.05)) / geometry.size.width
                    }
                    .accessibilityIdentifier("chatDivider")
                }
            }
        }
    }

    static func mailWidth(_ total: CGFloat, fraction: CGFloat) -> CGFloat {
        min(max(300, total * fraction), max(300, total - 301))
    }
}
