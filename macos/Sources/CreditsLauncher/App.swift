import AppKit
import Carbon.HIToolbox
import ServiceManagement
import SwiftUI

/// A borderless floating panel that can take keyboard focus without
/// activating the app's (non-existent) main window, like Spotlight.
final class LauncherPanel: NSPanel {
    init() {
        super.init(contentRect: NSRect(x: 0, y: 0, width: 720, height: 480),
                   styleMask: [.borderless, .nonactivatingPanel, .fullSizeContentView],
                   backing: .buffered, defer: false)
        level = .floating
        isFloatingPanel = true
        isOpaque = false
        backgroundColor = .clear
        hasShadow = true
        hidesOnDeactivate = false
        isMovableByWindowBackground = true
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient]
    }

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { true }

    /// The app usually isn't active while the panel has focus, so AppKit
    /// may skip the main menu; route ⌘C/⌘V/etc. to it directly.
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        if NSApp.mainMenu?.performKeyEquivalent(with: event) == true { return true }
        return super.performKeyEquivalent(with: event)
    }
}

/// ⌥Space via Carbon's RegisterEventHotKey, which needs no Accessibility
/// permission.
final class HotKey {
    private var ref: EventHotKeyRef?
    private let action: () -> Void

    init(keyCode: Int, modifiers: Int, action: @escaping () -> Void) {
        self.action = action
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        InstallEventHandler(GetApplicationEventTarget(), { _, _, userData in
            let me = Unmanaged<HotKey>.fromOpaque(userData!).takeUnretainedValue()
            DispatchQueue.main.async { me.action() }
            return noErr
        }, 1, &spec, Unmanaged.passUnretained(self).toOpaque(), nil)
        let id = EventHotKeyID(signature: OSType(0x4352_4544), id: 1)  // "CRED"
        let status = RegisterEventHotKey(UInt32(keyCode), UInt32(modifiers), id,
                                         GetApplicationEventTarget(), 0, &ref)
        if status != noErr {
            NSLog("CreditsLauncher: couldn't register ⌥Space (status \(status)); another app may own it")
        }
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate, NSMenuDelegate {
    let model = LauncherModel()
    private var panel: LauncherPanel!
    private var statusItem: NSStatusItem!
    private var hotKey: HotKey?
    private var keyMonitor: Any?
    private var loginItem: NSMenuItem!
    private var themeMenu: NSMenu!

    func applicationDidFinishLaunching(_ notification: Notification) {
        panel = LauncherPanel()
        panel.delegate = self
        let host = NSHostingView(rootView: LauncherView(model: model))
        host.wantsLayer = true
        host.layer?.cornerRadius = 14
        host.layer?.masksToBounds = true
        panel.contentView = host
        applyTheme()
        model.close = { [weak self] in self?.hide() }

        // Never shown (the app has no menu bar), but routes ⌘A/⌘C/⌘V/⌘Z to the search field.
        let edit = NSMenu(title: "Edit")
        edit.addItem(withTitle: "Undo", action: Selector(("undo:")), keyEquivalent: "z")
        edit.addItem(withTitle: "Redo", action: Selector(("redo:")), keyEquivalent: "Z")
        edit.addItem(withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        edit.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        edit.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        edit.addItem(withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
        let mainMenu = NSMenu()
        mainMenu.addItem(withTitle: "Edit", action: nil, keyEquivalent: "").submenu = edit
        NSApp.mainMenu = mainMenu

        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        statusItem.button?.image = Theme.menuBarIcon
        let menu = NSMenu()
        menu.addItem(withTitle: "Search Credits", action: #selector(show), keyEquivalent: " ")
            .keyEquivalentModifierMask = .option
        menu.addItem(withTitle: "Open Credits in Browser", action: #selector(openSite), keyEquivalent: "")
        menu.addItem(.separator())
        themeMenu = NSMenu(title: "Theme")
        themeMenu.addItem(withTitle: "From config.toml", action: #selector(chooseTheme(_:)), keyEquivalent: "")
        themeMenu.addItem(.separator())
        for theme in ThemeName.allCases {
            let item = themeMenu.addItem(withTitle: theme.title, action: #selector(chooseTheme(_:)), keyEquivalent: "")
            item.representedObject = theme.rawValue
        }
        menu.addItem(withTitle: "Theme", action: nil, keyEquivalent: "").submenu = themeMenu
        menu.delegate = self
        loginItem = menu.addItem(withTitle: "Open at Login", action: #selector(toggleLoginItem), keyEquivalent: "")
        loginItem.isHidden = loginItemManaged
        menu.addItem(.separator())
        menu.addItem(withTitle: "Quit", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        statusItem.menu = menu

        hotKey = HotKey(keyCode: kVK_Space, modifiers: optionKey) { [weak self] in
            Task { @MainActor in self?.toggle() }
        }

        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard let self, event.window === self.panel else { return event }
            return self.handleKey(event) ? nil : event
        }

        enableLoginItemOnFirstInstall()
        loginItem.state = SMAppService.mainApp.status == .enabled ? .on : .off

        if !launchedAsLoginItem && !loginItemManaged { show() }
    }

    /// Set in the Info.plist of Nix builds, which an installer starts at login
    /// with launchd: SMAppService would register a store path that goes stale
    /// on the next rebuild.
    private var loginItemManaged: Bool {
        Bundle.main.object(forInfoDictionaryKey: "CreditsLoginItemManaged") as? Bool ?? false
    }

    /// Turns on Open at Login the first time the app runs from an
    /// Applications folder (not from a build directory). After that it's
    /// left to the menu's toggle.
    private func enableLoginItemOnFirstInstall() {
        let key = "loginItemConfigured"
        guard !loginItemManaged, !UserDefaults.standard.bool(forKey: key),
              Bundle.main.bundlePath.contains("/Applications/") else { return }
        do {
            try SMAppService.mainApp.register()
            UserDefaults.standard.set(true, forKey: key)
        } catch {
            NSLog("CreditsLauncher: couldn't enable Open at Login: \(error)")
        }
    }

    private var launchedAsLoginItem: Bool {
        let event = NSAppleEventManager.shared().currentAppleEvent
        return event?.eventID == kAEOpenApplication
            && event?.paramDescriptor(forKeyword: keyAEPropData)?.enumCodeValue == keyAELaunchedAsLogInItem
    }

    @objc func toggleLoginItem() {
        let service = SMAppService.mainApp
        do {
            if service.status == .enabled { try service.unregister() } else { try service.register() }
        } catch {
            NSLog("CreditsLauncher: couldn't change Open at Login: \(error)")
        }
        UserDefaults.standard.set(true, forKey: "loginItemConfigured")
        loginItem.state = service.status == .enabled ? .on : .off
    }

    func menuWillOpen(_ menu: NSMenu) {
        ThemeStore.shared.reload()
        applyTheme()
    }

    @objc func chooseTheme(_ sender: NSMenuItem) {
        ThemeStore.shared.pick((sender.representedObject as? String).flatMap(ThemeName.init(rawValue:)))
        applyTheme()
    }

    /// Matches the panel's appearance and the menu to the theme.
    private func applyTheme() {
        let store = ThemeStore.shared
        panel.appearance = store.name.appearance
        guard let themeMenu, let follow = themeMenu.items.first else { return }
        follow.title = "From config.toml (\(store.configured.title))"
        follow.state = store.picked == nil ? .on : .off
        for item in themeMenu.items.dropFirst() {
            item.state = item.representedObject as? String == store.picked?.rawValue ? .on : .off
        }
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        show()
        return false
    }

    private func handleKey(_ event: NSEvent) -> Bool {
        let commandOnly = event.modifierFlags.intersection([.command, .shift, .option, .control]) == .command
        // An image open full size takes every key; esc, space or ⌘Y closes it.
        if model.viewing != nil {
            switch Int(event.keyCode) {
            case kVK_Escape, kVK_Space: model.viewing = nil
            case kVK_ANSI_Y where commandOnly: model.viewing = nil
            default: break
            }
            return true
        }
        if commandOnly, Int(event.keyCode) == kVK_ANSI_Y, model.detailHit != nil {
            if let image = model.pageImage { model.view(image.url, image.label) }
            return true
        }
        if commandOnly, Int(event.keyCode) == kVK_ANSI_P, model.canPeek {
            model.peeking.toggle()
            return true
        }
        if commandOnly, Int(event.keyCode) == kVK_ANSI_R, model.detailTags != nil {
            model.focusTags += 1
            return true
        }
        // The tag field takes ⏎ and the arrows; esc hands the keys back to search.
        if model.editingTags {
            switch Int(event.keyCode) {
            case kVK_Escape: model.focusSearch += 1; return true
            case kVK_Return, kVK_ANSI_KeypadEnter, kVK_UpArrow, kVK_DownArrow: return false
            default: break
            }
        }
        if commandOnly, Int(event.keyCode) == kVK_ANSI_LeftBracket, model.detailHit != nil {
            model.goBack()
            return true
        }
        // ⌘[ and ⌘] turn a paged list's pages, as back and forward do in a browser.
        if commandOnly, model.detailHit == nil, model.canTurnPage,
           [kVK_ANSI_LeftBracket, kVK_ANSI_RightBracket].contains(Int(event.keyCode)) {
            model.turnPage(Int(event.keyCode) == kVK_ANSI_LeftBracket ? -1 : 1)
            return true
        }
        if commandOnly,
           let list = ListAction.allCases.first(where: { $0.key == event.charactersIgnoringModifiers?.lowercased() }) {
            model.toggle(list)
            return true
        }
        // Emacs-style ⌃N / ⌃P, as in other macOS lists and text fields.
        if event.modifierFlags.intersection([.command, .shift, .option, .control]) == .control {
            switch Int(event.keyCode) {
            case kVK_ANSI_N: model.move(1); return true
            case kVK_ANSI_P: model.move(-1); return true
            default: break
            }
        }
        switch Int(event.keyCode) {
        case kVK_Tab where model.showingHome && model.detailHit == nil:
            model.switchTab(event.modifierFlags.contains(.shift) ? -1 : 1)
            return true
        case kVK_DownArrow: model.move(1); return true
        case kVK_UpArrow: model.move(-1); return true
        case kVK_Return, kVK_ANSI_KeypadEnter:
            return model.handleReturn(command: event.modifierFlags.contains(.command))
        case kVK_Escape: model.handleEscape(); return true
        default: return false
        }
    }

    private func toggle() {
        panel.isVisible && panel.isKeyWindow ? hide() : show()
    }

    @objc func show() {
        let screen = NSScreen.screens.first { NSMouseInRect(NSEvent.mouseLocation, $0.frame, false) }
            ?? NSScreen.main
        if let frame = screen?.visibleFrame {
            let size = panel.frame.size
            panel.setFrameOrigin(NSPoint(x: frame.midX - size.width / 2,
                                         y: frame.minY + frame.height * 0.62 - size.height / 2))
        }
        ThemeStore.shared.reload()
        applyTheme()
        // Take keys without activating the app, like Raycast or Spotlight,
        // so the app you were in stays frontmost.
        if NSApp.isHidden { NSApp.unhideWithoutActivation() }
        panel.makeKeyAndOrderFront(nil)
        model.panelOpened()
    }

    func hide() {
        panel.orderOut(nil)
        // If something did activate the app (launching it from Finder or
        // Spotlight), give focus back to the previous app.
        if NSApp.isActive { NSApp.hide(nil) }
    }

    @objc func openSite() {
        NSWorkspace.shared.open(model.api.base)
    }

    // Close when focus moves to another app or window, like Spotlight.
    func windowDidResignKey(_ notification: Notification) {
        hide()
    }
}

@main
struct Main {
    static func main() {
        let app = NSApplication.shared
        let delegate = MainActor.assumeIsolated { AppDelegate() }
        app.delegate = delegate
        app.setActivationPolicy(.accessory)
        app.run()
    }
}
