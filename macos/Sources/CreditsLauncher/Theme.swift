import AppKit
import SwiftUI

/// Rosé Pine, matching the web app's default theme (static/app.css).
enum Theme {
    static let bg = Color(hex: 0x191724)
    static let card = Color(hex: 0x1f1d2e)
    static let line = Color(hex: 0x403d52)
    static let fg = Color(hex: 0xe0def4)
    static let muted = Color(hex: 0x908caa)
    static let subtle = Color(hex: 0x6e6a86)
    static let accent = Color(hex: 0xc4a7e7)  // iris
    static let movie = Color(hex: 0xf6c177)   // gold
    static let tv = Color(hex: 0xebbcba)      // rose
    static let person = Color(hex: 0x9ccfd8)  // foam
    static let love = Color(hex: 0xeb6f92)

    static func color(for kind: Kind) -> Color {
        switch kind {
        case .movie: return movie
        case .tv: return tv
        case .person: return person
        }
    }

    /// The app icon's bars as a menu bar template image.
    static var menuBarIcon: NSImage {
        let image = NSImage(size: NSSize(width: 18, height: 18), flipped: true) { _ in
            NSColor.black.setFill()
            for (x, y, w, alpha) in [(4.5, 2.0, 9.0, 0.45), (2.5, 6.0, 13.0, 0.7), (5.0, 10.0, 8.0, 0.9), (3.5, 14.0, 11.0, 1.0)] {
                NSColor.black.withAlphaComponent(alpha).setFill()
                NSBezierPath(roundedRect: NSRect(x: x, y: y, width: w, height: 2.4), xRadius: 1.2, yRadius: 1.2).fill()
            }
            return true
        }
        image.isTemplate = true
        return image
    }
}

extension Color {
    init(hex: UInt32) {
        self.init(.sRGB,
                  red: Double((hex >> 16) & 0xff) / 255,
                  green: Double((hex >> 8) & 0xff) / 255,
                  blue: Double(hex & 0xff) / 255)
    }
}
