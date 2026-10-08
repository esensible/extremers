import Foundation

/// A run of text in one style.
public struct TextRun: Hashable, Sendable {
    public var text: String
    public var style: TextStyle
    public init(_ text: String, _ style: TextStyle) {
        self.text = text
        self.style = style
    }
}

/// What a measurer reports for runs laid out at a width.
public struct TextLines: Sendable {
    /// Number of lines.
    public var count: Int
    /// Width of the widest line.
    public var widest: Double
    /// Each line as a UTF-16 range of the runs' concatenated text, when the
    /// measurer knows it (for drawing the same breaks).
    public var ranges: [Range<Int>]?

    public init(count: Int, widest: Double, ranges: [Range<Int>]? = nil) {
        self.count = count
        self.widest = widest
        self.ranges = ranges
    }
}

/// Text measurement for one font. The algorithm around it (shrink-to-fit,
/// line limits, line height) is shared by every host: `TextMeasure`.
public protocol TextMeasurer: AnyObject {
    /// Default line height / font size.
    var lineHeightRatio: Double { get }
    /// Lays the runs out with every font size (and explicit line height)
    /// multiplied by `scale`, wrapping at `maxWidth` (infinity = no wrapping;
    /// "\n" always breaks).
    func lines(_ runs: [TextRun], scale: Double, maxWidth: Double) -> TextLines
}

/// A measured text leaf (outermost `text`, `countdown`, `clock`).
public struct MeasuredText: Hashable, Sendable {
    public var width: Double
    public var height: Double
    /// Effective font size of the leaf after shrink-to-fit.
    public var fontSize: Double
    public var lines: Int
    /// fontSize / the leaf's own fontSize.
    public var scale: Double
    /// Height of one line.
    public var lineHeight: Double
    /// Lines past `numberOfLines` were cut (or a single line is wider than
    /// the box): draw the last line with a tail ellipsis.
    public var truncated: Bool
}

/// The text algorithm of the host interface (README: Text measurement),
/// a direct port of the browser host's `text-layout.js` so both sides make
/// the same decisions.
public enum TextMeasure {
    /// pt, the shrink-to-fit size grid.
    public static let fitStep = 0.5
    /// pt, tolerance for "fits".
    public static let eps = 0.001

    /// Line height of the runs at `scale`: the tallest run's.
    public static func lineHeight(_ runs: [TextRun], scale: Double, ratio: Double) -> Double {
        var h = 0.0
        for r in runs { h = max(h, (r.style.lineHeight ?? r.style.fontSize * ratio) * scale) }
        return h
    }

    /// Shrink-to-fit: the largest font size on the grid F, F - 0.5, F - 1, ...
    /// (strictly above F x minScale) whose single-line width fits maxWidth;
    /// if none fits, F x minScale. Returned as the scale (size / F).
    public static func fitScale(_ m: TextMeasurer, _ runs: [TextRun], fontSize F: Double,
                                minScale: Double, maxWidth: Double) -> Double {
        func width(_ size: Double) -> Double { m.lines(runs, scale: size / F, maxWidth: .infinity).widest }
        if !(maxWidth < .infinity) || minScale >= 1 { return 1 }
        if width(F) <= maxWidth + eps { return 1 }
        let fMin = F * minScale
        var k = 1.0
        while F - k * fitStep > fMin {
            let c = F - k * fitStep
            if width(c) <= maxWidth + eps { return c / F }
            k += 1
        }
        return minScale
    }

    /// Measures a leaf for Yoga. `fontSize`, `numberOfLines`, `minScale` are
    /// the leaf's own.
    public static func measure(_ m: TextMeasurer, _ runs: [TextRun], fontSize F: Double,
                               numberOfLines: Int, minScale: Double, maxWidth: Double) -> MeasuredText {
        if runs.isEmpty || runs.allSatisfy({ $0.text.isEmpty }) {
            return MeasuredText(width: 0, height: 0, fontSize: F, lines: 0, scale: 1, lineHeight: 0, truncated: false)
        }
        let shrink = minScale < 1
        let scale = shrink ? fitScale(m, runs, fontSize: F, minScale: minScale, maxWidth: maxWidth) : 1
        let single = shrink || numberOfLines == 1
        var lines: Int
        var widest: Double
        var truncated = false
        if single {
            widest = m.lines(runs, scale: scale, maxWidth: .infinity).widest
            lines = 1
            truncated = widest > maxWidth + eps
        } else {
            let l = m.lines(runs, scale: scale, maxWidth: maxWidth)
            lines = l.count
            widest = l.widest
            if numberOfLines > 0, lines > numberOfLines {
                lines = numberOfLines
                truncated = true
            }
        }
        let lh = lineHeight(runs, scale: scale, ratio: m.lineHeightRatio)
        return MeasuredText(width: min(widest, maxWidth), height: Double(lines) * lh, fontSize: F * scale,
                            lines: lines, scale: scale, lineHeight: lh, truncated: truncated)
    }
}

/// The stub measurer of the layout-parity fixtures: every character (code
/// point) is 0.6 x fontSize wide, line height ratio 1.25, greedy wrapping by
/// character (a character that doesn't fit starts a new line; a line holds
/// at least one).
public final class StubTextMeasurer: TextMeasurer {
    public init() {}
    public let lineHeightRatio = 1.25

    public func lines(_ runs: [TextRun], scale: Double, maxWidth: Double) -> TextLines {
        var lines = 1, line = 0.0, widest = 0.0, any = false
        for r in runs {
            let cw = 0.6 * r.style.fontSize * scale
            for ch in r.text.unicodeScalars {
                if ch == "\n" {
                    widest = max(widest, line)
                    lines += 1
                    line = 0
                    any = false
                    continue
                }
                if any, line + cw > maxWidth + TextMeasure.eps {
                    widest = max(widest, line)
                    lines += 1
                    line = 0
                }
                line += cw
                any = true
            }
        }
        return TextLines(count: lines, widest: max(widest, line))
    }
}

// MARK: - countdown / clock text (README: Countdown formats)

public enum TimeFormat {
    private static func pad2(_ n: Int) -> String { n < 10 ? "0\(n)" : String(n) }

    private static func hms(_ s: Int) -> String {
        s >= 3600 ? "\(s / 3600):\(pad2((s / 60) % 60)):\(pad2(s % 60))" : "\(s / 60):\(pad2(s % 60))"
    }

    /// `remainingMs`: integer ms until the target (negative once passed).
    public static func countdown(_ format: CountdownFormat, remainingMs: Int) -> String {
        let up = remainingMs <= 0 ? 0 : Int((Double(remainingMs + 999) / 1000).rounded(.down))
        switch format {
        case .elapsed: return hms(max(0, Int((Double(-remainingMs) / 1000).rounded(.down))))
        case .remaining: return hms(up)
        case .countdown: return "\(pad2(up / 60)):\(pad2(up % 60))"
        }
    }

    /// Local wall-clock time.
    public static func clock(_ format: ClockFormat, date: Date, calendar: Calendar = .current) -> String {
        let c = calendar.dateComponents([.hour, .minute, .second], from: date)
        let h = c.hour ?? 0, m = c.minute ?? 0, s = c.second ?? 0
        switch format {
        case .hourMinuteSecond24: return "\(pad2(h)):\(pad2(m)):\(pad2(s))"
        case .hourMinute12: return "\(h % 12 == 0 ? 12 : h % 12):\(pad2(m))"
        case .hourMinute24: return "\(pad2(h)):\(pad2(m))"
        }
    }

    /// JS `Math.round`.
    static func jsRound(_ x: Double) -> Int { Int((x + 0.5).rounded(.down)) }
}
