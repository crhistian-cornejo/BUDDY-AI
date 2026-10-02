import Foundation
import Darwin

// Telegram — what MIKA keeps from the user's picked channels and how it reaches the agents. Twin of the file side of
// apps/windows/src-tauri/src/integrations/telegram.rs:
//  * the posts go into `<agent>/workspace/telegram/posts.jsonl`, and their photos into `telegram/media/`, of every agent
//    whose agent.md says `integration: telegram` (PARLEY), for 3 days, the media folder capped at 200 MB;
//  * what channels post is untrusted: the helper downloads Telegram photos only and checks each one; here every name it
//    returns is checked again (`<digits>_<digits>.jpg`, nothing else reaches a path), each photo gets the quarantine
//    attribute (Windows: the Mark of the Web), anything else in the media folder is deleted, and MIKA never opens,
//    previews or Quick Looks those files (ChatImagePolicy refuses any `telegram` folder): only the agent's model reads
//    them;
//  * the newest message id read per chat is in `MIKA/telegram/state.json`.
// The helper process (mika-telegram) is TelegramHelper.swift; the poller and the card data, TelegramPoller.swift.
// Everything in here is plain Foundation, so the unit tests compile it as is.

/// One post of a picked chat, as the helper sends it and as `posts.jsonl` keeps it.
struct TelegramPost: Codable, Equatable, Sendable {
    var chatId: Int64
    var chat: String
    var id: Int
    /// Unix seconds.
    var date: Int64
    var text: String = ""
    /// Who wrote it when that is not the chat itself (a member of a group, a channel's signature); empty otherwise.
    var sender: String = ""
    /// Every link in the post: the plain ones and the ones behind a word ("LINK").
    var links: [String] = []
    /// File names inside `telegram/media/`.
    var photos: [String] = []
    /// What else it carries (video, audio, documento, sticker, encuesta…): named by the helper, never downloaded.
    var media: String = ""
    /// Posts of the same album share it.
    var album: Int64?

    init(chatId: Int64, chat: String, id: Int, date: Int64, text: String = "", sender: String = "", links: [String] = [],
         photos: [String] = [], media: String = "", album: Int64? = nil) {
        self.chatId = chatId; self.chat = chat; self.id = id; self.date = date; self.text = text; self.sender = sender
        self.links = links; self.photos = photos; self.media = media; self.album = album
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        chatId = try c.decode(Int64.self, forKey: .chatId)
        chat = (try? c.decodeIfPresent(String.self, forKey: .chat)) ?? ""
        id = try c.decode(Int.self, forKey: .id)
        date = try c.decode(Int64.self, forKey: .date)
        text = (try? c.decodeIfPresent(String.self, forKey: .text)) ?? ""
        sender = String(((try? c.decodeIfPresent(String.self, forKey: .sender)) ?? "").prefix(80))
        links = (try? c.decodeIfPresent([String].self, forKey: .links)) ?? []
        photos = (try? c.decodeIfPresent([String].self, forKey: .photos)) ?? []
        media = String(((try? c.decodeIfPresent(String.self, forKey: .media)) ?? "").prefix(20))
        album = try? c.decodeIfPresent(Int64.self, forKey: .album)
    }

    /// "🎥 video", "📷 una captura"… what a post with no words shows instead; nil when it has none of it.
    var attachmentLabel: String? {
        TelegramPost.attachment(photos: photos.count, media: media)
    }

    static func attachment(photos: Int, media: String) -> String? {
        if photos > 0 { return "📷 \(photos == 1 ? "una captura" : "\(photos) capturas")" }
        let icons = ["video": "🎥", "audio": "🎧", "documento": "📄", "sticker": "🙂", "encuesta": "📊", "imagen": "🖼", "ubicación": "📍", "contacto": "👤"]
        let name = media.trimmingCharacters(in: .whitespaces)
        return name.isEmpty ? nil : "\(icons[name] ?? "📎") \(name)"
    }
}

/// A chat the user picked in Settings. `hash` is the access hash Telegram asks for to read it from this account; it is
/// a string, as on Windows, because it is a full 64-bit number.
struct TelegramChat: Codable, Equatable, Sendable, Identifiable {
    var id: Int64
    var hash: String
    var title: String

    /// Each chat once, only with a numeric hash, at most 50, titles of 120 characters at most. Applied on load and on
    /// every change (Settings::sanitize on Windows).
    static func sanitize(_ chats: [TelegramChat]) -> [TelegramChat] {
        var seen = Set<Int64>()
        var out: [TelegramChat] = []
        for var chat in chats where Int64(chat.hash) != nil && !seen.contains(chat.id) {
            seen.insert(chat.id)
            chat.title = String(chat.title.prefix(120))
            out.append(chat)
            if out.count == 50 { break }
        }
        return out
    }
}

/// A channel or group the user is in, as the helper lists it for the picker.
struct TelegramChatOption: Decodable, Equatable, Sendable, Identifiable {
    var id: Int64
    var hash: Int64
    var title: String
    /// "channel" or "group".
    var kind: String

    private enum CodingKeys: String, CodingKey { case id, hash, title, kind }

    init(id: Int64, hash: Int64, title: String, kind: String) {
        self.id = id; self.hash = hash; self.title = title; self.kind = kind
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(Int64.self, forKey: .id)
        hash = try c.decode(Int64.self, forKey: .hash)
        title = String(((try? c.decodeIfPresent(String.self, forKey: .title)) ?? "").prefix(120))
        kind = (try? c.decodeIfPresent(String.self, forKey: .kind)) ?? "channel"
    }

    /// What Settings keeps when the user ticks it.
    var picked: TelegramChat { TelegramChat(id: id, hash: String(hash), title: title) }
}

/// Where the login stands, for Settings.
struct TelegramStatus: Equatable, Sendable {
    /// api_id and api_hash are stored.
    var configured = false
    var authorized = false
    /// "idle", "code" (a code was sent) or "password" (the account has 2FA).
    var step = "idle"
    /// The 2FA password hint, when Telegram has one.
    var hint: String?
    /// Who is logged in, for "Conectado como …".
    var name: String?
    var error: String?
}

// MARK: - The helper's protocol (see the top of apps/windows/telegram/src/main.rs)

/// A chat in a `fetch` request.
struct TelegramChatRef: Codable, Equatable, Sendable {
    var id: Int64
    var hash: Int64
    var title: String
}

struct TelegramFetchChat: Encodable, Equatable, Sendable {
    var chat: TelegramChatRef
    /// The newest message id already read; 0 the first time (the helper then brings the last few posts).
    var after: Int
}

/// One request to the helper.
enum TelegramCommand: Sendable {
    case connect(apiID: Int64, apiHash: String, session: String?)
    case status
    case sendCode(phone: String)
    case signIn(code: String)
    case password(String)
    case signOut
    case chats(limit: Int)
    case fetch(chats: [TelegramFetchChat], limit: Int, mediaDir: String)
    case quit

    /// The request as one JSON line (without the newline). Numbers stay exact: a 64-bit access hash must not go
    /// through a Double.
    func line(id: UInt64) -> String? {
        var wire = Wire(id: id, cmd: "")
        switch self {
        case .connect(let apiID, let apiHash, let session):
            wire.cmd = "connect"; wire.apiId = apiID; wire.apiHash = apiHash; wire.session = session
        case .status:
            wire.cmd = "status"
        case .sendCode(let phone):
            wire.cmd = "sendCode"; wire.phone = phone
        case .signIn(let code):
            wire.cmd = "signIn"; wire.code = code
        case .password(let password):
            wire.cmd = "password"; wire.password = password
        case .signOut:
            wire.cmd = "signOut"
        case .chats(let limit):
            wire.cmd = "chats"; wire.limit = limit
        case .fetch(let chats, let limit, let mediaDir):
            wire.cmd = "fetch"; wire.chats = chats; wire.limit = limit; wire.mediaDir = mediaDir
        case .quit:
            wire.cmd = "quit"
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.withoutEscapingSlashes]
        guard let data = try? encoder.encode(wire) else { return nil }
        return String(decoding: data, as: UTF8.self)
    }

    private struct Wire: Encodable {
        var id: UInt64
        var cmd: String
        var apiId: Int64?
        var apiHash: String?
        var session: String?
        var phone: String?
        var code: String?
        var password: String?
        var limit: Int?
        var chats: [TelegramFetchChat]?
        var mediaDir: String?
    }
}

/// Any line the helper prints: an answer (`id`, `ok`, `result` or `error`) or a session event (`event`, `value`).
struct TelegramEnvelope: Decodable, Sendable {
    var id: UInt64?
    var ok: Bool?
    var error: String?
    var event: String?
    var value: String?
}

/// The `result` of an answer.
struct TelegramReply<T: Decodable>: Decodable {
    var result: T
}

/// `status`, `sendCode`, `signIn` and `password` answer with this.
struct TelegramHelperStatus: Decodable, Sendable {
    var authorized: Bool
    var step: String?
    var hint: String?
    var name: String?
}

/// What a `fetch` brings: the posts (oldest first), the newest id seen per chat, and the chats that could not be read.
struct TelegramFetchResult: Decodable, Sendable {
    struct Last: Decodable, Sendable { var chatId: Int64; var last: Int64 }
    struct Failure: Decodable, Sendable { var chatId: Int64?; var error: String? }

    var posts: [TelegramPost] = []
    var last: [Last] = []
    var errors: [Failure] = []

    private enum CodingKeys: String, CodingKey { case posts, last, errors }

    init(posts: [TelegramPost] = [], last: [Last] = [], errors: [Failure] = []) {
        self.posts = posts; self.last = last; self.errors = errors
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        posts = (try? c.decodeIfPresent([TelegramPost].self, forKey: .posts)) ?? []
        last = (try? c.decodeIfPresent([Last].self, forKey: .last)) ?? []
        errors = (try? c.decodeIfPresent([Failure].self, forKey: .errors)) ?? []
    }
}

// MARK: - Files

enum TelegramInbox {
    /// The integration name an agent puts in its agent.md to receive the posts.
    static let agentIntegration = "telegram"
    /// How long posts and their photos stay for the agents.
    static let keepSeconds: Int64 = 3 * 24 * 3600
    /// The media folder never grows past this: the oldest photos go first.
    static let maxMediaBytes: Int64 = 200 * 1024 * 1024

    static func dir(workspace: URL) -> URL { workspace.appendingPathComponent("telegram", isDirectory: true) }
    static func postsFile(_ dir: URL) -> URL { dir.appendingPathComponent("posts.jsonl") }
    static func mediaDir(_ dir: URL) -> URL { dir.appendingPathComponent("media", isDirectory: true) }

    /// The file of a photo the user asked to see, or nil: a safe name, really inside the media folder (no links out of it),
    /// a plain file of at most 8 MB that starts like a JPEG. Nothing else is ever opened.
    static func viewablePhoto(_ name: String, mediaDir: URL) -> URL? {
        guard safeMediaName(name) else { return nil }
        let base = mediaDir.standardizedFileURL.resolvingSymlinksInPath()
        let file = base.appendingPathComponent(name).standardizedFileURL
        guard file.resolvingSymlinksInPath().deletingLastPathComponent().path == base.path,
              let values = try? file.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey]),
              values.isRegularFile == true, let size = values.fileSize, size > 4, size <= 8_000_000,
              let handle = try? FileHandle(forReadingFrom: file) else { return nil }
        defer { try? handle.close() }
        guard let head = try? handle.read(upToCount: 3), Array(head) == [0xFF, 0xD8, 0xFF] else { return nil }
        return file
    }

    /// `MIKA/telegram/state.json`, next to the agents folder.
    static func stateURL(agentsRoot: URL) -> URL {
        agentsRoot.deletingLastPathComponent().appendingPathComponent("telegram", isDirectory: true)
            .appendingPathComponent("state.json")
    }

    /// The only file names the helper may hand back: `<chat>_<post>.jpg`, digits only (at most 20 each). Anything else
    /// could point outside the media folder, so it never gets anywhere near a path.
    static func safeMediaName(_ name: String) -> Bool {
        guard name.hasSuffix(".jpg") else { return false }
        let parts = name.dropLast(4).split(separator: "_", maxSplits: 1, omittingEmptySubsequences: false)
        guard parts.count == 2 else { return false }
        return parts.allSatisfy { part in
            !part.isEmpty && part.count <= 20 && part.unicodeScalars.allSatisfy { $0.value >= 48 && $0.value <= 57 }
        }
    }

    /// The posts with only the photo names MIKA accepts.
    static func checked(_ posts: [TelegramPost]) -> [TelegramPost] {
        posts.map { post in
            var post = post
            post.photos = post.photos.filter(safeMediaName)
            return post
        }
    }

    /// Appends the posts to every agent's `posts.jsonl` (the photos were downloaded into the first one and are copied
    /// over), marks each photo as downloaded from the internet, and drops what is too old, too much or not ours.
    /// What the island says about posts that just arrived: one line per chat, who wrote (the channel or group, as the
    /// helper names it) and what it said, cut to `limit` characters. A post with only a picture or a
    /// link says so. No model reads it: it is the post's own words.
    static func summaries(of posts: [TelegramPost], limit: Int = 110) -> [(chat: String, text: String)] {
        var order: [Int64] = []
        var byChat: [Int64: [TelegramPost]] = [:]
        for post in posts.sorted(by: { ($0.date, $0.id) < ($1.date, $1.id) }) {
            if byChat[post.chatId] == nil { order.append(post.chatId) }
            byChat[post.chatId, default: []].append(post)
        }
        return order.compactMap { chatId in
            guard let group = byChat[chatId], let latest = group.last(where: { !$0.text.isEmpty }) ?? group.last else { return nil }
            let words = latest.text.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
            var said = words
            if said.count > limit { said = String(said.prefix(limit - 1)) + "…" }
            let photos = group.reduce(0) { $0 + $1.photos.count }
            let other = group.last(where: { !$0.media.isEmpty })?.media ?? ""
            if said.isEmpty {
                said = photos > 0 ? "envió \(photos == 1 ? "una captura" : "\(photos) capturas")" : other.isEmpty ? "compartió un enlace" : "envió \(other)"
            }
            else if photos > 0 { said += " (+\(photos == 1 ? "una captura" : "\(photos) capturas"))" }
            let title = latest.chat.isEmpty ? "Telegram" : latest.chat
            let count = group.count > 1 ? "\(group.count) mensajes · " : ""
            let who = latest.sender.isEmpty ? "" : latest.sender + ": "
            return (title, count + who + said)
        }
    }

    static func store(_ posts: [TelegramPost], in dirs: [URL], now: Int64) {
        guard let first = dirs.first else { return }
        let fm = FileManager.default
        let posts = checked(posts)
        let lines = posts.compactMap(line).joined()
        for dir in dirs {
            try? fm.createDirectory(at: mediaDir(dir), withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            for name in posts.flatMap(\.photos) {
                let target = mediaDir(dir).appendingPathComponent(name)
                if dir != first {
                    try? fm.removeItem(at: target)
                    try? fm.copyItem(at: mediaDir(first).appendingPathComponent(name), to: target)
                }
                if fm.fileExists(atPath: target.path) { quarantine(target, now: now) }
            }
            JSONLines.append(lines, to: postsFile(dir))
            prune(dir, now: now)
        }
    }

    /// Posts and photos older than 3 days go, and so does any file in `media/` that is not one of ours (a half-done
    /// download included); then, while the photos weigh more than 200 MB, the oldest go.
    static func prune(_ dir: URL, now: Int64, maxBytes: Int64 = TelegramInbox.maxMediaBytes) {
        let cutoff = now - keepSeconds
        let file = postsFile(dir)
        let all = read(file)
        if all.contains(where: { $0.date < cutoff }) {
            let kept = all.filter { $0.date >= cutoff }.compactMap(line).joined()
            try? Data(kept.utf8).write(to: file, options: .atomic)
        }
        let fm = FileManager.default
        let keys: [URLResourceKey] = [.contentModificationDateKey, .fileSizeKey, .isRegularFileKey]
        guard let entries = try? fm.contentsOfDirectory(at: mediaDir(dir), includingPropertiesForKeys: keys) else { return }
        let nowDate = Date(timeIntervalSince1970: TimeInterval(now))
        var files: [(url: URL, modified: Date, size: Int64)] = []
        for entry in entries {
            guard let values = try? entry.resourceValues(forKeys: Set(keys)) else { continue }
            let modified = values.contentModificationDate ?? .distantPast
            if nowDate.timeIntervalSince(modified) > TimeInterval(keepSeconds) || !safeMediaName(entry.lastPathComponent) {
                try? fm.removeItem(at: entry)
            } else {
                files.append((url: entry, modified: modified, size: Int64(values.fileSize ?? 0)))
            }
        }
        var total = files.reduce(Int64(0)) { $0 + $1.size }
        for file in files.sorted(by: { $0.modified < $1.modified }) where total > maxBytes {
            try? fm.removeItem(at: file.url)
            total -= file.size
        }
    }

    /// `com.apple.quarantine`, as a browser download gets: the system treats the file as coming from the internet if
    /// anyone ever opens it outside MIKA. The twin of Windows' Mark of the Web (Zone.Identifier, ZoneId=3).
    static let quarantineAttribute = "com.apple.quarantine"

    static func quarantineValue(now: Int64) -> String { "0083;\(String(now, radix: 16));MIKA;" }

    static func quarantine(_ file: URL, now: Int64) {
        let value = quarantineValue(now: now)
        _ = file.withUnsafeFileSystemRepresentation { path -> Int32 in
            guard let path else { return -1 }
            return value.withCString { setxattr(path, quarantineAttribute, $0, strlen($0), 0, XATTR_NOFOLLOW) }
        }
    }

    static func hasQuarantine(_ file: URL) -> Bool {
        file.withUnsafeFileSystemRepresentation { path -> Bool in
            guard let path else { return false }
            return getxattr(path, quarantineAttribute, nil, 0, 0, XATTR_NOFOLLOW) >= 0
        }
    }

    static func read(_ file: URL) -> [TelegramPost] {
        guard let text = try? String(contentsOf: file, encoding: .utf8) else { return [] }
        let decoder = JSONDecoder()
        return text.split(separator: "\n").compactMap { try? decoder.decode(TelegramPost.self, from: Data($0.utf8)) }
    }

    /// The posts newer than `since` (Unix seconds), oldest first.
    static func postsSince(_ dir: URL, since: Int64) -> [TelegramPost] {
        read(postsFile(dir)).filter { $0.date > since }.sorted { ($0.date, $0.id) < ($1.date, $1.id) }
    }

    private static func line(_ post: TelegramPost) -> String? {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.withoutEscapingSlashes]
        guard let data = try? encoder.encode(post) else { return nil }
        return String(decoding: data, as: UTF8.self) + "\n"
    }

    // MARK: state.json

    /// The newest message id read per chat.
    static func loadState(_ url: URL) -> [Int64: Int] {
        guard let data = try? Data(contentsOf: url),
              let map = try? JSONDecoder().decode([String: Int].self, from: data) else { return [:] }
        var out: [Int64: Int] = [:]
        for (key, value) in map { if let id = Int64(key) { out[id] = value } }
        return out
    }

    static func saveState(_ state: [Int64: Int], to url: URL) {
        let map = Dictionary(uniqueKeysWithValues: state.map { (String($0.key), $0.value) })
        guard let data = try? JSONEncoder().encode(map) else { return }
        try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true,
                                                 attributes: [.posixPermissions: 0o700])
        try? data.write(to: url, options: .atomic)
    }

    /// The state after a fetch: a chat's newest id only moves forward.
    static func advance(_ state: [Int64: Int], with last: [TelegramFetchResult.Last]) -> [Int64: Int] {
        var out = state
        for entry in last {
            let id = Int(clamping: entry.last)
            if id > (out[entry.chatId] ?? 0) { out[entry.chatId] = id }
        }
        return out
    }
}

// MARK: - The card

/// What the Telegram card shows: PARLEY's latest picks, the latest posts, and what went wrong.
struct TelegramCard: Equatable, Sendable {
    struct Post: Equatable, Sendable, Identifiable {
        var id: String
        var chat: String
        var text: String
        /// Unix seconds.
        var date: Int64
        var photos: Int
        /// The member who wrote it, when the chat is a group.
        var sender = ""
        var media = ""
        /// Every link the post has, and the names of its photos (`telegram/media/`), for the tap-to-open parts of the feed.
        var links: [String] = []
        var photoFiles: [String] = []
        /// The words of the post with the links taken out and clipped: the one or two lines the feed shows closed.
        var headline: String { TelegramCard.headline(text) }
    }

    var posts: [Post] = []
    var mediaDir: URL?
    var scan = PicksScan()
    /// One chat could not be read (left, banned): the others still are.
    var warning: String?
    /// The last poll failed (not logged in, no channels, the helper is missing…). The data on disk stays shown.
    var error: String?

    var hasData: Bool { !scan.picks.isEmpty || !posts.isEmpty || scan.at != nil }
    var unit: String { scan.unit == "u" ? "u" : "S/" }

    /// The latest 4 posts of the last 24 h (newest first) and the latest review.
    static func load(inbox: URL, picks: URL, now: Int64) -> TelegramCard {
        var card = TelegramCard()
        card.mediaDir = TelegramInbox.mediaDir(inbox)
        card.posts = TelegramInbox.postsSince(inbox, since: now - 24 * 3600).reversed().prefix(12).map {
            Post(id: "\($0.chatId)/\($0.id)", chat: $0.chat, text: String($0.text.prefix(1200)), date: $0.date, photos: $0.photos.count,
                 sender: $0.sender, media: $0.media, links: Array($0.links.prefix(6)), photoFiles: Array($0.photos.prefix(4)))
        }
        card.scan = PicksLedger.lastScan(picks)
        return card
    }

    /// What a post says, short: no links, no empty lines, the first sentence or two up to `limit` characters, cut at a word.
    static func headline(_ text: String, limit: Int = 110) -> String {
        let urlPattern = try! NSRegularExpression(pattern: #"(?:https?://|www\.)\S+"#, options: .caseInsensitive)
        let range = NSRange(text.startIndex..., in: text)
        let bare = urlPattern.stringByReplacingMatches(in: text, range: range, withTemplate: "")
        let lines = bare.components(separatedBy: .newlines)
            .map { $0.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ") }
            .filter { $0.contains(where: { $0.isLetter || $0.isNumber }) }
        var out = ""
        for line in lines {
            if !out.isEmpty && out.count + line.count + 3 > limit { break }
            out += (out.isEmpty ? "" : " · ") + line
            if out.count >= limit * 2 / 3 { break }
        }
        guard out.count > limit else { return out }
        let cut = out.prefix(limit - 1)
        let atWord = cut.lastIndex(of: " ").map { String(cut[..<$0]) } ?? String(cut)
        return (atWord.count >= limit / 2 ? atWord : String(cut)) + "…"
    }

    /// "ejemplo.com" for a link a post carries, or nil when it is not an http(s) address.
    static func host(of link: String) -> String? {
        guard let url = URL(string: link), ["http", "https"].contains(url.scheme?.lowercased() ?? ""), let host = url.host, !host.isEmpty else { return nil }
        return host.lowercased().hasPrefix("www.") ? String(host.dropFirst(4)) : host
    }

    /// "S/ 15", "3 u", or "sin monto (tope)" when the daily cap left nothing.
    static func stake(_ amount: Double, unit: String) -> String {
        guard amount.isFinite, amount > 0 else { return "sin monto (tope)" }
        return unit == "u" ? "\(Picks.fmt(amount)) u" : "S/ \(Picks.fmt(amount))"
    }

    static func odds(_ value: Double) -> String { value.isFinite ? "@" + String(format: "%.2f", value) : "" }

    /// The only links a button may open: https on betano.pe or its subdomains (checked again here, whatever the file
    /// on disk says).
    static func safeLink(_ value: String?) -> URL? {
        guard let value = value?.trimmingCharacters(in: .whitespacesAndNewlines), Picks.allowedLink(value),
              let url = URL(string: value), url.scheme?.lowercased() == "https" else { return nil }
        return url
    }

    /// "just now", "5m", "2h", "3d" (as the other cards).
    static func timeAgo(_ seconds: Int64, now: Int64) -> String {
        let diff = now - seconds
        if diff < 60 { return "just now" }
        if diff < 3600 { return "\(diff / 60)m" }
        if diff < 86_400 { return "\(diff / 3600)h" }
        return "\(diff / 86_400)d"
    }
}
