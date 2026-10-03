import Foundation
import LocalAuthentication
import UIKit

/// One message on screen.
struct Bubble: Identifiable, Equatable {
    let id = UUID()
    var role: String
    var agent: String
    var agentName: String
    var text: String
    var failed = false
    var took: String?
    var streaming = false
}

/// Everything the screens show: the paired machines, the one in use and what it says.
@MainActor
@Observable
final class AppModel {
    var machines: [Machine] = Machine.load()
    var current: Machine?
    private(set) var link: RemoteLink?

    var hello: Hello?
    var chats: [ChatSummary] = []
    var sessions: [SessionInfo] = []
    var usage: [ProviderUsage] = []
    var briefing: [BriefingItem] = []
    var approvals: [Approval] = []
    var sprite: Sprite?
    var mascotState = "idle"
    var suggestions: [String] = []

    /// The chat on screen.
    var chatID: String?
    var bubbles: [Bubble] = []
    var answering = false
    var activity: String?
    var chatError: String?

    var pairing = false
    var pairError: String?

    var status: RemoteLink.Status { link?.status ?? .offline("Sin equipos emparejados.") }

    init() {
        if let first = machines.first { select(first) }
    }

    func select(_ machine: Machine) {
        link?.disconnect()
        current = machine
        hello = nil; chats = []; sessions = []; usage = []; briefing = []; approvals = []
        newChat()
        let link = RemoteLink(machine: machine)
        link.onEvent = { [weak self] event in self?.handle(event) }
        link.onReady = { [weak self] in Task { await self?.refresh() } }
        self.link = link
        link.connect()
    }

    /// The phone came back to the app: a dropped connection is tried again at once.
    func wake() {
        if let link, link.status != .ready, link.status != .connecting { link.connect() }
    }

    func pair(_ uri: String) async {
        pairing = true
        pairError = nil
        defer { pairing = false }
        do {
            let machine = try await RemoteLink.pair(uri: uri.trimmingCharacters(in: .whitespacesAndNewlines), deviceName: UIDevice.current.name)
            machines.removeAll { $0.id == machine.id }
            machines.append(machine)
            Machine.save(machines)
            select(machine)
        } catch {
            pairError = error.localizedDescription
        }
    }

    func forget(_ machine: Machine) {
        machine.forget()
        machines.removeAll { $0.id == machine.id }
        Machine.save(machines)
        if current?.id == machine.id {
            link?.disconnect()
            link = nil
            current = nil
            if let next = machines.first { select(next) }
        }
    }

    /// Loads what the home shows. Each part on its own: one that fails leaves the others.
    func refresh() async {
        guard let link else { return }
        if let who: Hello = try? await link.call("hello") {
            hello = who
            if var machine = current, machine.name != who.name || machine.platform != who.platform {
                machine.name = who.name
                machine.platform = who.platform
                current = machine
                if let index = machines.firstIndex(where: { $0.id == machine.id }) { machines[index] = machine }
                Machine.save(machines)
            }
        }
        if sprite == nil { sprite = try? await link.call("agent_sprite", ["agentId": "buddy"]) }
        chats = (try? await link.call("chats", ["limit": 50])) ?? chats
        sessions = (try? await link.call("sessions")) ?? sessions
        usage = (try? await link.call("usage")) ?? usage
        briefing = (try? await link.call("briefing")) ?? briefing
        suggestions = (try? await link.call("chat_suggestions")) ?? suggestions
    }

    // MARK: Chat

    func newChat() {
        chatID = nil
        bubbles = []
        answering = false
        activity = nil
        chatError = nil
    }

    func open(_ chat: ChatSummary) async {
        newChat()
        chatID = chat.id
        guard let link else { return }
        do {
            let messages: [ChatMessage] = try await link.call("messages", ["chatId": chat.id])
            guard chatID == chat.id else { return }
            bubbles = messages.map { Bubble(role: $0.role, agent: $0.agent, agentName: AgentNames.name($0.agent), text: $0.text, failed: $0.failed, took: $0.took) }
        } catch {
            chatError = error.localizedDescription
        }
    }

    func send(_ text: String) async {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, let link else { return }
        chatError = nil
        bubbles.append(Bubble(role: "user", agent: "buddy", agentName: "", text: text))
        answering = true
        activity = "Pensando"
        do {
            var args: [String: Any] = ["text": text]
            if let chatID { args["chatId"] = chatID }
            chatID = try await link.call("send_message", args, as: String.self)
        } catch {
            answering = false
            activity = nil
            chatError = error.localizedDescription
        }
    }

    func stop() {
        guard let chatID, let link else { return }
        Task { try? await link.send("cancel_chat", ["chatId": chatID]) }
    }

    // MARK: Approvals

    /// Answers a permission request. Allowing asks for Face ID (or the phone's code) first, every time.
    func answer(_ approval: Approval, allow: Bool) async {
        guard let link else { return }
        if allow {
            let context = LAContext()
            var unavailable: NSError?
            // A phone with no code at all cannot prove who holds it: nothing is allowed from it.
            guard context.canEvaluatePolicy(.deviceOwnerAuthentication, error: &unavailable) else {
                chatError = "Pon un código o Face ID en este iPhone para poder permitir desde él."
                return
            }
            let reason = "Permitir a \(AgentNames.name(approval.agent)): \(approval.summary)"
            guard (try? await context.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason)) == true else { return }
        }
        do {
            try await link.send("answer_approval", ["requestId": approval.id, "allow": allow])
            approvals.removeAll { $0.id == approval.id }
        } catch {
            chatError = error.localizedDescription
        }
    }

    // MARK: Events

    private func handle(_ event: [String: Any]) {
        let text = { (key: String) in event[key] as? String ?? "" }
        let mine = text("chatId") == chatID && chatID != nil
        switch text("type") {
        case "chatStarted" where mine:
            answering = true
            activity = text("agent") == "buddy" ? "Pensando" : "\(text("agentName")) se encarga"
            if bubbles.last?.streaming != true {
                bubbles.append(Bubble(role: "assistant", agent: text("agent"), agentName: text("agentName"), text: "", streaming: true))
            } else {
                bubbles[bubbles.count - 1].agent = text("agent")
                bubbles[bubbles.count - 1].agentName = text("agentName")
            }
        case "chatDelta" where mine:
            if bubbles.last?.streaming != true {
                bubbles.append(Bubble(role: "assistant", agent: "buddy", agentName: "Buddy", text: "", streaming: true))
            }
            bubbles[bubbles.count - 1].text += text("text")
            activity = nil
        case "chatTool" where mine:
            activity = text("summary").isEmpty ? text("name") : text("summary")
        case "chatActivity" where mine:
            activity = text("label")
        case "chatDone" where mine:
            finish(failure: nil)
        case "chatFailed" where mine:
            finish(failure: text("message"))
        case "chatDone", "chatFailed":
            Task { await reloadChats() }
        case "mascotState":
            mascotState = text("state")
        case "sessionUpdate":
            let id = text("sessionId")
            sessions.removeAll { $0.sessionId == id }
            if text("state") != "ended" {
                sessions.insert(SessionInfo(sessionId: id, agent: text("agent"), project: text("project"), state: text("state"), updatedAt: Int64(Date().timeIntervalSince1970)), at: 0)
            }
        case "approvalRequest":
            let approval = Approval(id: text("requestId"), agent: text("agent"), project: text("project"), title: text("title"),
                                    summary: text("summary"), detail: text("detail"), canAllow: event["canAllow"] as? Bool ?? false)
            approvals.removeAll { $0.id == approval.id }
            approvals.append(approval)
        case "approvalClosed":
            approvals.removeAll { $0.id == text("requestId") }
        case "usageChanged":
            Task { [weak self] in if let fresh: [ProviderUsage] = try? await self?.link?.call("usage") { self?.usage = fresh } }
        case "briefingReady":
            Task { [weak self] in if let fresh: [BriefingItem] = try? await self?.link?.call("briefing") { self?.briefing = fresh } }
        default:
            break
        }
    }

    private func finish(failure: String?) {
        answering = false
        activity = nil
        if let index = bubbles.indices.last, bubbles[index].streaming {
            bubbles[index].streaming = false
            if let failure {
                bubbles[index].failed = true
                if bubbles[index].text.isEmpty { bubbles[index].text = failure }
            }
        } else if let failure {
            bubbles.append(Bubble(role: "assistant", agent: "buddy", agentName: "Buddy", text: failure, failed: true))
        }
        Task { await reloadChats() }
    }

    private func reloadChats() async {
        if let fresh: [ChatSummary] = try? await link?.call("chats", ["limit": 50]) { chats = fresh }
    }
}
