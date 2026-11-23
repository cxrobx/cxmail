import ImageIO
import PhotosUI
import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// A photo or file added on the phone. Photos are re-encoded as JPEG so HEIC,
/// Live Photo stills and huge screenshots all leave the phone the same way;
/// files keep their bytes and name.
struct ComposePhoto: Identifiable, Equatable {
    let id: UUID
    let data: Data
    let name: String
    let thumbnail: UIImage?
    var isImage: Bool { thumbnail != nil }

    init?(jpeg data: Data, id: UUID = UUID()) {
        guard let thumb = PhotoEncoder.downscaled(data, maxPixel: 240).flatMap(UIImage.init(data:)) else { return nil }
        self.id = id; self.data = data; self.name = "photo.jpg"; self.thumbnail = thumb
    }
    init(file data: Data, name: String, id: UUID = UUID()) {
        self.id = id; self.data = data; self.name = name; self.thumbnail = nil
    }
    static func == (a: Self, b: Self) -> Bool { a.id == b.id }
}

enum PhotoEncoder {
    /// Mail keeps detail; chat matches the size Claude reads images at.
    static let mailPixels: CGFloat = 2560
    static let chatPixels: CGFloat = 1568
    /// compose_draft caps each file at 25 MB; keep the whole upload under the mini's 48 MiB request cap.
    static let maxFileBytes = 20 * 1024 * 1024
    static let maxTotalBytes = 30 * 1024 * 1024

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

    /// Adds pasted or picked image bytes, respecting the count limit. Returns an error to show, if any.
    @MainActor static func add(images: [Data], to photos: inout [ComposePhoto], limit: Int, maxPixel: CGFloat) -> String? {
        var failed = false
        for data in images {
            guard photos.count < limit else { return "You can add up to \(limit) attachments." }
            if let photo = photo(from: data, maxPixel: maxPixel) { photos.append(photo) } else { failed = true }
        }
        return failed ? "That image couldn’t be added." : nil
    }
}

/// Library and Files buttons shared by the mail composer and chat. Pasting
/// happens in the text itself (`PastingTextView`).
struct PhotoAddControls: View {
    @Binding var photos: [ComposePhoto]
    let limit: Int
    let maxPixel: CGFloat
    /// Mail attaches any file; chat only takes images (all Claude can read).
    var anyFile = false
    /// Chat shows one + menu (Photo Library / Choose Files) as T3's composer does.
    var menu = false
    var onError: (String) -> Void = { _ in }
    @State private var picked: [PhotosPickerItem] = []
    @State private var importing = false
    @State private var pickingPhotos = false

    var body: some View {
        Group {
            if menu {
                Menu {
                    Button("Photo Library", systemImage: "photo.on.rectangle") { pickingPhotos = true }
                        .accessibilityIdentifier("addPhoto")
                    Button("Choose Files", systemImage: "folder") { importing = true }
                        .accessibilityIdentifier("addFile")
                } label: {
                    Image(systemName: "plus").font(.system(size: 20, weight: .medium))
                        .frame(width: 44, height: 44).contentShape(Rectangle())
                }
                .disabled(photos.count >= limit)
                .accessibilityLabel("Add attachment").accessibilityIdentifier("addAttachment")
                .photosPicker(isPresented: $pickingPhotos, selection: $picked,
                              maxSelectionCount: max(1, limit - photos.count), matching: .images)
            } else {
                HStack(spacing: 4) {
                    PhotosPicker(selection: $picked, maxSelectionCount: max(1, limit - photos.count), matching: .images) {
                        Image(systemName: "photo.on.rectangle")
                            .font(.body.weight(.medium))
                            .frame(width: 44, height: 44)
                            .contentShape(Rectangle())
                    }
                    .disabled(photos.count >= limit)
                    .accessibilityLabel("Add photo")
                    .accessibilityIdentifier("addPhoto")
                    Button { importing = true } label: {
                        Image(systemName: "paperclip")
                            .font(.body.weight(.medium))
                            .frame(width: 44, height: 44)
                            .contentShape(Rectangle())
                    }
                    .disabled(photos.count >= limit)
                    .accessibilityLabel(anyFile ? "Attach file" : "Add image from Files")
                    .accessibilityIdentifier("addFile")
                }
            }
        }
        .fileImporter(isPresented: $importing, allowedContentTypes: anyFile ? [.item] : [.image],
                      allowsMultipleSelection: true) { result in
            switch result {
            case .success(let urls): addFiles(urls)
            case .failure: onError("That file couldn’t be opened.")
            }
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

    private func addFiles(_ urls: [URL]) {
        for url in urls {
            guard photos.count < limit else { onError("You can add up to \(limit) attachments."); return }
            let scoped = url.startAccessingSecurityScopedResource()
            defer { if scoped { url.stopAccessingSecurityScopedResource() } }
            guard let data = try? Data(contentsOf: url) else { onError("\(url.lastPathComponent) couldn’t be read."); continue }
            // Chat can only carry images; mail keeps whatever file was chosen, as is.
            if !anyFile {
                if let photo = PhotoEncoder.photo(from: data, maxPixel: maxPixel) { photos.append(photo) }
                else { onError("\(url.lastPathComponent) isn’t an image Claude can read.") }
                continue
            }
            guard data.count <= PhotoEncoder.maxFileBytes else {
                onError("\(url.lastPathComponent) is over 20 MB."); continue
            }
            guard photos.reduce(0, { $0 + $1.data.count }) + data.count <= PhotoEncoder.maxTotalBytes else {
                onError("Attachments are limited to 30 MB together."); continue
            }
            photos.append(ComposePhoto(file: data, name: url.lastPathComponent))
        }
    }
}

/// Removable thumbnails (photos) and chips (files) that will go with the message.
struct PhotoStrip: View {
    @Binding var photos: [ComposePhoto]
    var size: CGFloat = 64

    var body: some View {
        ScrollView(.horizontal) {
            HStack(spacing: 8) {
                ForEach(photos) { photo in
                    tile(photo)
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
                            .accessibilityLabel("Remove \(photo.isImage ? "photo" : photo.name)")
                            .accessibilityIdentifier("removePhoto")
                        }
                        .accessibilityElement(children: .contain)
                        .accessibilityLabel(photo.isImage ? "Photo" : photo.name)
                        .accessibilityIdentifier(photo.isImage ? "composePhoto" : "composeFile")
                }
            }
            .padding(.top, 8).padding(.trailing, 8)
        }
        .scrollIndicators(.hidden)
    }

    @ViewBuilder private func tile(_ photo: ComposePhoto) -> some View {
        if let thumbnail = photo.thumbnail {
            Image(uiImage: thumbnail)
                .resizable().scaledToFill()
                .frame(width: size, height: size)
                .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
        } else {
            VStack(spacing: 4) {
                Image(systemName: "doc.fill").font(.title3).foregroundStyle(Color.accentColor)
                Text(photo.name).font(.caption2).lineLimit(2).multilineTextAlignment(.center)
            }
            .padding(6)
            .frame(width: size * 1.6, height: size)
            .background(Color(uiColor: .tertiarySystemFill), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        }
    }
}
