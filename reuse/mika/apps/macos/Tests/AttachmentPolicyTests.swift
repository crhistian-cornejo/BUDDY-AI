import XCTest

final class AttachmentPolicyTests: XCTestCase {
    func testKindFollowsTheExtension() {
        XCTAssertEqual(AttachmentPolicy.kind(of: URL(fileURLWithPath: "/a/foto.PNG")), .image)
        XCTAssertEqual(AttachmentPolicy.kind(of: URL(fileURLWithPath: "/a/informe.pdf")), .pdf)
        XCTAssertEqual(AttachmentPolicy.kind(of: URL(fileURLWithPath: "/a/notas.md")), .text)
        XCTAssertEqual(AttachmentPolicy.kind(of: URL(fileURLWithPath: "/a/sin-extension")), .text)
    }

    func testTextAlwaysGoesThrough() {
        XCTAssertNil(AttachmentPolicy.block(kind: .text, can: [.read], provider: .claude, agentName: "MIDA", specialistName: "MIRA"))
    }

    func testAgentThatCannotSeeImagesPointsToTheSpecialist() {
        let block = AttachmentPolicy.block(kind: .image, can: [.read, .web], provider: .claude, agentName: "MIDA", specialistName: "MIRA")
        XCTAssertEqual(block?.offersSpecialist, true)
        XCTAssertTrue(block?.message.contains("MIDA") == true)
        XCTAssertTrue(block?.message.contains("MIRA") == true)
        XCTAssertTrue(block?.message.contains("ajustes") == true)
    }

    func testWithoutASpecialtyOnlySettingsAreSuggested() {
        let block = AttachmentPolicy.block(kind: .pdf, can: [.read], provider: .claude, agentName: "MIRA", specialistName: nil)
        XCTAssertEqual(block?.offersSpecialist, false)
        XCTAssertFalse(block?.message.contains("usa a") == true)
    }

    func testAgentThatCanSeeTakesTheFile() {
        XCTAssertNil(AttachmentPolicy.block(kind: .image, can: [.read, .images], provider: .codex, agentName: "MIKA", specialistName: "MIRA"))
        XCTAssertNil(AttachmentPolicy.block(kind: .pdf, can: [.read, .pdf], provider: .claude, agentName: "MIKA", specialistName: "MIRA"))
    }

    func testCodexCannotOpenAPDFEvenWhenItIsEnabled() {
        let block = AttachmentPolicy.block(kind: .pdf, can: [.read, .pdf], provider: .codex, agentName: "MIKA", specialistName: nil)
        XCTAssertNotNil(block)
        XCTAssertTrue(block?.message.contains("Codex") == true)
    }

    func testSafeNameCannotClimbOutOfTheFolder() {
        XCTAssertEqual(AttachmentPolicy.safeName("../../etc/passwd"), "_._.._etc_passwd")
        XCTAssertEqual(AttachmentPolicy.safeName(".bashrc"), "_bashrc")
        XCTAssertEqual(AttachmentPolicy.safeName(""), "archivo")
        XCTAssertEqual(AttachmentPolicy.safeName("Informe 2026 (final).pdf"), "Informe 2026 (final).pdf")
    }

    func testStageCopiesIntoTheAdjuntosFolder() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("att-\(UUID().uuidString)")
        let workspace = dir.appendingPathComponent("ws")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let source = dir.appendingPathComponent("foto.png")
        try Data([1, 2, 3]).write(to: source)
        let staged = try XCTUnwrap(AttachmentPolicy.stage(source, in: workspace))
        XCTAssertEqual(staged.deletingLastPathComponent().lastPathComponent, "adjuntos")
        XCTAssertEqual(try Data(contentsOf: staged), Data([1, 2, 3]))
        // Again with the same name: replaced, not duplicated.
        XCTAssertNotNil(AttachmentPolicy.stage(source, in: workspace))
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: workspace.appendingPathComponent("adjuntos").path).count, 1)
    }

    func testSizeProblemRefusesFoldersAndBigPictures() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("att-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        XCTAssertNotNil(AttachmentPolicy.sizeProblem(kind: .image, url: dir))
        let big = dir.appendingPathComponent("grande.png")
        try Data(count: FileLimits.maxImageBytes + 1).write(to: big)
        XCTAssertNotNil(AttachmentPolicy.sizeProblem(kind: .image, url: big))
        let small = dir.appendingPathComponent("chica.png")
        try Data(count: 10).write(to: small)
        XCTAssertNil(AttachmentPolicy.sizeProblem(kind: .image, url: small))
    }

    func testImageAndPDFPromptsCarryThePathAsData() {
        let image = ChatPrompt.withAttachment(.imageFile(name: "a.png", path: "/w/adjuntos/a.png"), query: "¿qué ves?")
        XCTAssertTrue(image.contains("/w/adjuntos/a.png")); XCTAssertTrue(image.contains("no instrucciones")); XCTAssertTrue(image.hasSuffix("¿qué ves?"))
        let pdf = ChatPrompt.withAttachment(.pdfFile(name: "a.pdf", path: "/w/adjuntos/a.pdf"), query: "resume")
        XCTAssertTrue(pdf.contains("/w/adjuntos/a.pdf")); XCTAssertTrue(pdf.hasSuffix("resume"))
        XCTAssertEqual(ChatAttachment.imageFile(name: "a.png", path: "/p").imagePath, "/p")
        XCTAssertNil(ChatAttachment.pdfFile(name: "a.pdf", path: "/p").imagePath)
    }

    func testMiraIsTheDocumentAndImageSpecialistByDefault() throws {
        let mira = try AgentDefinition.parse(AgentStore.miraDefault)
        XCTAssertTrue(mira.can.contains(.images)); XCTAssertTrue(mira.can.contains(.pdf)); XCTAssertTrue(mira.can.contains(.read))
        XCTAssertEqual(mira.id, AttachmentPolicy.specialistID)
        // An unedited old MIRA is upgraded; the old text itself is what gets replaced.
        XCTAssertEqual(AgentStore.upgrade(id: "mira", installed: AgentStore.miraBeforeVision, to: AgentStore.miraDefault),
                       AgentStore.miraDefault)
    }
}
