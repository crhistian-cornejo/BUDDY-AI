import AppKit

/// Links that come from Claude's output or from API data. Only absolute http(s) URLs with a
/// host are opened, and the UI shows that host before the user clicks.
enum ExternalLink {
    /// The URL if it is an absolute http or https URL with a host, otherwise nil.
    static func validated(_ string: String?) -> URL? {
        guard let raw = string?.trimmingCharacters(in: .whitespacesAndNewlines), !raw.isEmpty,
              let url = URL(string: raw),
              let scheme = url.scheme?.lowercased(), scheme == "https" || scheme == "http",
              let host = url.host(), !host.isEmpty else { return nil }
        return url
    }

    /// Host shown to the user next to an "Open" action (without a leading "www.").
    static func displayHost(_ url: URL) -> String {
        let host = url.host() ?? ""
        return host.hasPrefix("www.") ? String(host.dropFirst(4)) : host
    }

    @MainActor
    static func open(_ url: URL) {
        NSWorkspace.shared.open(url)
    }
}
