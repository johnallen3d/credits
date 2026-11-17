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
final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    let model = LauncherModel()
    private var panel: LauncherPanel!
    private var statusItem: NSStatusItem!
    private var hotKey: HotKey?
    private var keyMonitor: Any?
    private var loginItem: NSMenuItem!

    func applicationDidFinishLaunching(_ notification: Notification) {
        panel = LauncherPanel()
        panel.delegate = self
        let host = NSHostingView(rootView: LauncherView(model: model))
        host.wantsLayer = true
        host.layer?.cornerRadius = 14
        host.layer?.masksToBounds = true
        panel.contentView = host
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
        loginItem = menu.addItem(withTitle: "Open at Login", action: #selector(toggleLoginItem), keyEquivalent: "")
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

        if !launchedAsLoginItem { show() }
    }

    /// Turns on Open at Login the first time the app runs from an
    /// Applications folder (not from a build directory). After that it's
    /// left to the menu's toggle.
    private func enableLoginItemOnFirstInstall() {
        let key = "loginItemConfigured"
        guard !UserDefaults.standard.bool(forKey: key),
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

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        show()
        return false
    }

    private func handleKey(_ event: NSEvent) -> Bool {
        switch Int(event.keyCode) {
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
        NSApp.activate()
        panel.makeKeyAndOrderFront(nil)
        model.panelOpened()
    }

    func hide() {
        panel.orderOut(nil)
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
