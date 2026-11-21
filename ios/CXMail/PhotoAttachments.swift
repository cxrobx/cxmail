import ImageIO
import PhotosUI
import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// A photo added on the phone. Its bytes are already re-encoded as JPEG, so
/// HEIC, Live Photo stills and huge screenshots all leave the phone the same way.
struct ComposePhoto: Identifiable, Equatable {
    let id: UUID
    let data: Data
    let thumbnail: UIImage

    init?(jpeg data: Data, id: UUID = UUID()) {
        guard let thumb = PhotoEncoder.downscaled(data, maxPixel: 240).flatMap(UIImage.init(data:)) else { return nil }
        self.id = id; self.data = data; self.thumbnail = thumb
    }
    static func == (a: Self, b: Self) -> Bool { a.id == b.id }
}

enum PhotoEncoder {
    /// Mail keeps detail; chat matches the size Claude reads images at.
    static let mailPixels: CGFloat = 2560
    static let chatPixels: CGFloat = 1568

    static func downscaled(_ data: Data, maxPixel: CGFloat, quality: CGFloat = 0.82) -> Data? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixel,
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }
        return UIImage(cgImage: image).jpegData(compressionQuality: quality)
    }

    static func photo(from data: Data, maxPixel: CGFloat) -> ComposePhoto? {
        downscaled(data, maxPixel: maxPixel).flatMap { ComposePhoto(jpeg: $0) }
    }
}

/// Watches the system pasteboard for an image without reading it, so no
/// paste prompt appears until the person taps Paste.
@MainActor @Observable
final class PasteboardWatcher {
    var hasImage = UIPasteboard.general.hasImages
    @ObservationIgnored private var observers: [NSObjectProtocol] = []
    init() {
        let names = [UIPasteboard.changedNotification, UIApplication.didBecomeActiveNotification]
        observers = names.map { name in
            NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.hasImage = UIPasteboard.general.hasImages }
            }
        }
    }
    deinit { observers.forEach(NotificationCenter.default.removeObserver) }
}

/// The add-photo affordances shared by the mail composer and chat: a library
/// picker, and the system Paste button whenever the clipboard holds an image.
struct PhotoAddControls: View {
    @Binding var photos: [ComposePhoto]
    let limit: Int
    let maxPixel: CGFloat
    var onError: (String) -> Void = { _ in }
    @State private var picked: [PhotosPickerItem] = []
    @State private var pasteboard = PasteboardWatcher()

    var body: some View {
        HStack(spacing: 8) {
            PhotosPicker(selection: $picked, maxSelectionCount: max(1, limit - photos.count), matching: .images) {
                Image(systemName: "photo.on.rectangle")
                    .font(.body.weight(.medium))
                    .frame(width: 44, height: 44)
                    .contentShape(Rectangle())
            }
            .disabled(photos.count >= limit)
            .accessibilityLabel("Add photo")
            .accessibilityIdentifier("addPhoto")
            if pasteboard.hasImage && photos.count < limit { pasteControl.fixedSize() }
        }
        .onChange(of: picked) { _, items in
            guard !items.isEmpty else { return }
            picked = []
            Task {
                for item in items {
                    guard photos.count < limit else { break }
                    if let data = try? await item.loadTransferable(type: Data.self),
                       let photo = PhotoEncoder.photo(from: data, maxPixel: maxPixel) {
                        photos.append(photo)
                    } else { onError("That photo couldn’t be added. Try another one.") }
                }
            }
        }
    }

    /// The system Paste button reads the clipboard without the "Allow Paste"
    /// prompt, and for that reason ignores synthesized taps. UI-test fixtures get
    /// a plain button feeding the same `paste` path from the same clipboard.
    @ViewBuilder private var pasteControl: some View {
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("--uitesting") {
            Button("Paste", systemImage: "doc.on.clipboard") { paste(UIPasteboard.general.itemProviders) }
                .buttonStyle(.bordered).buttonBorderShape(.capsule).controlSize(.small)
                .accessibilityIdentifier("pastePhoto")
        } else { systemPaste }
        #else
        systemPaste
        #endif
    }
    private var systemPaste: some View {
        PasteButton(supportedContentTypes: [.image]) { providers in paste(providers) }
            .labelStyle(.titleAndIcon)
            .buttonBorderShape(.capsule)
            .controlSize(.small)
            .tint(.accentColor)
            .accessibilityIdentifier("pastePhoto")
    }

    private func paste(_ providers: [NSItemProvider]) {
        let images = providers.filter { p in
            p.canLoadObject(ofClass: UIImage.self) || p.registeredTypeIdentifiers.contains { UTType($0)?.conforms(to: .image) == true }
        }
        guard !images.isEmpty else { onError("The clipboard doesn’t hold an image."); return }
        for provider in images.prefix(max(0, limit - photos.count)) {
            Self.imageData(from: provider) { data in
                let photo = data.flatMap { PhotoEncoder.photo(from: $0, maxPixel: maxPixel) }
                Task { @MainActor in
                    if let photo, photos.count < limit { photos.append(photo) }
                    else if photo == nil { onError("The clipboard image couldn’t be added.") }
                }
            }
        }
    }

    /// Ask for the image in the exact type the clipboard holds (PNG, HEIC, JPEG…);
    /// a generic `public.image` request is not always coerced. Fall back to UIImage.
    nonisolated private static func imageData(from provider: NSItemProvider, completion: @escaping @Sendable (Data?) -> Void) {
        let asObject = {
            _ = provider.loadObject(ofClass: UIImage.self) { object, _ in
                completion((object as? UIImage)?.pngData())
            }
        }
        guard let type = provider.registeredTypeIdentifiers.first(where: { UTType($0)?.conforms(to: .image) == true }) else {
            asObject(); return
        }
        _ = provider.loadDataRepresentation(forTypeIdentifier: type) { data, _ in
            if let data, !data.isEmpty { completion(data) } else { asObject() }
        }
    }
}

/// Removable thumbnails for photos that will go with the message.
struct PhotoStrip: View {
    @Binding var photos: [ComposePhoto]
    var size: CGFloat = 64

    var body: some View {
        ScrollView(.horizontal) {
            HStack(spacing: 8) {
                ForEach(photos) { photo in
                    Image(uiImage: photo.thumbnail)
                        .resizable().scaledToFill()
                        .frame(width: size, height: size)
                        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                        .overlay(alignment: .topTrailing) {
                            Button { photos.removeAll { $0.id == photo.id } } label: {
                                Image(systemName: "xmark.circle.fill")
                                    .symbolRenderingMode(.palette)
                                    .foregroundStyle(.white, .black.opacity(0.6))
                                    .font(.title3)
                                    .frame(width: 44, height: 44, alignment: .topTrailing)
                                    .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .offset(x: 6, y: -6)
                            .accessibilityLabel("Remove photo")
                            .accessibilityIdentifier("removePhoto")
                        }
                        .accessibilityElement(children: .contain)
                        .accessibilityLabel("Photo")
                        .accessibilityIdentifier("composePhoto")
                }
            }
            .padding(.top, 8).padding(.trailing, 8)
        }
        .scrollIndicators(.hidden)
    }
}
