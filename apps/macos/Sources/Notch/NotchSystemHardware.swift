import AppKit
import AudioToolbox
import CoreAudio
import IOKit.graphics
import ObjectiveC

struct NotchHardwareReading: Sendable {
    var volume: Float?
    var muted: Bool
    var volumeWritable: Bool
    var brightness: Float?
    var brightnessWritable: Bool
    var output: String
    var device: AudioDeviceID
}

/// Confined to the monitor's worker queue. No audio samples are captured.
final class NotchSystemHardware: @unchecked Sendable {
    private var lastVolume: Float = 0.5
    private let display: NotchDisplayBrightness = .init()

    func read() -> NotchHardwareReading {
        let device = outputDevice()
        let channels = volumeAddresses(device)
        let volumes = channels.compactMap { float(device, $0) }
        let volume = volumes.isEmpty ? nil : volumes.reduce(0, +) / Float(volumes.count)
        if let volume, volume > 0 { lastVolume = volume }
        var muteAddress = address(kAudioDevicePropertyMute)
        var muted: UInt32 = 0; var size = UInt32(MemoryLayout<UInt32>.size)
        _ = AudioObjectGetPropertyData(device, &muteAddress, 0, nil, &size, &muted)
        let writable = !channels.isEmpty && channels.allSatisfy { isWritable(device, $0) }
        return .init(volume: volume, muted: muted != 0, volumeWritable: writable,
                     brightness: display.read(), brightnessWritable: display.canWrite,
                     output: name(device), device: device)
    }

    func press(_ action: NotchMediaKey.Action, fine: Bool) -> NotchHardwareReading {
        let reading = read(); let step: Float = fine ? 1 / 64 : 1 / 16
        switch action {
        case .volumeUp, .volumeDown:
            if let volume = reading.volume {
                let next = min(max((reading.muted ? 0 : volume) + (action == .volumeUp ? step : -step), 0), 1)
                setVolume(next, device: reading.device)
                setMute(next == 0, device: reading.device)
            }
        case .mute:
            var muteAddress = address(kAudioDevicePropertyMute)
            if isWritable(reading.device, muteAddress) {
                var muted: UInt32 = reading.muted ? 0 : 1
                _ = AudioObjectSetPropertyData(reading.device, &muteAddress, 0, nil, UInt32(MemoryLayout<UInt32>.size), &muted)
            } else { setVolume((reading.volume ?? 0) == 0 ? lastVolume : 0, device: reading.device) }
        case .brightnessUp, .brightnessDown:
            if let brightness = reading.brightness { display.write(min(max(brightness + (action == .brightnessUp ? step : -step), 0), 1)) }
        }
        return read()
    }

    private func outputDevice() -> AudioDeviceID {
        var property = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyDefaultOutputDevice, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
        var device = AudioDeviceID(0); var size = UInt32(MemoryLayout<AudioDeviceID>.size)
        _ = AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &property, 0, nil, &size, &device)
        return device
    }
    private func address(_ selector: AudioObjectPropertySelector, channel: UInt32 = 0) -> AudioObjectPropertyAddress {
        .init(mSelector: selector, mScope: kAudioDevicePropertyScopeOutput, mElement: channel)
    }
    private func volumeAddresses(_ device: AudioDeviceID) -> [AudioObjectPropertyAddress] {
        // The virtual main control preserves the system's existing stereo balance.
        for selector in [kAudioHardwareServiceDeviceProperty_VirtualMainVolume, kAudioDevicePropertyVolumeScalar] {
            let property = address(selector)
            if float(device, property) != nil { return [property] }
        }
        return [UInt32(1), 2].map { address(kAudioDevicePropertyVolumeScalar, channel: $0) }.filter { float(device, $0) != nil }
    }
    private func float(_ device: AudioDeviceID, _ property: AudioObjectPropertyAddress) -> Float? {
        var property = property; var value: Float = 0; var size = UInt32(MemoryLayout<Float>.size)
        guard AudioObjectGetPropertyData(device, &property, 0, nil, &size, &value) == noErr, value.isFinite else { return nil }
        return min(max(value, 0), 1)
    }
    private func isWritable(_ device: AudioDeviceID, _ property: AudioObjectPropertyAddress) -> Bool {
        var property = property; var writable = DarwinBoolean(false)
        return AudioObjectIsPropertySettable(device, &property, &writable) == noErr && writable.boolValue
    }
    private func setVolume(_ level: Float, device: AudioDeviceID) {
        let properties = volumeAddresses(device)
        let existing = properties.compactMap { float(device, $0) }
        let main = existing.isEmpty ? 0 : existing.reduce(0, +) / Float(existing.count)
        for var property in properties {
            // Channel fallback retains relative balance whenever the previous output was audible.
            var value = properties.count > 1 && main > 0 ? min(level * (float(device, property) ?? main) / main, 1) : level
            _ = AudioObjectSetPropertyData(device, &property, 0, nil, UInt32(MemoryLayout<Float>.size), &value)
        }
    }
    private func setMute(_ muted: Bool, device: AudioDeviceID) {
        var property = address(kAudioDevicePropertyMute); var value: UInt32 = muted ? 1 : 0
        if isWritable(device, property) { _ = AudioObjectSetPropertyData(device, &property, 0, nil, UInt32(MemoryLayout<UInt32>.size), &value) }
    }
    private func name(_ device: AudioDeviceID) -> String {
        var property = AudioObjectPropertyAddress(mSelector: kAudioObjectPropertyName, mScope: kAudioObjectPropertyScopeGlobal, mElement: 0)
        var name: Unmanaged<CFString>?; var size = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
        guard AudioObjectGetPropertyData(device, &property, 0, nil, &size, &name) == noErr,
              let name else { return "Audio" }
        return name.takeRetainedValue() as String
    }
}

/// Apple doesn't publish a modern built-in brightness setter. Resolve supported symbols at runtime;
/// an unavailable display keeps its native hardware-key behaviour.
private final class NotchDisplayBrightness {
    typealias Get = @convention(c) (UInt32, UnsafeMutablePointer<Float>) -> Int32
    typealias Set = @convention(c) (UInt32, Float) -> Int32
    private let handle = dlopen("/System/Library/PrivateFrameworks/DisplayServices.framework/DisplayServices", RTLD_LAZY | RTLD_LOCAL)
    private var getter: Get?
    private var setter: Set?
    private var proxy: NSObject?
    private let getSelector = NSSelectorFromString("brightnessForDisplay:")
    private let setSelector = NSSelectorFromString("setBrightness:forDisplay:")
    private typealias ProxyGet = @convention(c) (NSObject, Selector, UInt64) -> Float
    private typealias ProxySet = @convention(c) (NSObject, Selector, Float, UInt64) -> ObjCBool

    init() {
        if let handle {
            if let symbol = dlsym(handle, "DisplayServicesGetBrightness") { getter = unsafeBitCast(symbol, to: Get.self) }
            if let symbol = dlsym(handle, "DisplayServicesSetBrightness") { setter = unsafeBitCast(symbol, to: Set.self) }
        }
        _ = Bundle(path: "/System/Library/PrivateFrameworks/CoreBrightness.framework")?.load()
        for name in ["CBBrightnessProxy", "CBDisplayBrightnessClient"] {
            if let type = NSClassFromString(name) as? NSObject.Type {
                let candidate = type.init()
                if candidate.responds(to: getSelector) && candidate.responds(to: setSelector) { proxy = candidate; break }
            }
        }
    }
    deinit { if let handle { dlclose(handle) } }
    private var displayID: CGDirectDisplayID? {
        var displays = [CGDirectDisplayID](repeating: 0, count: 16); var count: UInt32 = 0
        guard CGGetActiveDisplayList(16, &displays, &count) == .success else { return nil }
        return displays.prefix(Int(count)).first { CGDisplayIsBuiltin($0) != 0 }
    }
    var canWrite: Bool { displayID != nil && (proxy != nil || setter != nil) }
    func read() -> Float? {
        guard let displayID else { return nil }
        if let proxy, let method = class_getInstanceMethod(type(of: proxy), getSelector) {
            let get = unsafeBitCast(method_getImplementation(method), to: ProxyGet.self)
            let value = get(proxy, getSelector, 0)
            if value.isFinite && value >= 0 && value <= 1 { return value }
        }
        var value: Float = 0
        guard getter?(displayID, &value) == 0, value.isFinite, value >= 0, value <= 1 else { return nil }
        return value
    }
    func write(_ value: Float) {
        guard let displayID else { return }
        if let proxy, let method = class_getInstanceMethod(type(of: proxy), setSelector) {
            let set = unsafeBitCast(method_getImplementation(method), to: ProxySet.self)
            if set(proxy, setSelector, value, 0).boolValue { return }
        }
        _ = setter?(displayID, value)
    }
}
