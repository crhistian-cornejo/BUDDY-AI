import Foundation

/// A brief, real system change, rendered beside the hardware notch without opening the tools.
struct NotchStatus: Equatable, Sendable {
    enum Kind: String, Sendable { case volume, brightness, connection, focus }
    var id = UUID()
    var kind: Kind
    var title: String
    var symbol: String
    var level: Double? = nil
    var text: String = ""
    var active: Bool? = nil
    var battery: Double? = nil
    var detail: String = ""

    var help: String {
        [title, level.map { "\(Int(($0 * 100).rounded())) %" } ?? text,
         battery.map { "Batería \(Int(($0 * 100).rounded())) %" } ?? "", detail].filter { !$0.isEmpty }.joined(separator: " · ")
    }
}

/// Only these hardware keys may be consumed. Modified shortcuts and unrelated keys keep their normal behaviour.
enum NotchMediaKey {
    enum Action: Equatable, Sendable { case volumeUp, volumeDown, mute, brightnessUp, brightnessDown }
    struct Press: Equatable, Sendable { var action: Action; var down: Bool; var fine: Bool }
    static func decode(data: Int, option: Bool, shift: Bool, command: Bool, control: Bool) -> Press? {
        guard !command, !control, !option || shift else { return nil }
        let action: Action
        switch (data >> 16) & 0xffff {
        case 0: action = .volumeUp
        case 1: action = .volumeDown
        case 7: action = .mute
        case 2: action = .brightnessUp
        case 3: action = .brightnessDown
        default: return nil
        }
        let state = (data >> 8) & 0xff
        guard state == 0x0a || state == 0x0b else { return nil }
        return .init(action: action, down: state == 0x0a, fine: option && shift)
    }
}
