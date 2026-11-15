// Adapt CXMail's actual desktop artwork to iOS's opaque, system-masked icon.
// The crop removes only the macOS icon's transparent outside margin.
// Run from ios/: swift scripts/icon.swift
import Foundation
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers
let size = 1024
let sourceURL = URL(fileURLWithPath: "../src-tauri/icons/icon_source.png")
let source = CGImageSourceCreateWithURL(sourceURL as CFURL, nil)!
let artwork = CGImageSourceCreateImageAtIndex(source, 0, nil)!
let cropped = artwork.cropping(to: CGRect(x: 96, y: 96, width: 832, height: 832))!
let context = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8,
    bytesPerRow: size * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
context.setFillColor(CGColor(red: 39.0/255, green: 39.0/255, blue: 43.0/255, alpha: 1))
context.fill(CGRect(x: 0, y: 0, width: size, height: size))
context.interpolationQuality = .high
context.draw(cropped, in: CGRect(x: 0, y: 0, width: size, height: size))
let directory = URL(fileURLWithPath: "CXMail/Assets.xcassets/AppIcon.appiconset")
try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
let destination = CGImageDestinationCreateWithURL(directory.appendingPathComponent("AppIcon.png") as CFURL, UTType.png.identifier as CFString, 1, nil)!
CGImageDestinationAddImage(destination, context.makeImage()!, nil)
precondition(CGImageDestinationFinalize(destination))
