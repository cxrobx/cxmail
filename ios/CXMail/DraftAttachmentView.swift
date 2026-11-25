import PDFKit
import QuickLook
import SwiftUI
import UniformTypeIdentifiers

struct DraftAttachment: Identifiable {
    let index: Int
    let name: String
    let ref: MessageRef
    let digest: String
    var id: Int { index }

    var query: [URLQueryItem] {
        ref.query + [.init(name: "digest", value: digest), .init(name: "index", value: String(index))]
    }

    var localName: String {
        let invalid = CharacterSet.controlCharacters.union(CharacterSet(charactersIn: "/\\:"))
        let cleaned = name.components(separatedBy: invalid).joined(separator: "_")
        if cleaned.utf8.count > 240 {
            let shortened = String(decoding: cleaned.utf8.prefix(180), as: UTF8.self)
            let suffix = String(decoding: (cleaned as NSString).pathExtension.utf8.prefix(24), as: UTF8.self)
            return suffix.isEmpty ? "Attachment" : shortened + "." + suffix
        }
        return cleaned.isEmpty || cleaned == "." || cleaned == ".." ? "Attachment" : cleaned
    }
}

struct DraftAttachmentTile: View {
    let item: DraftAttachment

    private var fileExtension: String { (item.name as NSString).pathExtension.lowercased() }
    private var fileType: UTType? { UTType(filenameExtension: fileExtension) }

    private var format: String {
        switch fileExtension {
        case "doc", "docx", "docm": "Word document"
        case "pdf": "PDF"
        case "xlsx": "Excel workbook"
        case "pptx": "PowerPoint presentation"
        default: fileType?.localizedDescription ?? "File"
        }
    }

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: fileType?.conforms(to: .image) == true ? "photo" : "doc.text.fill")
                .font(.title3)
                .foregroundStyle(Color.accentColor)
                .frame(width: 36, height: 36)
                .background(Color.accentColor.opacity(0.12), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 3) {
                Text(item.name).font(.subheadline.weight(.semibold)).foregroundStyle(.primary)
                    .fixedSize(horizontal: false, vertical: true)
                Text(format).font(.caption).foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
            Image(systemName: "chevron.right")
                .font(.caption.weight(.semibold))
                .foregroundStyle(Color.accentColor)
                .accessibilityHidden(true)
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary.opacity(0.55), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .contentShape(Rectangle())
    }
}

struct DraftAttachmentView: View {
    let api: APIClient
    let item: DraftAttachment
    let invalidated: (MailError) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var file: URL?
    @State private var pagePreview: URL?
    @State private var preparingPages = false
    @State private var error: String?
    @State private var attempt = 0

    var body: some View {
        NavigationStack {
            Group {
                if preparingPages {
                    ProgressView("Preparing preview…")
                } else if let file {
                    if let pagePreview {
                        OfficePagePreview(file: pagePreview)
                            .accessibilityIdentifier("wordPagePreview")
                    } else if canPreview(file) {
                        AttachmentQuickLook(file: file)
                            .accessibilityIdentifier("draftAttachmentPreview")
                    } else {
                        ContentUnavailableView("Preview unavailable", systemImage: "doc", description: Text("Use Share to save or open this file."))
                    }
                } else if let error {
                    VStack(spacing: 12) {
                        ContentUnavailableView("Unable to open attachment", systemImage: "exclamationmark.circle", description: Text(error))
                        Button("Try Again") { attempt += 1 }
                    }
                } else {
                    ProgressView("Opening attachment…")
                }
            }
            .navigationTitle(item.name)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }.accessibilityIdentifier("closeDraftAttachment")
                }
                if let file {
                    ToolbarItem(placement: .primaryAction) {
                        ShareLink(item: file).accessibilityIdentifier("shareDraftAttachment")
                    }
                }
            }
        }
        .task(id: attempt) { await load() }
        .onDisappear {
            if let file { try? FileManager.default.removeItem(at: file.deletingLastPathComponent()) }
        }
    }

    private func canPreview(_ file: URL) -> Bool {
        // Documents and passive media use native Quick Look. Hand web formats
        // and other files to the system share sheet on explicit user action.
        guard let type = UTType(filenameExtension: file.pathExtension),
              !type.conforms(to: .html), !type.conforms(to: .svg), !type.conforms(to: .xml)
        else { return false }
        return QLPreviewController.canPreview(file as NSURL)
    }

    private func load() async {
        error = nil
        do {
            let data = try await api.draftAttachment(query: item.query)
            try Task.checkCancellation()
            let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
            let url = directory.appendingPathComponent(item.localName, isDirectory: false)
            do {
                try data.write(to: url, options: [.atomic, .completeFileProtection])
                file = url
            } catch {
                try? FileManager.default.removeItem(at: directory)
                throw error
            }
            let formats = ["docx": "word", "xlsx": "xlsx", "pptx": "pptx"]
            if let format = formats[url.pathExtension.lowercased()] {
                await prepareOfficePreview(in: directory, format: format)
            }
        } catch is CancellationError {
            return
        } catch MailError.reviewExpired {
            if !Task.isCancelled { invalidated(.reviewExpired) }
        } catch MailError.draftUnavailable {
            if !Task.isCancelled { invalidated(.draftUnavailable) }
        } catch {
            if !Task.isCancelled { self.error = error.localizedDescription }
        }
    }
    private func prepareOfficePreview(in directory: URL, format: String) async {
        preparingPages = true
        defer { preparingPages = false }
        do {
            let pdf = try await api.officeAttachmentPreview(query: item.query, format: format)
            try Task.checkCancellation()
            let url = directory.appendingPathComponent("Page Preview.pdf")
            try pdf.write(to: url, options: [.atomic, .completeFileProtection])
            pagePreview = url
        } catch MailError.reviewExpired {
            if !Task.isCancelled { invalidated(.reviewExpired) }
        } catch MailError.draftUnavailable {
            if !Task.isCancelled { invalidated(.draftUnavailable) }
        } catch {
            // Preview the original file when Office conversion fails.
        }
    }

}

private struct OfficePagePreview: UIViewRepresentable {
    let file: URL

    func makeUIView(context: Context) -> PDFView {
        let view = PDFView()
        view.displayMode = .singlePageContinuous
        view.displayDirection = .vertical
        view.backgroundColor = .systemGroupedBackground
        view.document = PDFDocument(url: file)
        view.autoScales = true
        return view
    }

    func updateUIView(_ view: PDFView, context: Context) {}
}

private struct AttachmentQuickLook: UIViewControllerRepresentable {
    let file: URL

    func makeCoordinator() -> PreviewSource { PreviewSource(file: file) }

    func makeUIViewController(context: Context) -> QLPreviewController {
        let controller = QLPreviewController()
        controller.dataSource = context.coordinator
        return controller
    }

    func updateUIViewController(_ controller: QLPreviewController, context: Context) {}

    final class PreviewSource: NSObject, QLPreviewControllerDataSource {
        let file: URL
        init(file: URL) { self.file = file }

        func numberOfPreviewItems(in controller: QLPreviewController) -> Int { 1 }
        func previewController(_ controller: QLPreviewController, previewItemAt index: Int) -> QLPreviewItem { file as NSURL }
    }
}
