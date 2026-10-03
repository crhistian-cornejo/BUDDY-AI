import XCTest
@testable import Buddy

final class YouTubeNotchTests: XCTestCase {
    @MainActor
    func testViewerKeepsTheNotchOpenAndFitsTheOfficialPlayer() {
        let model = NotchModel()
        let video = YouTubeVideo(sourceId: "test", tabId: 7, videoId: "abcdefghijk", title: "Video de prueba", browser: "Chrome", seconds: 42, duration: 120, playing: true, caption: "", service: "youtube", url: "https://www.youtube.com/watch?v=abcdefghijk")
        model.youtube = YouTubeStatus(enabled: true, connected: true, detected: video, viewer: video, destination: "notch")
        model.pinned = true
        model.setHovering(false)
        XCTAssertEqual(model.mode, .open)
        let size = NotchLayout.size(model, notch: CGSize(width: 190, height: 32))
        XCTAssertEqual(size.width, 560)
        XCTAssertEqual(size.height, 388)
        model.collapse()
        XCTAssertEqual(model.mode, .idle)
    }
    @MainActor
    func testFloatingPlayerDoesNotReplaceNotchToolsAndStartsAbovePet() {
        let model = NotchModel(); model.pinned = true
        let video = YouTubeVideo(sourceId: "test", tabId: 7, videoId: "abcdefghijk", title: "Demo", browser: "Chrome", seconds: 42, duration: 120, playing: true, caption: "", service: "youtube", url: "https://www.youtube.com/watch?v=abcdefghijk")
        model.youtube = YouTubeStatus(enabled: true, connected: true, detected: video, viewer: video, destination: "floating")
        XCTAssertEqual(NotchLayout.size(model, notch: CGSize(width: 190, height: 32)).height, 292)
        let screen = NSRect(x: 0, y: 0, width: 1280, height: 800)
        let pet = NSRect(x: 1000, y: 40, width: 128, height: 128)
        let frame = VideoPlacement.frame(pet: pet, chat: nil, area: screen)
        XCTAssertTrue(screen.contains(frame))
        XCTAssertGreaterThanOrEqual(VideoPlacement.picture(in: frame).minY, pet.maxY, "the picture sits above Buddy, not over it")
        XCTAssertEqual(frame.midX, pet.midX, accuracy: 1, "centred on Buddy")
    }
    /// The video is part of Buddy: it goes where Buddy goes, and never leaves the screen.
    func testTheVideoFollowsThePetAndStaysOnScreen() {
        let screen = NSRect(x: 0, y: 0, width: 1280, height: 800)
        for pet in [NSRect(x: 0, y: 0, width: 128, height: 128), NSRect(x: 1152, y: 0, width: 128, height: 128),
                    NSRect(x: 600, y: 672, width: 128, height: 128), NSRect(x: 20, y: 400, width: 128, height: 128)] {
            let frame = VideoPlacement.frame(pet: pet, chat: nil, area: screen)
            XCTAssertTrue(screen.contains(frame), "\(pet)")
            XCTAssertFalse(VideoPlacement.picture(in: frame).intersects(pet), "the picture never covers Buddy: \(pet)")
            XCTAssertEqual(frame.size, VideoPlacement.windowSize)
        }
        // Moving Buddy moves the video by the same amount while there is room.
        let a = VideoPlacement.frame(pet: NSRect(x: 500, y: 40, width: 128, height: 128), chat: nil, area: screen)
        let b = VideoPlacement.frame(pet: NSRect(x: 560, y: 70, width: 128, height: 128), chat: nil, area: screen)
        XCTAssertEqual(b.minX - a.minX, 60, accuracy: 0.5); XCTAssertEqual(b.minY - a.minY, 30, accuracy: 0.5)
    }
    /// Opening the chat moves the video out of its way, still next to Buddy and still whole.
    func testTheVideoMovesAsideWhenTheChatOpens() {
        let screen = NSRect(x: 0, y: 0, width: 1280, height: 800)
        // Buddy at the right: the chat opens at its left. A short chat (only the composer) leaves the place above Buddy free.
        let pet = NSRect(x: 1100, y: 40, width: 128, height: 128)
        let composer = NSRect(x: 652, y: 46, width: 440, height: 44)
        let above = VideoPlacement.frame(pet: pet, chat: composer, area: screen)
        XCTAssertFalse(VideoPlacement.picture(in: above).intersects(composer)); XCTAssertFalse(VideoPlacement.picture(in: above).intersects(pet))
        // A tall chat takes that place: the video goes above the chat.
        let tall = NSRect(x: 652, y: 46, width: 440, height: 452)
        let moved = VideoPlacement.frame(pet: pet, chat: tall, area: screen)
        XCTAssertTrue(screen.contains(moved))
        XCTAssertFalse(VideoPlacement.picture(in: moved).intersects(tall), "\(moved)"); XCTAssertFalse(VideoPlacement.picture(in: moved).intersects(pet))
        // The chat at Buddy's right: the video stays above Buddy, pushed to the side the chat is not on.
        let middle = NSRect(x: 560, y: 40, width: 128, height: 128)
        let right = NSRect(x: 696, y: 46, width: 440, height: 452)
        let side = VideoPlacement.frame(pet: middle, chat: right, area: screen)
        XCTAssertLessThanOrEqual(VideoPlacement.picture(in: side).maxX, right.minX, "clear of the chat: \(side)")
        XCTAssertFalse(VideoPlacement.picture(in: side).intersects(middle))
        XCTAssertLessThan(VideoPlacement.picture(in: side).minY - middle.maxY, 20, "and still by Buddy")
        // Closing the chat brings it back over Buddy.
        XCTAssertEqual(VideoPlacement.frame(pet: middle, chat: nil, area: screen).midX, middle.midX, accuracy: 1)
    }
    /// While the chat grows the video rides on top of it: it climbs, it never jumps from side to side.
    func testTheVideoKeepsItsColumnWhileTheChatGrows() {
        let screen = NSRect(x: 0, y: 0, width: 1280, height: 800)
        let pet = NSRect(x: 1100, y: 40, width: 128, height: 128)
        var last: NSRect?
        for height in stride(from: 44, through: 452, by: 68) {
            let chat = NSRect(x: 652, y: 46, width: 440, height: CGFloat(height))
            let frame = VideoPlacement.frame(pet: pet, chat: chat, area: screen)
            XCTAssertFalse(frame.intersects(chat), "\(height)"); XCTAssertFalse(frame.intersects(pet), "\(height)")
            XCTAssertEqual(frame.minY, chat.maxY + VideoPlacement.gap, accuracy: 0.5, "right on top of the chat: \(height)")
            if let last { XCTAssertEqual(frame.minX, last.minX, accuracy: 0.5, "same column: \(height)") }
            last = frame
        }
        // Next to Buddy: its edge is the chat's edge by the pet.
        XCTAssertEqual(last?.maxX ?? 0, 1092, accuracy: 0.5)
    }
    /// The corner makes the picture a little larger, never small or huge, always 16:9.
    func testTheVideoResizesWithinLimitsAndKeepsItsShape() {
        XCTAssertEqual(VideoPlacement.size(width: 100), CGSize(width: 384, height: 216))
        XCTAssertEqual(VideoPlacement.size(width: 512), CGSize(width: 512, height: 288))
        XCTAssertEqual(VideoPlacement.size(width: 5000), CGSize(width: 640, height: 360))
        let screen = NSRect(x: 0, y: 0, width: 1280, height: 800)
        let pet = NSRect(x: 1100, y: 40, width: 128, height: 128)
        let large = VideoPlacement.frame(pet: pet, chat: nil, area: screen, size: VideoPlacement.size(width: 640))
        XCTAssertEqual(large.size, CGSize(width: 640, height: 360))
        XCTAssertTrue(screen.contains(large)); XCTAssertFalse(large.intersects(pet))
    }
    /// The window is the picture: nothing around it to draw a border or a shadow with.
    func testTheVideoWindowIsOnlyThePicture() {
        XCTAssertEqual(VideoPlacement.windowSize, VideoPlacement.pictureSize)
        let frame = NSRect(origin: .zero, size: VideoPlacement.windowSize)
        XCTAssertEqual(VideoPlacement.picture(in: frame), frame)
    }
    /// With the notch closed, a new playback position alone changes nothing on screen.
    func testOnlyThePositionChangingIsNotNewsForAClosedNotch() {
        let video = { (seconds: Double, playing: Bool) in YouTubeVideo(sourceId: "test", tabId: 7, videoId: "abcdefghijk", title: "Demo", browser: "Chrome", seconds: seconds, duration: 120, playing: playing, caption: "", service: "youtube", url: "https://www.youtube.com/watch?v=abcdefghijk") }
        let status = { (v: YouTubeVideo) in YouTubeStatus(enabled: true, connected: true, detected: v, viewer: nil, destination: "notch") }
        XCTAssertTrue(NotchController.sameButPosition(status(video(10, true)), status(video(11, true))))
        XCTAssertFalse(NotchController.sameButPosition(status(video(10, true)), status(video(10, false))))
        XCTAssertFalse(NotchController.sameButPosition(nil, status(video(10, true))))
        XCTAssertTrue(NotchController.sameButPosition(nil, nil))
    }
    @MainActor
    func testDetectionUsesTheMusicCardHeight() {
        let model = NotchModel(); model.pinned = true
        let video = YouTubeVideo(sourceId: "test", tabId: 7, videoId: "abcdefghijk", title: "Video de prueba", browser: "Chrome", seconds: 42, duration: 120, playing: true, caption: "", service: "youtube", url: "https://www.youtube.com/watch?v=abcdefghijk")
        model.youtube = YouTubeStatus(enabled: true, connected: true, detected: video, viewer: nil, destination: "notch")
        XCTAssertEqual(NotchLayout.size(model, notch: CGSize(width: 190, height: 32)).height, 292)
    }
}
