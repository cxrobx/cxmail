import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// A multiline text field that also accepts pasted images: long-press → Paste,
/// the keyboard's "Paste from…" suggestion and ⌘V all land here. Text pastes
/// normally; images go to `onPasteImages` instead of the text.
struct PastingTextView: UIViewRepresentable {
    @Binding var text: String
    @Binding var isFocused: Bool
    var placeholder: String
    var minLines: Int = 1
    var maxLines: Int? = nil
    var identifier: String
    var onPasteImages: ([Data]) -> Void

    func makeUIView(context: Context) -> ImagePastingTextView {
        let view = ImagePastingTextView()
        view.delegate = context.coordinator
        view.font = .preferredFont(forTextStyle: .body)
        view.adjustsFontForContentSizeCategory = true
        view.backgroundColor = .clear
        view.textContainerInset = .zero
        view.textContainer.lineFragmentPadding = 0
        view.isScrollEnabled = false
        view.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        view.accessibilityIdentifier = identifier
        view.placeholder = placeholder
        view.onPasteImages = { onPasteImages($0) }
        view.text = text
        return view
    }

    func updateUIView(_ view: ImagePastingTextView, context: Context) {
        context.coordinator.parent = self
        view.onPasteImages = { onPasteImages($0) }
        if view.text != text { view.text = text; view.refreshPlaceholder() }
        if isFocused && !view.isFirstResponder && view.window != nil {
            DispatchQueue.main.async { view.becomeFirstResponder() }
        } else if !isFocused && view.isFirstResponder {
            DispatchQueue.main.async { view.resignFirstResponder() }
        }
    }

    func sizeThatFits(_ proposal: ProposedViewSize, uiView view: ImagePastingTextView, context: Context) -> CGSize? {
        let width = proposal.width ?? 320
        let line = view.font?.lineHeight ?? 22
        let fitted = view.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude)).height
        var height = max(fitted, line * CGFloat(minLines))
        if let maxLines, height > line * CGFloat(maxLines) {
            height = line * CGFloat(maxLines)
            if !view.isScrollEnabled { DispatchQueue.main.async { view.isScrollEnabled = true } }
        } else if view.isScrollEnabled {
            DispatchQueue.main.async { view.isScrollEnabled = false }
        }
        return CGSize(width: width, height: ceil(height))
    }

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    final class Coordinator: NSObject, UITextViewDelegate {
        var parent: PastingTextView
        init(_ parent: PastingTextView) { self.parent = parent }
        func textViewDidChange(_ view: UITextView) {
            parent.text = view.text
            (view as? ImagePastingTextView)?.refreshPlaceholder()
            view.invalidateIntrinsicContentSize()
        }
        func textViewDidBeginEditing(_ view: UITextView) { if !parent.isFocused { parent.isFocused = true } }
        func textViewDidEndEditing(_ view: UITextView) { if parent.isFocused { parent.isFocused = false } }
    }
}

final class ImagePastingTextView: UITextView {
    var onPasteImages: ([Data]) -> Void = { _ in }
    private let placeholderLabel = UILabel()

    var placeholder: String {
        get { placeholderLabel.text ?? "" }
        set { placeholderLabel.text = newValue; accessibilityHint = nil }
    }

    override init(frame: CGRect, textContainer: NSTextContainer?) {
        super.init(frame: frame, textContainer: textContainer)
        placeholderLabel.font = .preferredFont(forTextStyle: .body)
        placeholderLabel.adjustsFontForContentSizeCategory = true
        placeholderLabel.textColor = .placeholderText
        placeholderLabel.numberOfLines = 0
        placeholderLabel.isAccessibilityElement = false
        placeholderLabel.translatesAutoresizingMaskIntoConstraints = false
        addSubview(placeholderLabel)
        NSLayoutConstraint.activate([
            placeholderLabel.topAnchor.constraint(equalTo: topAnchor),
            placeholderLabel.leadingAnchor.constraint(equalTo: leadingAnchor),
            placeholderLabel.widthAnchor.constraint(equalTo: widthAnchor),
        ])
        // Lets the system offer Paste (menu and keyboard suggestion) when the
        // clipboard holds only an image.
        pasteConfiguration = UIPasteConfiguration(acceptableTypeIdentifiers:
            [UTType.plainText, .text, .url, .image].map(\.identifier))
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    override var text: String! { didSet { refreshPlaceholder() } }
    func refreshPlaceholder() { placeholderLabel.isHidden = !(text ?? "").isEmpty }
    override var accessibilityValue: String? {
        get { text } set { }
    }

    override func canPerformAction(_ action: Selector, withSender sender: Any?) -> Bool {
        if action == #selector(paste(_:)), UIPasteboard.general.hasImages { return true }
        return super.canPerformAction(action, withSender: sender)
    }

    override func paste(_ sender: Any?) {
        let board = UIPasteboard.general
        guard board.hasImages else { return super.paste(sender) }
        // Prefer the original bytes (keeps PNG/HEIC/JPEG as copied); fall back to UIImage.
        var images: [Data] = []
        for index in 0..<board.numberOfItems {
            let set = IndexSet(integer: index)
            let types = board.types(forItemSet: set)?.first ?? []
            if let type = types.first(where: { UTType($0)?.conforms(to: .image) == true }),
               let data = board.data(forPasteboardType: type, inItemSet: set)?.first {
                images.append(data)
            }
        }
        if images.isEmpty { images = (board.images ?? []).compactMap { $0.pngData() } }
        if images.isEmpty { return super.paste(sender) }
        onPasteImages(images)
    }

    override func paste(itemProviders: [NSItemProvider]) {
        // Keyboard "Paste from…" and drop-style pastes arrive here.
        let imageProviders = itemProviders.filter { p in
            p.registeredTypeIdentifiers.contains { UTType($0)?.conforms(to: .image) == true }
        }
        guard !imageProviders.isEmpty else { return super.paste(itemProviders: itemProviders) }
        let group = DispatchGroup()
        let lock = NSLock()
        var images = [Int: Data]()
        for (i, provider) in imageProviders.enumerated() {
            guard let type = provider.registeredTypeIdentifiers.first(where: { UTType($0)?.conforms(to: .image) == true }) else { continue }
            group.enter()
            _ = provider.loadDataRepresentation(forTypeIdentifier: type) { data, _ in
                if let data { lock.lock(); images[i] = data; lock.unlock() }
                group.leave()
            }
        }
        group.notify(queue: .main) { [weak self] in
            let found = images.keys.sorted().compactMap { images[$0] }
            if !found.isEmpty { self?.onPasteImages(found) }
        }
    }
}
