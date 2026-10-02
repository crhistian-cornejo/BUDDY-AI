import Foundation

struct ProviderStatus: Equatable, Sendable {
    var installed: Bool
    var connected: Bool
    var detail: String
}

enum ProviderStatusParser {
    /// Only a subscription login counts; an API-key login would silently change billing.
    static func claude(output: Data) -> Bool {
        guard let obj = (try? JSONSerialization.jsonObject(with: output)) as? [String: Any],
              obj["loggedIn"] as? Bool == true, let method = obj["authMethod"] as? String else { return false }
        let m = method.lowercased()
        return m.contains("claude") || m.contains("oauth") || m.contains("subscription")
    }

    static func codex(output: String, exitCode: Int32) -> Bool {
        exitCode == 0 && output.lowercased().contains("chatgpt")
    }
}

enum ProviderStatusChecker {
    static func check(_ provider: ProviderID,
                      locate: @Sendable (String) -> URL? = { CLILocator.find($0) }) async -> ProviderStatus {
        let label = provider == .claude ? "Claude" : "Codex"
        guard let exe = locate(provider == .claude ? "claude" : "codex") else {
            return ProviderStatus(installed: false, connected: false, detail: "Instala el cliente oficial de \(label).")
        }
        let args = provider == .claude ? ["auth", "status"] : ["login", "status"]
        let p = ProviderProcess(executable: exe, arguments: args,
                                environment: ProviderEnvironment.scrubbed(ProcessInfo.processInfo.environment),
                                currentDirectory: FileManager.default.temporaryDirectory)
        do { try p.start(stdin: nil) } catch {
            return ProviderStatus(installed: true, connected: false, detail: "No se pudo consultar el cliente.")
        }
        var collected = ""
        for await line in p.lines { collected += line + "\n" }
        let code = await p.waitUntilExit()
        let connected = provider == .claude
            ? ProviderStatusParser.claude(output: Data(collected.utf8))
            : ProviderStatusParser.codex(output: collected + p.stderrText, exitCode: code)
        return ProviderStatus(installed: true, connected: connected,
                              detail: connected ? "Conectado con tu suscripción." : "Conecta tu cuenta con el cliente oficial.")
    }
}
