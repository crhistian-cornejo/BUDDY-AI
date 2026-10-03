import IOKit.ps

struct NotchBattery: Equatable {
    let percent: Int?
    let charging: Bool
    let pluggedIn: Bool

    static func read() -> NotchBattery? {
        guard let info = IOPSCopyPowerSourcesInfo()?.takeRetainedValue(),
              let sources = IOPSCopyPowerSourcesList(info)?.takeRetainedValue() as? [CFTypeRef] else { return nil }
        for source in sources {
            guard let values = IOPSGetPowerSourceDescription(info, source)?.takeUnretainedValue() as? [String: Any],
                  values[kIOPSTypeKey] as? String == kIOPSInternalBatteryType else { continue }
            let current = values[kIOPSCurrentCapacityKey] as? Int
            let maximum = values[kIOPSMaxCapacityKey] as? Int
            let percent = current.flatMap { value in maximum.flatMap { $0 > 0 ? min(max(value * 100 / $0, 0), 100) : nil } }
            return .init(percent: percent, charging: values[kIOPSIsChargingKey] as? Bool ?? false,
                         pluggedIn: values[kIOPSPowerSourceStateKey] as? String == kIOPSACPowerValue)
        }
        return nil
    }
}
