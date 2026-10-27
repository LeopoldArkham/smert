// Desktop wallpaper showing smert's daily artwork, from /api/artwork, and
// its daily quote, from /api/quote, while the screen is locked.
//
// A background agent (launchd keeps it running, see install.sh) that draws
// both to a PNG for each screen and sets one as that screen's wallpaper. The
// lock screen always shows the wallpaper, so the agent switches to the quote
// when the screen locks and back to the artwork when it unlocks.
// It listens to /api/events and refetches as soon as either is rerolled,
// and refetches at the daily rollover, which sends no event. macOS only sets
// the wallpaper of the current Space, so it reapplies it whenever the active
// Space changes.
//
// `SmertWallpaper --once` fetches, sets the wallpaper and exits.

import AppKit
import CoreText
import UniformTypeIdentifiers

/// The smert server; `SMERT_URL` overrides it, e.g. for a local `cargo run`.
let serverURL = ProcessInfo.processInfo.environment["SMERT_URL"]
    ?? "http://dx63m4krbosa1k4t6nitmay6.167.233.56.221.sslip.io"
let eventsURL = URL(string: serverURL + "/api/events")!

/// Refetch at least this often, in case a change was pushed while the event
/// stream was down without the agent noticing.
let refreshInterval: TimeInterval = 60 * 60
/// The server sends a keep-alive every 20 s; this long without one means the
/// event stream is dead, even if it hasn't closed.
let eventsTimeout: TimeInterval = 60
/// Reconnecting waits 1 s after the first failure, doubling up to this.
let maxReconnectDelay: TimeInterval = 60

let supportDir = FileManager.default.homeDirectoryForCurrentUser
    .appendingPathComponent("Library/Application Support/smert-wallpaper")
let imagesDir = supportDir.appendingPathComponent("wallpapers")
/// Today's artwork as downloaded, before it is drawn into a wallpaper.
let artworksDir = supportDir.appendingPathComponent("artworks")
let fontsDir = supportDir.appendingPathComponent("fonts")

// The display page's palette.
let background = CGColor(srgbRed: 0x21 / 255, green: 0x22 / 255, blue: 0x2d / 255, alpha: 1)
let ink = CGColor(srgbRed: 0xff / 255, green: 0xf6 / 255, blue: 0xec / 255, alpha: 1)
let muted = CGColor(srgbRed: 0x7a / 255, green: 0x78 / 255, blue: 0x70 / 255, alpha: 1)

func rgb(_ hex: UInt32) -> CGColor {
    CGColor(srgbRed: CGFloat(hex >> 16 & 0xff) / 255, green: CGFloat(hex >> 8 & 0xff) / 255,
            blue: CGFloat(hex & 0xff) / 255, alpha: 1)
}

/// Quote size in points on a 1440-point-wide screen, as on the display page;
/// it scales with the screen's width.
let baseSize: CGFloat = 32
/// Text shrinks down to this fraction of its normal size to fit.
let minScale: CGFloat = 0.5

struct QuotePayload: Decodable {
    struct Quote: Decodable {
        let text: String
        let author: String
    }

    let nextRollover: String
    let quote: Quote?
}

struct ArtworkPayload: Decodable {
    struct Artwork: Decodable {
        let id: Int64
        /// Relative to the server.
        let imageUrl: String
        let artist: String
        let work: String
        let date: String

        var file: URL {
            let ext = URL(string: imageUrl)?.pathExtension ?? ""
            return artworksDir.appendingPathComponent(ext.isEmpty ? "\(id)" : "\(id).\(ext)")
        }
    }

    let nextRollover: String
    let artwork: Artwork?
}

/// What the agent fetches; each is also the name of its kind in /api/events.
enum Feed: String, CaseIterable {
    case quote, artwork

    var api: URL { URL(string: serverURL + "/api/" + rawValue)! }
    var cache: URL { supportDir.appendingPathComponent(rawValue + ".json") }
}

func cached<T: Decodable>(_ type: T.Type, from url: URL) -> T? {
    let decoder = JSONDecoder()
    decoder.keyDecodingStrategy = .convertFromSnakeCase
    return (try? Data(contentsOf: url)).flatMap { try? decoder.decode(type, from: $0) }
}

/// Whether the screen is locked right now; the lock notifications keep it
/// up to date after that.
func screenIsLocked() -> Bool {
    (CGSessionCopyCurrentDictionary() as? [String: Any])?["CGSSessionScreenIsLocked"] as? Bool ?? false
}

final class Agent: NSObject {
    private var fetching = Set<Feed>()
    /// A change was pushed during a fetch, which may have missed it.
    private var fetchAgain = Set<Feed>()
    private var rolloverTimer: Timer?
    private var locked = screenIsLocked()

    private lazy var events = URLSession(configuration: {
        let config = URLSessionConfiguration.default
        config.timeoutIntervalForRequest = eventsTimeout
        // A day, then it reconnects; there is no reason to ever end it sooner.
        config.timeoutIntervalForResource = 24 * 3600
        return config
    }(), delegate: self, delegateQueue: .main)
    private var stream: URLSessionDataTask?
    private var buffer = Data()
    private var failures = 0
    private var reconnectTimer: Timer?

    // ── Lifecycle ─────────────────────────────────────────────────────────────

    func start() {
        apply()
        refresh()
        listen()

        let workspace = NSWorkspace.shared.notificationCenter
        workspace.addObserver(self, selector: #selector(didWake), name: NSWorkspace.didWakeNotification, object: nil)
        workspace.addObserver(self, selector: #selector(apply), name: NSWorkspace.activeSpaceDidChangeNotification, object: nil)
        NotificationCenter.default.addObserver(
            self, selector: #selector(apply),
            name: NSApplication.didChangeScreenParametersNotification, object: nil)
        // Delivered at once: AppKit otherwise holds them while the app is
        // inactive, which this one always is.
        let distributed = DistributedNotificationCenter.default()
        distributed.addObserver(self, selector: #selector(didLock), name: .init("com.apple.screenIsLocked"),
                                object: nil, suspensionBehavior: .deliverImmediately)
        distributed.addObserver(self, selector: #selector(didUnlock), name: .init("com.apple.screenIsUnlocked"),
                                object: nil, suspensionBehavior: .deliverImmediately)

        Timer.scheduledTimer(withTimeInterval: refreshInterval, repeats: true) { [weak self] _ in
            self?.refresh()
        }
    }

    /// The event stream died with the network during sleep: reconnect at
    /// once, which also refetches.
    @objc private func didWake() {
        failures = 0
        listen()
    }

    /// The lock screen shows the wallpaper: put the quote up.
    @objc private func didLock() {
        locked = true
        apply()
    }

    @objc private func didUnlock() {
        locked = false
        apply()
    }

    // ── Data ──────────────────────────────────────────────────────────────────

    /// Fetch today's quote and artwork; on success cache them and redraw. On
    /// failure the cached ones stay up.
    @objc func refresh() {
        fetchAll(then: nil)
    }

    func fetchAll(then done: (() -> Void)?) {
        let group = DispatchGroup()
        for feed in Feed.allCases {
            group.enter()
            fetch(feed) { group.leave() }
        }
        group.notify(queue: .main) { done?() }
    }

    func fetch(_ feed: Feed, then done: (() -> Void)? = nil) {
        if fetching.contains(feed) {
            fetchAgain.insert(feed)
            done?()
            return
        }
        fetching.insert(feed)
        var request = URLRequest(url: feed.api, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 10)
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        URLSession.shared.dataTask(with: request) { data, response, error in
            DispatchQueue.main.async {
                let ok = (response as? HTTPURLResponse)?.statusCode == 200
                if let data, ok {
                    try? data.write(to: feed.cache, options: .atomic)
                } else {
                    log("\(feed) fetch failed: \(error?.localizedDescription ?? "HTTP \((response as? HTTPURLResponse)?.statusCode ?? 0)")")
                }
                let finish = {
                    self.fetching.remove(feed)
                    self.apply()
                    self.scheduleRollover()
                    done?()
                    if self.fetchAgain.remove(feed) != nil {
                        self.fetch(feed)
                    }
                }
                if feed == .artwork {
                    self.download(cached(ArtworkPayload.self, from: feed.cache)?.artwork, then: finish)
                } else {
                    finish()
                }
            }
        }.resume()
    }

    /// Download the artwork's image unless it already is, and delete the
    /// images of earlier ones.
    private func download(_ artwork: ArtworkPayload.Artwork?, then done: @escaping () -> Void) {
        try? FileManager.default.createDirectory(at: artworksDir, withIntermediateDirectories: true)
        for file in (try? FileManager.default.contentsOfDirectory(at: artworksDir, includingPropertiesForKeys: nil)) ?? []
        where file.lastPathComponent != artwork?.file.lastPathComponent {
            try? FileManager.default.removeItem(at: file)
        }
        guard let artwork, !FileManager.default.fileExists(atPath: artwork.file.path),
              let url = URL(string: artwork.imageUrl, relativeTo: URL(string: serverURL))
        else { return done() }
        URLSession.shared.downloadTask(with: url) { downloaded, response, error in
            // The download is deleted once this returns, so move it now.
            if let downloaded, (response as? HTTPURLResponse)?.statusCode == 200 {
                try? FileManager.default.moveItem(at: downloaded, to: artwork.file)
            } else {
                log("artwork download failed: \(error?.localizedDescription ?? "HTTP \((response as? HTTPURLResponse)?.statusCode ?? 0)")")
            }
            DispatchQueue.main.async(execute: done)
        }.resume()
    }

    /// Fetch again a minute after the next rollover.
    private func scheduleRollover() {
        rolloverTimer?.invalidate()
        let rollovers = [
            cached(QuotePayload.self, from: Feed.quote.cache)?.nextRollover,
            cached(ArtworkPayload.self, from: Feed.artwork.cache)?.nextRollover,
        ]
        guard let at = rollovers.compactMap({ $0.flatMap(ISO8601DateFormatter().date(from:)) }).min() else { return }
        let delay = max(at.timeIntervalSinceNow + 60, 60)
        rolloverTimer = Timer.scheduledTimer(withTimeInterval: delay, repeats: false) { [weak self] _ in
            self?.refresh()
        }
    }

    // ── Push ──────────────────────────────────────────────────────────────────

    /// (Re)open the event stream, replacing any open one.
    private func listen() {
        reconnectTimer?.invalidate()
        stream?.cancel()
        buffer.removeAll()
        var request = URLRequest(url: eventsURL)
        request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
        stream = events.dataTask(with: request)
        stream?.resume()
    }

    private func reconnectLater() {
        let delay = min(pow(2, Double(failures)), maxReconnectDelay)
        failures += 1
        reconnectTimer?.invalidate()
        reconnectTimer = Timer.scheduledTimer(withTimeInterval: delay, repeats: false) { [weak self] _ in
            self?.listen()
        }
    }

    /// One line of the event stream: `data: <kind>` when a pick changed.
    /// Comments (keep-alives) and blank lines between events are ignored.
    private func handle(line: String) {
        guard line.hasPrefix("data:") else { return }
        if let feed = Feed(rawValue: line.dropFirst("data:".count).trimmingCharacters(in: .whitespaces)) {
            fetch(feed)
        }
    }

    // ── Wallpaper ─────────────────────────────────────────────────────────────

    /// Draw the cached quote and artwork for every screen and set one as the
    /// wallpaper of the current Space, where it isn't already: the quote while
    /// the screen is locked, the artwork otherwise, or the quote until there
    /// is an artwork.
    @objc func apply() {
        let quote = cached(QuotePayload.self, from: Feed.quote.cache)
        let artwork = cached(ArtworkPayload.self, from: Feed.artwork.cache)?.artwork
        var current = Set<URL>()
        for screen in NSScreen.screens {
            let canvas = Canvas(screen)
            let quoteImage = quote.flatMap { quoteWallpaper(canvas, $0) }
            let artworkImage = artwork.flatMap { artworkWallpaper(canvas, $0) }
            current.formUnion([quoteImage, artworkImage].compactMap { $0 })
            guard let url = locked ? quoteImage ?? artworkImage : artworkImage ?? quoteImage else { continue }
            if NSWorkspace.shared.desktopImageURL(for: screen)?.standardizedFileURL == url.standardizedFileURL {
                continue
            }
            do {
                try NSWorkspace.shared.setDesktopImageURL(url, for: screen, options: [
                    .imageScaling: NSImageScaling.scaleProportionallyUpOrDown.rawValue,
                    .allowClipping: true,
                    .fillColor: NSColor(cgColor: background)!,
                ])
            } catch {
                log("could not set the wallpaper: \(error.localizedDescription)")
            }
        }
        prune(keeping: current)
    }

    private func quoteWallpaper(_ canvas: Canvas, _ payload: QuotePayload) -> URL? {
        let text = payload.quote.map { "\($0.text)\u{0}\($0.author)" } ?? ""
        return wallpaper("quote", "\(text)|\(canvas)") { render(payload.quote, canvas) }
    }

    /// Nil until the artwork's image is downloaded.
    private func artworkWallpaper(_ canvas: Canvas, _ artwork: ArtworkPayload.Artwork) -> URL? {
        guard FileManager.default.fileExists(atPath: artwork.file.path) else { return nil }
        return wallpaper("artwork", "\(artwork.id)|\(artwork.work)|\(artwork.artist)|\(artwork.date)|\(canvas)") {
            render(artwork, canvas)
        }
    }

    /// The wallpaper image `draw` makes, drawn if it isn't on disk yet. Its
    /// name hashes `key`, what it shows: macOS ignores a new image at the
    /// same path.
    private func wallpaper(_ kind: String, _ key: String, draw: () -> CGImage?) -> URL? {
        let url = imagesDir.appendingPathComponent(String(format: "\(kind)-%016llx.png", fnv1a(key)))
        if FileManager.default.fileExists(atPath: url.path) { return url }
        guard let image = draw(), let dest = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)
        else { return nil }
        CGImageDestinationAddImage(dest, image, nil)
        return CGImageDestinationFinalize(dest) ? url : nil
    }

    /// Delete wallpapers no screen shows, once they are a day old: other
    /// Spaces may still show them until they are next visited.
    private func prune(keeping current: Set<URL>) {
        let files = (try? FileManager.default.contentsOfDirectory(
            at: imagesDir, includingPropertiesForKeys: [.contentModificationDateKey])) ?? []
        let keep = Set(current.map(\.lastPathComponent))
        for file in files where !keep.contains(file.lastPathComponent) {
            let modified = (try? file.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate
            if let modified, modified.timeIntervalSinceNow < -24 * 3600 {
                try? FileManager.default.removeItem(at: file)
            }
        }
    }
}

extension Agent: URLSessionDataDelegate {
    func urlSession(
        _ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse,
        completionHandler: @escaping (URLSession.ResponseDisposition) -> Void
    ) {
        guard dataTask === stream, (response as? HTTPURLResponse)?.statusCode == 200 else {
            completionHandler(.cancel)
            return
        }
        // Connected: refetch to catch anything pushed while disconnected.
        failures = 0
        refresh()
        completionHandler(.allow)
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        guard dataTask === stream else { return }
        buffer.append(data)
        while let newline = buffer.firstIndex(of: UInt8(ascii: "\n")) {
            let line = String(decoding: buffer[buffer.startIndex..<newline], as: UTF8.self)
            buffer.removeSubrange(buffer.startIndex...newline)
            handle(line: line.trimmingCharacters(in: .newlines))
        }
    }

    /// The stream ended: the server restarted, the network dropped, or it
    /// went quiet past `eventsTimeout`. A stream `listen` replaced is ignored.
    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        guard task === stream else { return }
        log("event stream closed: \(error?.localizedDescription ?? "by the server")")
        stream = nil
        reconnectLater()
    }
}

// ── Drawing ───────────────────────────────────────────────────────────────────

/// A screen's wallpaper, in pixels.
struct Canvas: CustomStringConvertible {
    let pixels: CGSize
    /// Between the menu bar and the Dock, where things are centered.
    let box: CGRect
    /// The quote's font size; the rest scales with it.
    let size: CGFloat
    /// Pixels per point.
    let scale: CGFloat

    init(_ screen: NSScreen) {
        scale = screen.backingScaleFactor
        pixels = CGSize(width: screen.frame.width * scale, height: screen.frame.height * scale)
        let visible = screen.visibleFrame
        box = CGRect(
            x: 0,
            y: (visible.minY - screen.frame.minY) * scale,
            width: pixels.width,
            height: visible.height * scale)
        size = baseSize * screen.frame.width / 1440 * scale
    }

    var description: String { "\(Int(pixels.width))x\(Int(pixels.height))|\(box)|\(size)" }

    /// A context filled with the background.
    func context() -> CGContext? {
        guard let ctx = CGContext(
            data: nil, width: Int(pixels.width), height: Int(pixels.height), bitsPerComponent: 8,
            bytesPerRow: 0, space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)
        else { return nil }
        ctx.setFillColor(background)
        ctx.fill(CGRect(origin: .zero, size: pixels))
        return ctx
    }
}

/// The quote centered in `box`, with "— author" in italics below it, as on the
/// display page; shrunk to fit if it is long.
func render(_ quote: QuotePayload.Quote?, _ canvas: Canvas) -> CGImage? {
    guard let ctx = canvas.context() else { return nil }
    let (pixels, box, size) = (canvas.pixels, canvas.box, canvas.size)

    let width = (pixels.width * 0.7).rounded()
    let maxHeight = box.height * 0.85
    var scale: CGFloat = 1
    var (framesetter, height) = layout(quote, size: size, width: width)
    while height > maxHeight && scale > minScale {
        scale -= 0.05
        (framesetter, height) = layout(quote, size: size * scale, width: width)
    }

    // CoreText lays out from the top of the rect, so slack goes at the bottom.
    let slack = size
    let rect = CGRect(x: (pixels.width - width) / 2, y: box.midY - height / 2 - slack, width: width, height: height + slack)
    let frame = CTFramesetterCreateFrame(framesetter, CFRange(), CGPath(rect: rect, transform: nil), nil)
    CTFrameDraw(frame, ctx)
    return ctx.makeImage()
}

/// The artwork in its bronze frame, with its title in italics and "artist,
/// date" below, centered in `box` as on the display page.
func render(_ artwork: ArtworkPayload.Artwork, _ canvas: Canvas) -> CGImage? {
    guard let ctx = canvas.context(),
          let source = CGImageSourceCreateWithURL(artwork.file as CFURL, nil),
          // Turned upright, and no larger than the screen.
          let image = CGImageSourceCreateThumbnailAtIndex(source, 0, [
              kCGImageSourceCreateThumbnailFromImageAlways: true,
              kCGImageSourceCreateThumbnailWithTransform: true,
              kCGImageSourceThumbnailMaxPixelSize: max(canvas.pixels.width, canvas.pixels.height),
          ] as CFDictionary)
    else { return nil }
    let (pixels, box, scale) = (canvas.pixels, canvas.box, canvas.scale)
    let rem = canvas.size / 2

    // Caption.
    let text = NSMutableAttributedString()
    text.append(paragraph(artwork.work, font: font(italic: true, size: rem * 1.3), color: ink))
    let meta = [artwork.artist, artwork.date].filter { !$0.isEmpty }.joined(separator: ", ")
    if !meta.isEmpty {
        text.append(paragraph("\n" + meta, font: font(italic: false, size: rem), color: muted, spaceBefore: rem * 0.4))
    }
    let textWidth = (pixels.width * 0.7).rounded()
    let framesetter = CTFramesetterCreateWithAttributedString(text)
    let textHeight = CTFramesetterSuggestFrameSizeWithConstraints(
        framesetter, CFRange(), nil, CGSize(width: textWidth, height: .greatestFiniteMagnitude), nil
    ).height.rounded(.up)
    let gap = rem * 1.25

    // The frame: a bevelled border, then a bronze band around the painting.
    let border = 2 * scale
    let band = min(max(4 * scale, pixels.height * 0.0075), 10 * scale)
    let edge = border + band
    let maxWidth = pixels.width * 0.9 - 2 * edge
    let maxHeight = box.height * 0.92 - textHeight - gap - 2 * edge
    // Never larger than the display page shows it: its own size, in points.
    let fit = min(maxWidth / CGFloat(image.width), maxHeight / CGFloat(image.height), scale)
    let painting = CGSize(width: (CGFloat(image.width) * fit).rounded(), height: (CGFloat(image.height) * fit).rounded())

    let height = painting.height + 2 * edge + gap + textHeight
    let bottom = (box.midY - height / 2).rounded()
    let outer = CGRect(
        x: ((pixels.width - painting.width) / 2 - edge).rounded(), y: bottom + textHeight + gap,
        width: painting.width + 2 * edge, height: painting.height + 2 * edge)
    let inner = outer.insetBy(dx: edge, dy: edge)

    // The bevel, lit from the top left: light on the top and left edges.
    ctx.setFillColor(rgb(0x47301a))
    ctx.fill(outer)
    ctx.setFillColor(rgb(0x5e3f1e))
    ctx.fill(CGRect(x: outer.maxX - border, y: outer.minY, width: border, height: outer.height))
    ctx.setFillColor(rgb(0xc4a271))
    ctx.fill(CGRect(x: outer.minX, y: outer.minY + border, width: border, height: outer.height - border))
    ctx.setFillColor(rgb(0xd9b98a))
    ctx.fill(CGRect(x: outer.minX, y: outer.maxY - border, width: outer.width - border, height: border))

    // The band's sheen runs from the top left to the bottom right.
    let bandRect = outer.insetBy(dx: border, dy: border)
    let stops: [(CGFloat, UInt32)] = [(0, 0x6e4a22), (0.22, 0xb88d5a), (0.45, 0x8a6236), (0.68, 0xc29a66), (1, 0x6a4520)]
    let gradient = CGGradient(
        colorsSpace: CGColorSpace(name: CGColorSpace.sRGB)!,
        colors: stops.map { rgb($0.1) } as CFArray, locations: stops.map(\.0))!
    ctx.saveGState()
    ctx.clip(to: bandRect)
    ctx.drawLinearGradient(
        gradient, start: CGPoint(x: bandRect.minX, y: bandRect.maxY), end: CGPoint(x: bandRect.maxX, y: bandRect.minY),
        options: [])
    ctx.restoreGState()
    // The band's inner slope, next to the painting: dark on the top and left.
    ctx.setFillColor(rgb(0xebcda0))
    ctx.fill(inner.insetBy(dx: -scale, dy: -scale))
    ctx.setFillColor(rgb(0x28160a))
    ctx.fill(CGRect(x: inner.minX - scale, y: inner.minY, width: inner.width, height: inner.height + scale))

    ctx.interpolationQuality = .high
    ctx.draw(image, in: inner)

    let slack = rem
    let textRect = CGRect(x: (pixels.width - textWidth) / 2, y: bottom - slack, width: textWidth, height: textHeight + slack)
    CTFrameDraw(CTFramesetterCreateFrame(framesetter, CFRange(), CGPath(rect: textRect, transform: nil), nil), ctx)
    return ctx.makeImage()
}

func layout(_ quote: QuotePayload.Quote?, size: CGFloat, width: CGFloat) -> (CTFramesetter, CGFloat) {
    let text = NSMutableAttributedString()
    if let quote {
        // HTML collapses whitespace, so the display page shows one paragraph.
        let body = quote.text.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        text.append(paragraph(body, font: font(italic: false, size: size), color: ink))
        text.append(paragraph("\n— \(quote.author)", font: font(italic: true, size: size * 0.75), color: ink,
                              spaceBefore: size * 0.625))
    } else {
        text.append(paragraph("No quote yet.", font: font(italic: true, size: size * 0.75), color: muted))
    }
    let framesetter = CTFramesetterCreateWithAttributedString(text)
    let fit = CTFramesetterSuggestFrameSizeWithConstraints(
        framesetter, CFRange(), nil, CGSize(width: width, height: .greatestFiniteMagnitude), nil)
    return (framesetter, fit.height.rounded(.up))
}

func paragraph(_ string: String, font: CTFont, color: CGColor, spaceBefore: CGFloat = 0) -> NSAttributedString {
    let style = NSMutableParagraphStyle()
    style.alignment = .center
    style.paragraphSpacingBefore = spaceBefore
    return NSAttributedString(string: string, attributes: [
        NSAttributedString.Key(kCTFontAttributeName as String): font,
        NSAttributedString.Key(kCTForegroundColorAttributeName as String): color,
        .paragraphStyle: style,
    ])
}

/// Baskervville, as on the display page, from the fonts install.sh copies;
/// macOS's own Baskerville if they are missing.
func font(italic: Bool, size: CGFloat) -> CTFont {
    let file = fontsDir.appendingPathComponent(italic ? "baskervville_italic.ttf" : "baskervville.ttf")
    if let descriptors = CTFontManagerCreateFontDescriptorsFromURL(file as CFURL) as? [CTFontDescriptor],
       let descriptor = descriptors.first {
        return CTFontCreateWithFontDescriptor(descriptor, size, nil)
    }
    return CTFontCreateWithName((italic ? "Baskerville-Italic" : "Baskerville") as CFString, size, nil)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// A hash that, unlike `Hasher`, is the same from one run to the next.
func fnv1a(_ string: String) -> UInt64 {
    string.utf8.reduce(0xcbf2_9ce4_8422_2325) { ($0 ^ UInt64($1)) &* 0x0100_0000_01b3 }
}

func log(_ message: String) {
    FileHandle.standardError.write("\(Date()) \(message)\n".data(using: .utf8)!)
}

// ── Main ──────────────────────────────────────────────────────────────────────

try? FileManager.default.createDirectory(at: imagesDir, withIntermediateDirectories: true)

let app = NSApplication.shared
app.setActivationPolicy(.prohibited)
let agent = Agent()

if CommandLine.arguments.contains("--once") {
    agent.fetchAll { exit(0) }
} else {
    agent.start()
}
app.run()
