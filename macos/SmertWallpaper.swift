// Desktop wallpaper showing smert's daily quote, from /api/quote.
//
// A background agent (launchd keeps it running, see install.sh) that draws
// the quote to a PNG for each screen and sets it as that screen's wallpaper.
// It listens to /api/events and refetches as soon as the quote is rerolled,
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
let apiURL = URL(string: serverURL + "/api/quote")!
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
let cacheURL = supportDir.appendingPathComponent("quote.json")
let imagesDir = supportDir.appendingPathComponent("wallpapers")
let fontsDir = supportDir.appendingPathComponent("fonts")

// The display page's palette.
let background = CGColor(srgbRed: 0x21 / 255, green: 0x22 / 255, blue: 0x2d / 255, alpha: 1)
let ink = CGColor(srgbRed: 0xff / 255, green: 0xf6 / 255, blue: 0xec / 255, alpha: 1)
let muted = CGColor(srgbRed: 0x7a / 255, green: 0x78 / 255, blue: 0x70 / 255, alpha: 1)

/// Quote size in points on a 1440-point-wide screen, as on the display page;
/// it scales with the screen's width.
let baseSize: CGFloat = 32
/// Text shrinks down to this fraction of its normal size to fit.
let minScale: CGFloat = 0.5

struct Payload: Decodable {
    struct Quote: Decodable {
        let text: String
        let author: String
    }

    let nextRollover: String
    let quote: Quote?
}

final class Agent: NSObject {
    private var fetching = false
    /// A change was pushed during a fetch, which may have missed it.
    private var fetchAgain = false
    private var rolloverTimer: Timer?

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

    // ── Data ──────────────────────────────────────────────────────────────────

    private func cached() -> Payload? {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return (try? Data(contentsOf: cacheURL)).flatMap { try? decoder.decode(Payload.self, from: $0) }
    }

    /// Fetch today's quote; on success cache it and redraw. On failure the
    /// cached quote stays up.
    @objc func refresh() {
        fetch(then: nil)
    }

    func fetch(then done: (() -> Void)?) {
        if fetching {
            fetchAgain = true
            return
        }
        fetching = true
        var request = URLRequest(url: apiURL, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 10)
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        URLSession.shared.dataTask(with: request) { data, response, error in
            DispatchQueue.main.async {
                self.fetching = false
                let ok = (response as? HTTPURLResponse)?.statusCode == 200
                if let data, ok {
                    try? data.write(to: cacheURL, options: .atomic)
                } else {
                    log("fetch failed: \(error?.localizedDescription ?? "HTTP \((response as? HTTPURLResponse)?.statusCode ?? 0)")")
                }
                self.apply()
                self.scheduleRollover()
                done?()
                if self.fetchAgain {
                    self.fetchAgain = false
                    self.refresh()
                }
            }
        }.resume()
    }

    /// Fetch again a minute after the next rollover.
    private func scheduleRollover() {
        rolloverTimer?.invalidate()
        guard let at = cached().flatMap({ ISO8601DateFormatter().date(from: $0.nextRollover) }) else { return }
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
        if line.dropFirst("data:".count).trimmingCharacters(in: .whitespaces) == "quote" {
            refresh()
        }
    }

    // ── Wallpaper ─────────────────────────────────────────────────────────────

    /// Draw the cached quote for every screen and set it as the wallpaper of
    /// the current Space, where it isn't already.
    @objc func apply() {
        guard let payload = cached() else { return }
        var current = Set<URL>()
        for screen in NSScreen.screens {
            guard let url = wallpaper(for: screen, payload) else { continue }
            current.insert(url)
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

    /// The wallpaper image for `screen`, drawn if it isn't on disk yet. Its
    /// name hashes what it shows: macOS ignores a new image at the same path.
    private func wallpaper(for screen: NSScreen, _ payload: Payload) -> URL? {
        let scale = screen.backingScaleFactor
        let pixels = CGSize(width: screen.frame.width * scale, height: screen.frame.height * scale)
        // Center the text between the menu bar and the Dock.
        let visible = screen.visibleFrame
        let box = CGRect(
            x: 0,
            y: (visible.minY - screen.frame.minY) * scale,
            width: pixels.width,
            height: visible.height * scale)
        let size = baseSize * screen.frame.width / 1440 * scale

        let text = payload.quote.map { "\($0.text)\u{0}\($0.author)" } ?? ""
        let key = fnv1a("\(text)|\(Int(pixels.width))x\(Int(pixels.height))|\(box)|\(size)")
        let url = imagesDir.appendingPathComponent(String(format: "quote-%016llx.png", key))
        if FileManager.default.fileExists(atPath: url.path) { return url }

        let image = render(payload.quote, pixels: pixels, box: box, size: size)
        guard let image, let dest = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)
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

/// The quote centered in `box`, with "— author" in italics below it, as on the
/// display page; shrunk to fit if it is long.
func render(_ quote: Payload.Quote?, pixels: CGSize, box: CGRect, size: CGFloat) -> CGImage? {
    guard let ctx = CGContext(
        data: nil, width: Int(pixels.width), height: Int(pixels.height), bitsPerComponent: 8,
        bytesPerRow: 0, space: CGColorSpace(name: CGColorSpace.sRGB)!,
        bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)
    else { return nil }
    ctx.setFillColor(background)
    ctx.fill(CGRect(origin: .zero, size: pixels))

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

func layout(_ quote: Payload.Quote?, size: CGFloat, width: CGFloat) -> (CTFramesetter, CGFloat) {
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
    agent.fetch { exit(0) }
} else {
    agent.start()
}
app.run()
