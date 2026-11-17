import AppKit
import Observation
import SwiftUI

/// The four themes of the web, terminal and iOS clients, named as in the
/// config file's `theme` key.
enum ThemeName: String, CaseIterable {
    case rosePine = "rose-pine"
    case tokyoNight = "tokyo-night"
    case catppuccin
    /// Light or dark with the system.
    case classic

    var title: String {
        switch self {
        case .rosePine: return "Rosé Pine"
        case .tokyoNight: return "Tokyo Night"
        case .catppuccin: return "Catppuccin"
        case .classic: return "Classic"
        }
    }

    /// Palettes as in static/app.css; `subtle` is the terminal's dim.
    var palette: Palette {
        switch self {
        case .rosePine:
            return Palette(bg: 0x191724, card: 0x1f1d2e, line: 0x403d52, fg: 0xe0def4, muted: 0x908caa,
                           subtle: 0x6e6a86, accent: 0xc4a7e7, movie: 0xf6c177, tv: 0xebbcba,
                           person: 0x9ccfd8, love: 0xeb6f92)
        case .tokyoNight:
            return Palette(bg: 0x1a1b26, card: 0x24283b, line: 0x3b4261, fg: 0xc0caf5, muted: 0x828bb8,
                           subtle: 0x565f89, accent: 0x7aa2f7, movie: 0xff9e64, tv: 0x7dcfff,
                           person: 0x9ece6a, love: 0xf7768e)
        case .catppuccin:
            return Palette(bg: 0x1e1e2e, card: 0x313244, line: 0x45475a, fg: 0xcdd6f4, muted: 0xa6adc8,
                           subtle: 0x7f849c, accent: 0xcba6f7, movie: 0xfab387, tv: 0x74c7ec,
                           person: 0xa6e3a1, love: 0xf38ba8)
        case .classic:
            return Palette(bg: .init(0xfafaf9, 0x1c1917), card: .init(0xffffff, 0x292524),
                           line: .init(0xe7e5e4, 0x44403c), fg: .init(0x1c1917, 0xf5f5f4),
                           muted: .init(0x78716c, 0xa8a29e), subtle: .init(0xa8a29e, 0x78716c),
                           accent: .init(0x0d6efd, 0x6ea8fe), movie: .init(0x9a3412, 0xfdba74),
                           tv: .init(0x1d4ed8, 0x93c5fd), person: .init(0x047857, 0x6ee7b7),
                           love: .init(0xc2410c, 0xfdba74))
        }
    }

    /// Classic follows the system; the others are dark.
    var appearance: NSAppearance? {
        self == .classic ? nil : NSAppearance(named: .darkAqua)
    }
}

/// A light and a dark hex color; the dark themes use the same for both.
struct Shade: ExpressibleByIntegerLiteral {
    let light: UInt32
    let dark: UInt32

    init(_ light: UInt32, _ dark: UInt32) { self.light = light; self.dark = dark }
    init(integerLiteral hex: UInt32) { self.init(hex, hex) }

    var color: Color {
        if light == dark { return Color(hex: light) }
        let (light, dark) = (light, dark)
        return Color(nsColor: NSColor(name: nil) { appearance in
            let isDark = appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
            return NSColor(Color(hex: isDark ? dark : light))
        })
    }
}

struct Palette {
    let bg, card, line, fg, muted, subtle, accent, movie, tv, person, love: Shade
}

/// The chosen theme: the menu bar menu's pick, else the config file's
/// `theme` key (Rosé Pine if absent or unknown, as in the terminal client).
/// The pick is a launcher preference rather than a write to the config file,
/// which may be generated (e.g. by Nix) and read-only.
@Observable
final class ThemeStore {
    static let shared = ThemeStore()
    private static let key = "theme"

    private(set) var configured = ThemeStore.fromConfig()
    private(set) var picked = UserDefaults.standard.string(forKey: key).flatMap(ThemeName.init(rawValue:))
    var name: ThemeName { picked ?? configured }

    private static func fromConfig() -> ThemeName {
        CreditsConfig.value("theme").flatMap(ThemeName.init(rawValue:)) ?? .rosePine
    }

    /// Picks up edits made to the file since.
    func reload() {
        let latest = Self.fromConfig()
        if latest != configured { configured = latest }
    }

    /// A theme for the launcher alone, or nil to follow the config file.
    func pick(_ theme: ThemeName?) {
        picked = theme
        UserDefaults.standard.set(theme?.rawValue, forKey: Self.key)
    }
}

/// The current theme's colors. Reading them in a view's body observes the
/// theme, so views redraw when it changes.
enum Theme {
    private static var palette: Palette { ThemeStore.shared.name.palette }
    static var bg: Color { palette.bg.color }
    static var card: Color { palette.card.color }
    static var line: Color { palette.line.color }
    static var fg: Color { palette.fg.color }
    static var muted: Color { palette.muted.color }
    static var subtle: Color { palette.subtle.color }
    static var accent: Color { palette.accent.color }
    static var movie: Color { palette.movie.color }
    static var tv: Color { palette.tv.color }
    static var person: Color { palette.person.color }
    static var love: Color { palette.love.color }

    static func color(for kind: Kind) -> Color {
        switch kind {
        case .movie: return movie
        case .tv, .season, .episode: return tv
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
