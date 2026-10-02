import Foundation

/// PARLEY's reviews: the scheduler, the review itself and the note its chat gets with every question. Twin of the
/// scheduling half of apps/windows/src-tauri/src/services/picks.rs; the texts and the checks on its answer are in
/// Picks.swift.
///
/// Once the user switches the automatic reviews on (Settings › PARLEY), a review runs 2.5 min after launch, then every
/// interval, and sooner (never within 20 min of the last one) when a picked channel posts something that looks like a
/// pick. "Revisar ahora" and the card's "Revisar" run one on demand. A review is skipped while the user is talking to
/// PARLEY. MIKA never bets: the card only opens Betano, and only on a click.
@MainActor
final class PicksService {
    static let shared = PicksService()

    private var ticker: Task<Void, Never>?
    private var lastRun: Date?
    /// The last morning snapshot attempt, so a failure waits `Odds.morningRetry` before the next.
    private var morningTried: Date?
    /// Free form stats: the day already finished, and the last attempt (a failure waits `FreeStats.retry`).
    private var statsDone = ""
    private var statsTried: Date?
    /// A channel posted something that looks like a pick: the next review comes sooner.
    private var pending = false
    private(set) var scanning = false
    /// The daily digest is being made, and the last attempt (a failure waits `ParleyDigest.retry`).
    private(set) var digesting = false
    private var digestTried: Date?
    /// Per agent: the newest post date already given to it in its chat.
    private var chatSeen: [String: Int64] = [:]

    private init() {}

    /// A look every minute: one timer wake-up, nothing else, while the reviews are off.
    func start() {
        guard ticker == nil else { return }
        ticker = Task { [weak self] in
            try? await Task.sleep(for: .seconds(Picks.firstDelay))
            // A restart doesn't bring the next review forward: the last one (picks/last.json) still counts.
            self?.restoreLastRun()
            while !Task.isCancelled {
                await self?.tick()
                try? await Task.sleep(for: .seconds(60))
            }
        }
    }

    func notePicksPosted() { pending = true }

    private func restoreLastRun() {
        guard lastRun == nil else { return }
        let picksDir = PicksLedger.dir(workspace: AppState.shared.agentStore.workspace(id: Picks.agentID))
        if let at = PicksLedger.lastScan(picksDir).at {
            lastRun = min(Date(), Date(timeIntervalSince1970: TimeInterval(at) / 1000))
        }
    }

    private func tick() async {
        guard !scanning else { return }
        await morningSnapshot()
        await freeStats()
        let rules = AppState.shared.betting
        guard rules.autoScan else { return }
        await digestIfDue(rules)
        if let lastRun {
            let elapsed = Date().timeIntervalSince(lastRun)
            guard elapsed >= TimeInterval(rules.intervalMinutes) * 60 || (pending && elapsed >= Picks.postsGap) else { return }
        }
        // With the day's analysis done, a review only runs when there is something new: a channel posted a pick, or a match
        // of the analysis starts within 90 minutes and was not checked yet. Otherwise the turn is saved.
        if !worthReviewing() {
            lastRun = Date()
            return
        }
        do { _ = try await scan() } catch { MikaLog.info("parley: review failed: \(MessageError.text(error))") }
    }

    /// Once a day from 06:30 (or the first look after it when the Mac was off): the day's matches and prices are fetched
    /// once and saved (`odds/resumen-del-dia.md`, `hoy.csv`), so reviews and chat questions reuse them instead of asking
    /// again. Only with OddsPapi on and its key saved; marker `odds/daily.json`; a failure is retried after 15 minutes.
    /// Twin of `morning_snapshot` in picks.rs.
    private func morningSnapshot() async {
        let app = AppState.shared
        guard app.betting.useOddsApi, let key = KeychainStore.shared.get(Odds.keychainKey), !key.isEmpty else { return }
        let now = Date()
        let timeZone = TimeZone.current
        let cacheDir = Odds.cacheDir(agentsRoot: app.agentStore.root)
        let marker = cacheDir.appendingPathComponent("daily.json")
        let text = try? String(contentsOf: marker, encoding: .utf8)
        guard let day = Odds.morningDue(now: now, timeZone: timeZone, marker: text) else { return }
        if let tried = morningTried, now.timeIntervalSince(tried) < Odds.morningRetry { return }
        morningTried = now
        let rawDirs = [app.agentStore.workspace(id: Picks.agentID).appendingPathComponent("odds", isDirectory: true)]
        do {
            try await Odds.dailySnapshot(key: key, now: Int64(now.timeIntervalSince1970), timeZone: timeZone, cacheDir: cacheDir, rawDirs: rawDirs)
            try? FileManager.default.createDirectory(at: cacheDir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            try? Data("{\"day\":\"\(day)\"}".utf8).write(to: marker, options: .atomic)
        } catch {
            MikaLog.info("parley: morning snapshot failed: \(MessageError.text(error))")
        }
    }

    /// Free form of the day's teams and players (Football-Data, TennisMyLife), once a day after the morning snapshot has
    /// saved the match list (`odds/daily-fixtures.json`). Only with "Forma gratis" on; no key. Twin of the `daily_step` call
    /// in `morning_snapshot` (picks.rs). A failed run waits `FreeStats.retry`; files already reached today are not asked again.
    private func freeStats() async {
        let app = AppState.shared
        guard app.betting.useFreeStats else { return }
        let now = Date()
        let timeZone = TimeZone.current
        let day = ChatArchive.dateLabel(now, timeZone: timeZone)
        if statsDone == day { return }
        let cacheDir = Odds.cacheDir(agentsRoot: app.agentStore.root)
        let rawDirs = [app.agentStore.workspace(id: Picks.agentID).appendingPathComponent("odds", isDirectory: true)]
        let mayAttempt = statsTried.map { now.timeIntervalSince($0) >= FreeStats.retry } ?? true
        if !mayAttempt { return }
        let step = await FreeStats.dailyStep(now: now, timeZone: timeZone, cacheDir: cacheDir, rawDirs: rawDirs, mayAttempt: mayAttempt)
        switch step {
        case .waiting: break
        case .done: statsDone = day
        case .attempted(let complete):
            statsTried = now
            if complete { statsDone = day }
        }
    }

    /// The matches of the day's analysis already checked by a review (lineups, late injuries), by pair key.
    private var checkedMatches = Set<String>()
    /// How far ahead a review looks at the analysis' matches.
    static let reviewAhead: TimeInterval = 90 * 60

    /// Something new for a review: a post that looks like a pick since the last one, or an analysed match starting soon
    /// that no review checked yet. Without today's analysis, always (the review is the only analysis there is).
    private func worthReviewing() -> Bool {
        guard let digest = todayDigest() else { return true }
        let app = AppState.shared
        let inbox = TelegramInbox.dir(workspace: app.agentStore.workspace(id: Picks.agentID))
        let since = PicksLedger.lastScan(PicksLedger.dir(workspace: app.agentStore.workspace(id: Picks.agentID))).postsUntil
        if TelegramInbox.postsSince(inbox, since: since).contains(where: Picks.isCandidate) { return true }
        return !soonMatches(digest).isEmpty
    }

    /// The analysed matches that start within `reviewAhead` and were not checked yet.
    private func soonMatches(_ digest: Digest, now: Date = Date()) -> [DigestMatch] {
        let from = Picks.hour(now, timeZone: .current)
        let to = Picks.hour(now.addingTimeInterval(Self.reviewAhead), timeZone: .current)
        let today = ChatArchive.dateLabel(now, timeZone: .current)
        return digest.matches.filter {
            ($0.date.isEmpty || $0.date == today) && !$0.time.isEmpty && $0.time >= from && $0.time <= max(from, to)
                && !checkedMatches.contains(ParleyDigest.pairKey($0.home, $0.away))
        }
    }

    // MARK: - Daily digest

    /// Today's analysis (the JSON the page was drawn from), when it was made today.
    func todayDigest() -> Digest? {
        let workspace = AppState.shared.agentStore.workspace(id: Picks.agentID)
        let day = ChatArchive.dateLabel(Date(), timeZone: .current)
        guard let data = try? Data(contentsOf: ParleyDigest.jsonFile(workspace: workspace, day: day)) else { return nil }
        return try? JSONDecoder().decode(Digest.self, from: data)
    }

    /// Today's digest page, when it exists.
    func todayDigestURL() -> URL? {
        let app = AppState.shared
        let day = ChatArchive.dateLabel(Date(), timeZone: .current)
        let url = ParleyDigest.htmlFile(workspace: app.agentStore.workspace(id: Picks.agentID), day: day)
        return FileManager.default.fileExists(atPath: url.path) ? url : nil
    }

    /// Once a day, from the hour the user set, with the automatic reviews on: PARLEY searches every football and tennis
    /// match of the day and the page is made. A failure is retried after `ParleyDigest.retry`.
    private func digestIfDue(_ rules: BettingRules) async {
        guard rules.dailyDigest, !digesting, !scanning else { return }
        let now = Date()
        let workspace = AppState.shared.agentStore.workspace(id: Picks.agentID)
        let marker = try? String(contentsOf: ParleyDigest.dir(workspace: workspace).appendingPathComponent("last.json"), encoding: .utf8)
        guard ParleyDigest.due(now: now, timeZone: .current, hour: rules.digestHour, marker: marker) != nil else { return }
        // With Betano's odds on, the day's prices come first (the 06:30 snapshot); after 08:00 it does not wait any more.
        if rules.useOddsApi, Picks.hour(now, timeZone: .current) < "08:00" {
            let daily = try? String(contentsOf: Odds.cacheDir(agentsRoot: AppState.shared.agentStore.root).appendingPathComponent("daily.json"), encoding: .utf8)
            if !(daily?.contains(ChatArchive.dateLabel(now, timeZone: .current)) ?? false) { return }
        }
        if let tried = digestTried, now.timeIntervalSince(tried) < ParleyDigest.retry { return }
        digestTried = now
        do { _ = try await generateDigest() } catch { MikaLog.info("parley: digest failed: \(MessageError.text(error))") }
    }

    /// The day's analysis now (also "Generar ahora" in Settings): the agenda, the batches and the Telegram sweep
    /// (ParleyDigestPlan.swift) while PARLEY's pill works in the notch and the LED says what it is looking up; MIKA computes
    /// its own model, draws the page (`digest/<day>.html`), leaves PARLEY's summary with an "Abrir resumen" button in its
    /// chat, sends the macOS notification and asks in the notch "¿Lo ves?". Nothing opens by itself.
    /// Returns the page.
    @discardableResult
    func generateDigest() async throws -> URL {
        guard !digesting else { throw MessageError("PARLEY ya está armando el análisis del día.") }
        digesting = true
        defer { digesting = false }
        let app = AppState.shared
        let rules = app.betting
        guard let agent = app.hub.agents.first(where: { $0.id == Picks.agentID }) else {
            throw MessageError("PARLEY no está en la carpeta de agentes.")
        }
        let now = Date()
        let timeZone = TimeZone.current
        let day = ChatArchive.dateLabel(now, timeZone: timeZone)
        let workspace = app.agentStore.workspace(for: agent)
        let used = PicksLedger.usedToday(PicksLedger.dir(workspace: workspace), day: day, unit: rules.unit)
        let hasForm = FileManager.default.fileExists(atPath: workspace.appendingPathComponent("odds/stats-hoy.csv").path)
        // Every post of today, with who wrote it and its screenshots.
        let inbox = TelegramInbox.dir(workspace: workspace)
        let media = TelegramInbox.mediaDir(inbox)
        let posts = Array(TelegramInbox.postsSince(inbox, since: ParleyDigest.startOfDay(now, timeZone: timeZone) - 1)
            .suffix(ParleyDigest.maxPosts))
        var images: [String] = []
        for post in posts.reversed() {
            for name in post.photos where TelegramInbox.safeMediaName(name) && images.count < ParleyDigest.maxImages {
                let path = media.appendingPathComponent(name).path
                if FileManager.default.fileExists(atPath: path) { images.append(path) }
            }
        }
        let nowText = Picks.label(now, timeZone: timeZone)
        let from = max(ParleyDigest.dayStart, Picks.hour(now, timeZone: timeZone))
        let summaryFile = workspace.appendingPathComponent("odds/resumen-del-dia.md")
        let prices = ParleyDigest.oddsList((try? String(contentsOf: summaryFile, encoding: .utf8)) ?? "", from: from)
        let prompt = ParleyDigest.dayPrompt(rules: rules, day: day, now: nowText, from: from,
                                            posts: ParleyDigest.postsText(posts, timeZone: timeZone, media: media),
                                            prices: prices, hasForm: hasForm)

        // PARLEY works where the user can see it: its pill in the notch, the LED line, the compact island out.
        let progress = DigestProgress(name: agent.name, line: "analizando el día a partir de tus canales…")
        NotificationCenter.default.post(name: .hookReveal, object: nil)
        defer { progress.stop() }
        // One turn, medium reasoning (PARLEY's model); the fallback (Sonnet 5.5) takes over only without credits.
        let answer = try await app.runner.automatic(prompt: prompt, agent: agent, images: Array(images.prefix(ParleyDigest.dayImages)),
                                                    limit: ParleyDigest.dayLimit, session: "digest") { event in
            Task { @MainActor in progress.handle(event) }
        }
        let generated = Picks.hour(Date(), timeZone: timeZone)
        guard let parsed = ParleyDigest.parse(answer.text, day: day, generated: generated) else {
            throw MessageError("PARLEY respondió fuera de formato. Pulsa Generar ahora para repetirlo.")
        }
        var matches = parsed.matches
        for i in matches.indices {
            if matches[i].date.isEmpty { matches[i].date = day }
            if ParleyDigest.isStrong(matches[i].tournament) { matches[i].level = "fuerte" }
        }
        let ordered = matches.sorted { ($0.date, $0.time) < ($1.date, $1.time) }
        var assembled = ParleyDigest.assemble(matches: ordered, tips: parsed.tips, rules: rules, used: used, day: day,
                                              generated: generated, notes: parsed.warnings)
        if !parsed.summary.isEmpty { assembled.summary = parsed.summary + " " + assembled.summary }
        let digest: Digest? = assembled
        let html = ParleyDigest.html(assembled)
        let folder = ParleyDigest.dir(workspace: workspace)
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let page = ParleyDigest.htmlFile(workspace: workspace, day: day)
        do { try Data(html.utf8).write(to: page, options: .atomic) } catch { throw MessageError("No se pudo guardar el resumen: \(error.localizedDescription)") }
        if let data = try? JSONEncoder().encode(assembled) { try? data.write(to: ParleyDigest.jsonFile(workspace: workspace, day: day), options: .atomic) }
        try? Data("{\"day\":\"\(day)\"}".utf8).write(to: folder.appendingPathComponent("last.json"), options: .atomic)

        let line: String
        if let digest {
            let best = digest.picks.first.map { " Mejor: \($0.match) · \($0.market)." } ?? ""
            line = "\(digest.matches.count) partidos analizados, \(digest.picks.count) picks.\(best)"
        } else {
            line = "PARLEY respondió fuera de formato: su texto está en el resumen."
        }
        let summary = digest.map(ParleyDigest.chatSummary) ?? ""
        app.hub.recordAutomatic(agentID: agent.id, header: "Análisis del día · \(generated)", answer: summary, sources: answer.sources,
                                page: ChatPage(title: "Abrir resumen", path: page.path))
        await TelegramPoller.publishCard()
        Notifier.shared.parley(title: "PARLEY · análisis del día listo", body: line, digestPath: page.path)
        app.raiseIntegrationEvent(id: TelegramPoller.shared.id, success: true, label: "\(agent.name) · análisis del día", detail: line)
        app.agentSays(title: "PARLEY", text: "Análisis del día listo: \(line)", color: Picks.color, agent: Picks.agentID, led: false)
        // Nothing opens by itself (the user may be in a meeting): PARLEY asks in the notch whether to show it.
        NotificationCenter.default.post(name: .agentNotice, object: nil, userInfo: [
            "agent": Picks.agentID, "text": "Terminé el análisis del día. ¿Lo ves?", "action": "Ver", "path": page.path,
        ])
        return page
    }

    /// One review: PARLEY reads the posts since the last one, searches the web and proposes bets. Its answer (without
    /// the `[[picks]]` block) lands in its chat as "Revisión automática · HH:MM"; the picks go to the card, the ledger
    /// and, when there are some, the Telegram pill's badge and sound.
    func scan() async throws -> PicksScan {
        guard !scanning else { throw MessageError("PARLEY ya está revisando.") }
        scanning = true
        defer { scanning = false }
        let app = AppState.shared
        let rules = app.betting
        guard let agent = app.hub.agents.first(where: { $0.id == Picks.agentID }) else {
            throw MessageError("PARLEY no está en la carpeta de agentes.")
        }
        guard !app.hub.isAnswering(agent.id) else { throw MessageError("El agente está respondiendo en su chat.") }

        let now = Date()
        let timeZone = TimeZone.current
        let nowSecs = Int64(now.timeIntervalSince1970)
        let nowMs = ChatArchive.nowMs(now)
        let day = ChatArchive.dateLabel(now, timeZone: timeZone)
        let unit = rules.unit
        let workspace = app.agentStore.workspace(for: agent)
        let picksDir = PicksLedger.dir(workspace: workspace)
        let previous = PicksLedger.lastScan(picksDir)
        // New posts since the last review; a first review (or one after a long pause) looks at the last six hours.
        let since = max(previous.postsUntil, nowSecs - Picks.lookBack)
        let inbox = TelegramInbox.dir(workspace: workspace)
        let posts = Array(TelegramInbox.postsSince(inbox, since: since).suffix(Picks.maxPosts))
        let used = PicksLedger.usedToday(picksDir, day: day, unit: unit)
        let media = TelegramInbox.mediaDir(inbox)
        let windowEnd = nowSecs + Int64(rules.windowHours * 3600)
        // Real Betano odds, when the user switched them on and saved an OddsPapi key. A failure is told to PARLEY,
        // which then falls back on the web.
        // The day's analysis already has the prices: no OddsPapi or SGO call while it exists.
        let dayAnalysis = todayDigest()
        var odds: String?
        if rules.useOddsApi && dayAnalysis == nil {
            do {
                odds = try await Odds.snapshot(key: KeychainStore.shared.get(Odds.keychainKey), now: nowSecs, windowEnd: windowEnd,
                                               timeZone: timeZone, cacheDir: Odds.cacheDir(agentsRoot: app.agentStore.root))
            } catch {
                odds = "(No se pudieron leer: \(MessageError.text(error)) Busca las cuotas en la web.)"
            }
        }
        // Market reference (fair/consensus prices, live score and stats) from SportsGameOdds, and the only odds source
        // when OddsPapi is off or failed. A failure is told to PARLEY too.
        var reference: String?
        if rules.useSgo && dayAnalysis == nil {
            do {
                reference = try await Sgo.snapshot(key: KeychainStore.shared.get(Sgo.keychainKey), now: nowSecs, windowEnd: windowEnd,
                                                   timeZone: timeZone, cacheDir: Odds.cacheDir(agentsRoot: app.agentStore.root),
                                                   rawDirs: [workspace.appendingPathComponent("odds", isDirectory: true)])
            } catch {
                reference = "(No se pudieron leer: \(MessageError.text(error)))"
            }
        }
        // The morning analysis is the base: the review only checks what changed for the matches in its window.
        let base = dayAnalysis.map { ParleyDigest.base($0, from: Picks.hour(now, timeZone: timeZone), to: Picks.hour(windowEnd, timeZone: timeZone)) }
        if let dayAnalysis { for m in soonMatches(dayAnalysis, now: now) { checkedMatches.insert(ParleyDigest.pairKey(m.home, m.away)) } }
        let prompt = Picks.reviewPrompt(rules: rules, now: Picks.label(now, timeZone: timeZone),
                                        windowEnd: Picks.hour(windowEnd, timeZone: timeZone),
                                        used: used, posts: posts, timeZone: timeZone, media: media, odds: odds,
                                        reference: reference, base: base)
        let header = "Revisión automática · \(Picks.hour(now, timeZone: timeZone))"

        let answer: AutomaticAnswer
        do {
            let progress = DigestProgress(name: agent.name, line: "revisando picks y partidos…")
            defer { progress.stop() }
            // Only the new posts' screenshots, at most four: pictures are the heaviest part of a turn.
            answer = try await app.runner.automatic(prompt: prompt, agent: agent, images: Array(Picks.imagePaths(posts, media: media).prefix(4)),
                                                    limit: .seconds(600)) { event in Task { @MainActor in progress.handle(event) } }
        } catch {
            lastRun = Date()
            pending = false
            let message = MessageError.text(error)
            var failed = previous
            failed.at = nowMs
            failed.note = "No se pudo revisar: \(message)"
            failed.unit = unit
            PicksLedger.save(failed, to: picksDir)
            await TelegramPoller.publishCard()
            throw MessageError(message)
        }
        lastRun = Date()
        pending = false

        let (text, picks) = Picks.parse(answer.text, rules: rules, used: used)
        app.hub.recordAutomatic(agentID: agent.id, header: header, answer: text, sources: answer.sources)
        let firstLine = text.components(separatedBy: "\n").map { $0.trimmingCharacters(in: .whitespaces) }.first { !$0.isEmpty }
        let note = picks.isEmpty ? String((firstLine ?? "Sin picks en esta revisión.").prefix(160)) : nil
        let scan = PicksScan(at: nowMs, picks: picks, note: note, unit: unit,
                             postsUntil: max(posts.last?.date ?? since, previous.postsUntil))
        PicksLedger.save(scan, to: picksDir)
        PicksLedger.append(scan.picks, at: nowMs, day: day, unit: unit, to: picksDir)
        await TelegramPoller.publishCard()
        // Every review is announced with a macOS notification (even when it found nothing: "no picks" is an answer too),
        // with the buttons to see it and, once today's digest exists, to open that page.
        let hour = Picks.hour(now, timeZone: timeZone)
        let headline = picks.isEmpty
            ? (note ?? "Sin picks en esta revisión.")
            : "\(picks.count) pick\(picks.count == 1 ? "" : "s"): \(picks[0].partido) · \(picks[0].mercado) @ \(String(format: "%.2f", picks[0].cuota))"
        Notifier.shared.parley(title: "PARLEY · nuevo análisis de las \(hour)", body: headline, digestPath: todayDigestURL()?.path)
        // Only a complete pick the last review did not already have is worth saying: PARLEY greets like at launch, the
        // island coming out on its card, and, with the circle beside the notch, a bubble.
        let fresh = Picks.freshPicks(scan.picks, previous: previous.picks)
        if fresh.isEmpty {
            app.raiseIntegrationEvent(id: TelegramPoller.shared.id, success: true, label: "\(agent.name) · análisis de las \(hour)", detail: headline)
            app.agentSays(title: "PARLEY", text: "Nuevo análisis de las \(hour): \(headline)", color: Picks.color, agent: Picks.agentID)
        }
        if let first = fresh.first {
            let count = fresh.count
            app.raiseIntegrationEvent(id: TelegramPoller.shared.id, success: true,
                                      label: "\(agent.name) · \(count) pick\(count == 1 ? "" : "s")",
                                      detail: "\(first.partido) · \(first.mercado) @ \(String(format: "%.2f", first.cuota))")
            app.agentSays(title: "PARLEY", text: Picks.pickAnnouncement(fresh), color: Picks.color, agent: Picks.agentID)
            NotificationCenter.default.post(name: .agentGreets, object: nil, userInfo: ["id": TelegramPoller.shared.id])
        }
        return scan
    }

    /// What PARLEY gets in front of a question in its chat, and when MIKA passes it a request: the user's rules and the
    /// posts it has not seen yet (all of the last six hours when the conversation starts), with their screenshots.
    func chatNote(agent: AgentDefinition, fresh: Bool) -> ChatDataNote {
        let app = AppState.shared
        let rules = app.betting
        let now = Date()
        let timeZone = TimeZone.current
        let nowSecs = Int64(now.timeIntervalSince1970)
        let since = fresh ? nowSecs - Picks.lookBack : (chatSeen[agent.id] ?? nowSecs - Picks.lookBack)
        let inbox = TelegramInbox.dir(workspace: app.agentStore.workspace(for: agent))
        let posts = Array(TelegramInbox.postsSince(inbox, since: since).suffix(Picks.maxPosts))
        if let last = posts.last { chatSeen[agent.id] = last.date }
        let media = TelegramInbox.mediaDir(inbox)
        let picksDir = PicksLedger.dir(workspace: app.agentStore.workspace(id: Picks.agentID))
        let used = PicksLedger.usedToday(picksDir, day: ChatArchive.dateLabel(now, timeZone: timeZone), unit: rules.unit)
        // The chat starts from the morning analysis too: the picks of the day (the whole thing is in digest/).
        let base = fresh ? todayDigest().map { ParleyDigest.base($0) } : nil
        let text = Picks.chatNote(rules: rules, now: Picks.label(now, timeZone: timeZone), used: used, posts: posts,
                                  timeZone: timeZone, media: media, base: base)
        var note = ChatDataNote(text: text, images: Array(Picks.imagePaths(posts, media: media).prefix(4)))
        // The real Betano odds (OddsPapi) and the SGO market reference go along when the user has them on; the
        // snapshots are shared with the reviews (10 and 55 minutes).
        let oddsKey = rules.useOddsApi ? KeychainStore.shared.get(Odds.keychainKey).flatMap { $0.isEmpty ? nil : $0 } : nil
        let sgoKey = rules.useSgo ? KeychainStore.shared.get(Sgo.keychainKey).flatMap { $0.isEmpty ? nil : $0 } : nil
        // With today's analysis the chat works from it: no OddsPapi or SGO call per question.
        if (oddsKey != nil || sgoKey != nil) && todayDigest() == nil {
            let cacheDir = Odds.cacheDir(agentsRoot: app.agentStore.root)
            let rawDirs = [app.agentStore.workspace(id: Picks.agentID).appendingPathComponent("odds", isDirectory: true)]
            note.extra = { query in await Self.chatOdds(rules: rules, oddsKey: oddsKey, sgoKey: sgoKey, query: query, cacheDir: cacheDir, rawDirs: rawDirs) }
        }
        return note
    }

    /// Betano odds and the SGO reference for PARLEY's chat and MIKA's hand-off to it. Twin of `chat_odds` in picks.rs:
    /// the SGO table follows the question's own day or hours (`Odds.manualWindow`), and its intro says whether Betano's
    /// prices arrived.
    nonisolated static func chatOdds(rules: BettingRules, oddsKey: String?, sgoKey: String?, query: String, cacheDir: URL,
                                     rawDirs: [URL]) async -> String {
        let now = Int64(Date().timeIntervalSince1970)
        var out = ""
        var gotBetano = false
        if let oddsKey {
            let end = now + Int64(rules.windowHours * 3600)
            do {
                let table = try await Odds.snapshot(key: oddsKey, now: now, windowEnd: end, timeZone: .current, cacheDir: cacheDir)
                gotBetano = true
                out += Picks.chatOddsText(table)
            } catch {
                out += Picks.chatOddsUnavailable(MessageError.text(error))
            }
        }
        if let sgoKey {
            do {
                let table = try await Sgo.manualSnapshot(key: sgoKey, now: now, timeZone: .current, query: query, cacheDir: cacheDir, rawDirs: rawDirs)
                out += Picks.chatReferenceText(table, withBetano: gotBetano)
            } catch {
                out += Picks.chatReferenceUnavailable(MessageError.text(error))
            }
        }
        return out
    }
}

/// What PARLEY is doing during a long automatic turn: its pill's ticker shows the latest step, and the LED line under the
/// notch keeps saying it (refreshed every minute, so it never goes quiet while PARLEY reasons without searching).
@MainActor
final class DigestProgress {
    private let name: String
    private var line: String
    private var sources = 0
    private var lastLine = Date.distantPast
    private var heartbeat: Task<Void, Never>?

    init(name: String, line: String) {
        self.name = name
        self.line = line
        AppState.shared.setAgentWork(agentID: Picks.agentID, line: line)
        say()
        heartbeat = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(60))
                guard !Task.isCancelled else { return }
                self?.say()
            }
        }
    }

    /// A new headline (how many batches are done); the next search replaces it on the pill, the LED keeps saying it.
    func setLine(_ text: String) {
        line = text
        headline = text
        AppState.shared.setAgentWork(agentID: Picks.agentID, line: text)
        say()
    }

    /// The turn is over: the pill rests and the LED stops being refreshed.
    func stop() {
        heartbeat?.cancel()
        heartbeat = nil
        AppState.shared.setAgentWork(agentID: Picks.agentID, line: nil)
    }

    private var headline: String?

    /// The LED says the headline (batches done) when there is one, else the latest step.
    private func say() {
        AppState.shared.announce("\(name) · \(headline ?? line)", color: Picks.color, hold: 90)
    }

    func handle(_ event: ProviderEvent) {
        switch event {
        case .tool(let tool, let summary):
            let now = Date()
            guard now.timeIntervalSince(lastLine) > 2 else { return }
            lastLine = now
            line = ToolStatus.label(name: tool, summary: summary)
            AppState.shared.setAgentWork(agentID: Picks.agentID, line: line)
        case .source:
            sources += 1
            if sources % 10 == 0 {
                line = "\(sources) fuentes consultadas…"
                AppState.shared.setAgentWork(agentID: Picks.agentID, line: line)
            }
        default:
            break
        }
    }
}
