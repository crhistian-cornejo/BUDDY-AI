import Foundation

// The core's records, as the paired machine sends them (the same JSON the Windows app reads).

struct Hello: Decodable, Equatable {
    var name: String
    var platform: String
    var version: String
}

struct ChatSummary: Decodable, Identifiable, Equatable {
    var id: String
    var title: String
    var updatedAt: Int64
    var preview: String
}

struct SourceLink: Decodable, Equatable {
    var title: String
    var url: String
}

struct ChatMessage: Decodable, Identifiable, Equatable {
    var id: Int64
    var role: String
    var agent: String
    var provider: String?
    var text: String
    var sources: [SourceLink]
    var failed: Bool
    var createdAt: Int64
    var model: String?
    var took: String?
}

struct AgentInfo: Decodable, Identifiable, Equatable {
    var id: String
    var name: String
    var specialty: String
}

/// A character ready to paint: every frame is `size × size` pixels, row by row, as `0xAARRGGBB` (0 = transparent).
struct Sprite: Decodable, Equatable {
    var id: String
    var name: String
    var size: Int
    var states: [SpriteState]
}

struct SpriteState: Decodable, Equatable {
    var name: String
    var fps: Int
    var frames: [[UInt32]]
}

struct SessionInfo: Decodable, Identifiable, Equatable {
    var sessionId: String
    var agent: String
    var project: String
    var state: String
    var updatedAt: Int64
    var id: String { sessionId }
}

struct ProviderUsage: Decodable, Identifiable, Equatable {
    var provider: String
    var name: String
    var windows: [UsageWindow]
    var id: String { provider }
}

struct UsageWindow: Decodable, Equatable {
    var label: String
    var usedPct: Double
    var resetsAt: Int64?
}

struct BriefingItem: Decodable, Identifiable, Equatable {
    var topic: String
    var text: String
    var url: String?
    var at: Int64
    var id: String { "\(at)-\(text)" }
}

/// An agent asking for permission on the machine (from the core's `approvalRequest` event).
struct Approval: Identifiable, Equatable {
    var id: String
    var agent: String
    var project: String
    var title: String
    var summary: String
    var detail: String
    var canAllow: Bool
}

enum AgentNames {
    /// How an agent or provider is called on screen.
    static func name(_ id: String) -> String {
        switch id {
        case "claude": "Claude Code"
        case "codex": "Codex"
        case "antigravity": "Gemini"
        case "buddy": "Buddy"
        default: id.capitalized
        }
    }
}
