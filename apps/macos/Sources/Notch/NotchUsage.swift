import Foundation

/// One window per provider: Codex's weekly allowance, the others' five-hour allowance, then available data.
enum NotchUsage {
    static func window(for plan: ProviderUsage) -> UsageWindow? {
        let fiveHour = plan.windows.first { $0.label.lowercased() == "5 h" }
        let weekly = plan.windows.first { $0.label.lowercased() == "semana" }
        return (plan.provider == "codex" ? weekly ?? fiveHour : fiveHour ?? weekly) ?? plan.windows.first
    }

    static func help(plan: ProviderUsage, window: UsageWindow) -> String {
        var text = "\(plan.name) · \(window.label) · \(Int(window.usedPct.rounded())) % usado"
        if let resets = window.resetsAt {
            let date = Date(timeIntervalSince1970: TimeInterval(resets))
            let formatter = DateFormatter()
            formatter.locale = Locale(identifier: "es")
            formatter.setLocalizedDateFormatFromTemplate(Calendar.current.isDateInToday(date) ? "HH:mm" : "EEE d HH:mm")
            text += " · se reinicia \(formatter.string(from: date))"
        }
        return text
    }
}
