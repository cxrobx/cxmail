// Render the existing CXMail Remote envelope as an opaque iOS icon.
// Run from ios/: swift scripts/icon.swift
import Foundation
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers
let size = 1024
let context = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8,
    bytesPerRow: size * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
context.setFillColor(CGColor(red: 47.0/255, green: 111.0/255, blue: 222.0/255, alpha: 1))
context.fill(CGRect(x: 0, y: 0, width: size, height: size))
context.setStrokeColor(CGColor(gray: 1, alpha: 1)); context.setLineWidth(56); context.setLineJoin(.round)
context.stroke(CGRect(x: 224, y: 320, width: 576, height: 384))
context.move(to: .init(x: 224, y: 704)); context.addLine(to: .init(x: 512, y: 480)); context.addLine(to: .init(x: 800, y: 704)); context.strokePath()
let directory = URL(fileURLWithPath: "CXMail/Assets.xcassets/AppIcon.appiconset")
try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
let destination = CGImageDestinationCreateWithURL(directory.appendingPathComponent("AppIcon.png") as CFURL, UTType.png.identifier as CFString, 1, nil)!
CGImageDestinationAddImage(destination, context.makeImage()!, nil)
precondition(CGImageDestinationFinalize(destination))
