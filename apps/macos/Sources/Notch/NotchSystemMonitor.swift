import AppKit
import ApplicationServices
import CoreAudio
import Intents
import Observation
@preconcurrency import IOBluetooth

struct NotchFocusReading: Equatable, Sendable {
    var active: Bool
    var title: String

    static func fromAssertions(_ data: Data) -> Self? {
        guard let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let entries = root["data"] as? [[String: Any]],
              let records = entries.first?["storeAssertionRecords"] as? [[String: Any]] else { return nil }
        func identifier(_ object: Any) -> String? {
            if let dictionary = object as? [String: Any] {
                if let value = dictionary["modeIdentifier"] as? String { return value }
                for value in dictionary.values { if let found = identifier(value) { return found } }
            } else if let array = object as? [Any] {
                for value in array { if let found = identifier(value) { return found } }
            }
            return nil
        }
        let mode = identifier(records) ?? ""
        return .init(active: !records.isEmpty, title: mode.contains("donotdisturb.mode.default") || mode.isEmpty ? "No molestar" : "Concentración")
    }
}

/// The event tap examines only system-defined media events, never ordinary typing.
@MainActor @Observable
final class NotchSystemMonitor {
    private let core: BuddyCore
    var levelsEnabled = true
    var connectionsEnabled = true
    var focusEnabled = true
    private(set) var replacementReady = false
    private(set) var accessibilityGranted = false
    private(set) var focusAvailable = false
    private(set) var brightnessAvailable = false
    @ObservationIgnored var onStatus: ((NotchStatus) -> Void)?
    @ObservationIgnored private let hardware = NotchSystemHardware()
    @ObservationIgnored private let worker = DispatchQueue(label: "buddy.notch.hardware", qos: .userInteractive)
    @ObservationIgnored private var timer: Timer?
    @ObservationIgnored private var tap: CFMachPort?
    @ObservationIgnored private var tapSource: CFRunLoopSource?
    @ObservationIgnored private var bluetooth: NotchBluetoothObserver?
    @ObservationIgnored private var reading: NotchHardwareReading?
    @ObservationIgnored private var focusReading: NotchFocusReading?
    @ObservationIgnored private var checking = false
    @ObservationIgnored private var ticks = 0
    @ObservationIgnored private var observers: [NSObjectProtocol] = []
    @ObservationIgnored private var audioObservers: [(AudioObjectID, AudioObjectPropertyAddress, AudioObjectPropertyListenerBlock)] = []

    init(core: BuddyCore) { self.core = core }

    func start() {
        reload()
        modelessAudioObserver()
        for name in [NSApplication.didBecomeActiveNotification, NSWorkspace.didWakeNotification] {
            let center = name == NSWorkspace.didWakeNotification ? NSWorkspace.shared.notificationCenter : NotificationCenter.default
            observers.append(center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                Task { @MainActor in self?.ensureTap(); self?.check() }
            })
        }
        // Brightness has no public change callback on every supported Mac. Lightweight readings are serialized;
        // volume itself uses CoreAudio listeners. Focus is read every two seconds, without reading notifications.
        timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
            Task { @MainActor in
                guard let self else { return }
                self.ticks += 1
                if self.levelsEnabled || self.connectionsEnabled { self.check() }
                if self.ticks % 4 == 0 { self.ensureTap(); self.checkFocus() }
            }
        }
        check(); checkFocus()
    }

    func stop() {
        timer?.invalidate(); timer = nil
        if let tap { CGEvent.tapEnable(tap: tap, enable: false); CFMachPortInvalidate(tap) }
        if let tapSource { CFRunLoopRemoveSource(CFRunLoopGetMain(), tapSource, .commonModes) }
        tap = nil; tapSource = nil; replacementReady = false
        bluetooth?.stop(); bluetooth = nil
        for (object, var address, block) in audioObservers { AudioObjectRemovePropertyListenerBlock(object, &address, .main, block) }
        audioObservers.removeAll()
        for observer in observers {
            NotificationCenter.default.removeObserver(observer)
            NSWorkspace.shared.notificationCenter.removeObserver(observer)
        }
        observers.removeAll()
    }

    func set(_ feature: String, enabled: Bool) {
        do { try core.setSetting(key: "notch.system.\(feature)", value: enabled ? "true" : "false"); reload() }
        catch { NSLog("No se pudo guardar la opción del notch: %@", String(describing: error)) }
    }

    func reload() {
        levelsEnabled = (try? core.setting(key: "notch.system.levels")) != "false"
        connectionsEnabled = (try? core.setting(key: "notch.system.connections")) != "false"
        focusEnabled = (try? core.setting(key: "notch.system.focus")) != "false"
        if connectionsEnabled && bluetooth == nil {
            let observer = NotchBluetoothObserver { [weak self] name, connected, symbol in
                self?.onStatus?(.init(kind: .connection, title: name, symbol: symbol,
                                     text: connected ? "Conectado" : "Desconectado", active: connected))
            }
            bluetooth = observer; observer.start()
        } else if !connectionsEnabled { bluetooth?.stop(); bluetooth = nil }
        ensureTap()
    }

    func requestAccessibility() {
        let options = ["AXTrustedCheckOptionPrompt": true] as CFDictionary
        _ = AXIsProcessTrustedWithOptions(options)
        if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility") { NSWorkspace.shared.open(url) }
    }

    func requestFocus() {
        INFocusStatusCenter.default.requestAuthorization { [weak self] _ in
            Task { @MainActor in self?.checkFocus() }
        }
    }

    private func ensureTap() {
        accessibilityGranted = AXIsProcessTrusted()
        guard levelsEnabled, accessibilityGranted else {
            if let tap { CGEvent.tapEnable(tap: tap, enable: false) }
            replacementReady = false; return
        }
        if let tap { CGEvent.tapEnable(tap: tap, enable: true); replacementReady = true; return }
        let callback: CGEventTapCallBack = { _, type, event, context in
            guard let context else { return Unmanaged.passUnretained(event) }
            let consumed = MainActor.assumeIsolated {
                Unmanaged<NotchSystemMonitor>.fromOpaque(context).takeUnretainedValue().key(type, event) == nil
            }
            return consumed ? nil : Unmanaged.passUnretained(event)
        }
        // Prefer hardware delivery, before macOS handles the key; session delivery is a compatibility fallback.
        let context = UnsafeMutableRawPointer(Unmanaged.passUnretained(self).toOpaque())
        let created = [CGEventTapLocation.cghidEventTap, .cgSessionEventTap].lazy.compactMap {
            CGEvent.tapCreate(tap: $0, place: .headInsertEventTap, options: .defaultTap,
                              eventsOfInterest: CGEventMask(1) << 14, callback: callback, userInfo: context)
        }.first
        guard let created else { return }
        tap = created
        tapSource = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, created, 0)
        if let tapSource { CFRunLoopAddSource(CFRunLoopGetMain(), tapSource, .commonModes) }
        CGEvent.tapEnable(tap: created, enable: true); replacementReady = true
    }

    private func key(_ type: CGEventType, _ event: CGEvent) -> Unmanaged<CGEvent>? {
        if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
            if levelsEnabled, let tap { CGEvent.tapEnable(tap: tap, enable: true) }
            return Unmanaged.passUnretained(event)
        }
        guard levelsEnabled, let key = NSEvent(cgEvent: event), key.subtype.rawValue == 8,
              let press = NotchMediaKey.decode(data: key.data1, option: key.modifierFlags.contains(.option),
                shift: key.modifierFlags.contains(.shift), command: key.modifierFlags.contains(.command), control: key.modifierFlags.contains(.control)) else {
            return Unmanaged.passUnretained(event)
        }
        let isBrightness = press.action == .brightnessUp || press.action == .brightnessDown
        guard isBrightness ? reading?.brightness != nil && reading?.brightnessWritable == true : reading?.volumeWritable == true else {
            return Unmanaged.passUnretained(event)
        }
        if press.down {
            let hardware = hardware
            worker.async { [weak self] in
                let next = hardware.press(press.action, fine: press.fine)
                Task { @MainActor in self?.accept(next, forced: isBrightness ? .brightness : .volume) }
            }
        }
        return nil
    }

    private func check() {
        guard !checking else { return }
        checking = true; let hardware = hardware
        worker.async { [weak self] in
            let next = hardware.read()
            Task { @MainActor in self?.checking = false; self?.accept(next) }
        }
    }

    private func accept(_ next: NotchHardwareReading, forced: NotchStatus.Kind? = nil) {
        let before = reading; reading = next
        brightnessAvailable = next.brightness != nil && next.brightnessWritable
        if before?.device != next.device { attachAudioDevice(next.device) }
        if connectionsEnabled, let before, next.device != 0, before.device != next.device {
            onStatus?(.init(kind: .connection, title: "Salida de audio", symbol: "hifispeaker.fill", text: "Conectada", active: true, detail: next.output))
        }
        guard levelsEnabled else { return }
        let volumeChanged = before != nil && before?.device == next.device &&
            (abs((before?.volume ?? 0) - (next.volume ?? 0)) > 0.002 || before?.muted != next.muted)
        if forced == .volume || volumeChanged, let level = next.volume {
            onStatus?(.init(kind: .volume, title: "Volumen", symbol: next.muted || level == 0 ? "speaker.slash.fill" : "speaker.wave.2.fill",
                            level: next.muted ? 0 : Double(level), detail: next.output))
        }
        let brightnessChanged = before?.brightness != nil && next.brightness != nil && abs((before?.brightness ?? 0) - (next.brightness ?? 0)) > 0.002
        if forced == .brightness || brightnessChanged, let level = next.brightness {
            onStatus?(.init(kind: .brightness, title: "Pantalla", symbol: "sun.max.fill", level: Double(level)))
        }
    }

    private func listen(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector, scope: AudioObjectPropertyScope, channel: UInt32 = 0) {
        var address = AudioObjectPropertyAddress(mSelector: selector, mScope: scope, mElement: channel)
        let block: AudioObjectPropertyListenerBlock = { [weak self] _, _ in Task { @MainActor in self?.check() } }
        if AudioObjectAddPropertyListenerBlock(object, &address, .main, block) == noErr { audioObservers.append((object, address, block)) }
    }
    private func modelessAudioObserver() {
        listen(AudioObjectID(kAudioObjectSystemObject), kAudioHardwarePropertyDefaultOutputDevice, scope: kAudioObjectPropertyScopeGlobal)
    }
    private func attachAudioDevice(_ device: AudioDeviceID) {
        for (object, var address, block) in audioObservers where object != AudioObjectID(kAudioObjectSystemObject) {
            AudioObjectRemovePropertyListenerBlock(object, &address, .main, block)
        }
        audioObservers.removeAll { $0.0 != AudioObjectID(kAudioObjectSystemObject) }
        for channel: UInt32 in [0, 1, 2] { listen(device, kAudioDevicePropertyVolumeScalar, scope: kAudioDevicePropertyScopeOutput, channel: channel) }
        listen(device, kAudioDevicePropertyMute, scope: kAudioDevicePropertyScopeOutput)
    }

    private func checkFocus() {
        guard focusEnabled else { return }
        let center = INFocusStatusCenter.default
        var current: NotchFocusReading?
        if center.authorizationStatus == .authorized, let active = center.focusStatus.isFocused {
            current = .init(active: active, title: "Concentración")
        } else {
            // Optional compatibility source, only if the app already has access. Never request Full Disk Access.
            let url = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/DoNotDisturb/DB/Assertions.json")
            if let attributes = try? FileManager.default.attributesOfItem(atPath: url.path),
               let size = attributes[.size] as? NSNumber, size.intValue < 1_000_000,
               let data = try? Data(contentsOf: url) { current = NotchFocusReading.fromAssertions(data) }
        }
        focusAvailable = current != nil
        guard let current else { return }
        if let before = focusReading, before != current {
            onStatus?(.init(kind: .focus, title: current.title, symbol: "moon.fill", text: current.active ? "On" : "Off", active: current.active,
                            detail: current.active ? "Notificaciones silenciadas" : "Notificaciones activadas"))
        }
        focusReading = current
    }
}

@MainActor
private final class NotchBluetoothObserver: NSObject {
    private var connected: IOBluetoothUserNotification?
    private var disconnects: [String: IOBluetoothUserNotification] = [:]
    private var devices: Set<String> = []
    private let changed: (String, Bool, String) -> Void
    init(changed: @escaping (String, Bool, String) -> Void) { self.changed = changed; super.init() }
    func start() {
        if let paired = IOBluetoothDevice.pairedDevices() as? [IOBluetoothDevice] {
            for device in paired where device.isConnected() { devices.insert(device.addressString); watch(device) }
        }
        connected = IOBluetoothDevice.register(forConnectNotifications: self, selector: #selector(didConnect(_:device:)))
    }
    func stop() {
        connected?.unregister(); connected = nil
        for notification in disconnects.values { notification.unregister() }
        disconnects.removeAll(); devices.removeAll()
    }
    private func watch(_ device: IOBluetoothDevice) {
        guard disconnects[device.addressString] == nil else { return }
        disconnects[device.addressString] = device.register(forDisconnectNotification: self, selector: #selector(didDisconnect(_:device:)))
    }
    nonisolated private func symbol(_ device: IOBluetoothDevice) -> String {
        let name = device.nameOrAddress.lowercased()
        if name.contains("airpods pro") { return "airpodspro" }
        if name.contains("airpods") { return "airpods" }
        let major = (device.classOfDevice >> 8) & 0x1f
        if major == 4 { return "headphones" }
        if major == 5 { return "keyboard" }
        return "antenna.radiowaves.left.and.right"
    }
    @objc nonisolated private func didConnect(_ notification: IOBluetoothUserNotification, device: IOBluetoothDevice) {
        let address = device.addressString ?? ""
        let name = device.nameOrAddress ?? "Bluetooth"
        let icon = symbol(device)
        Task { @MainActor [weak self] in
            guard let self, self.connected != nil, !address.isEmpty, self.devices.insert(address).inserted else { return }
            if let device = IOBluetoothDevice(addressString: address) { self.watch(device) }
            self.changed(name, true, icon)
        }
    }
    @objc nonisolated private func didDisconnect(_ notification: IOBluetoothUserNotification, device: IOBluetoothDevice) {
        let address = device.addressString ?? ""
        let name = device.nameOrAddress ?? "Bluetooth"
        let icon = symbol(device)
        Task { @MainActor [weak self] in
            guard let self, self.devices.remove(address) != nil else { return }
            self.disconnects.removeValue(forKey: address)?.unregister()
            self.changed(name, false, icon)
        }
    }
}
