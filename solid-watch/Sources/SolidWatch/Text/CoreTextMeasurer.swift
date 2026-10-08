import CoreGraphics
import CoreText
import Foundation
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

/// Fonts: registration of bundled font files, and the CTFont for a style.
/// The same CTFont draws the text (via `Font(ctFont)`) that measured it.
public enum Fonts {
    /// The family used for all text; nil = the system font. Set it after
    /// registering the app's bundled font, before loading a bundle.
    public static var family: String? {
        didSet {
            cache.removeAllObjects()
            let f = family
            prewarmQueue.async {
                let t0 = ContinuousClock.now
                prewarm(family: f)
                prewarmTime = ContinuousClock.now - t0
            }
        }
    }

    /// Registers a font file for this process; returns the families in it.
    @discardableResult
    public static func register(_ url: URL) throws -> [String] {
        var err: Unmanaged<CFError>?
        if !CTFontManagerRegisterFontsForURL(url as CFURL, .process, &err), let e = err?.takeRetainedValue(),
           CFErrorGetCode(e) != CTFontManagerError.alreadyRegistered.rawValue {
            throw e as Error
        }
        guard let descs = CTFontManagerCreateFontDescriptorsFromURL(url as CFURL) as? [CTFontDescriptor] else { return [] }
        return descs.compactMap { CTFontDescriptorCopyAttribute($0, kCTFontFamilyNameAttribute) as? String }
    }

    private static let cache = NSCache<NSString, CTFont>()

    /// Core Text's first layout with a font is slow (on the watch, tens of ms:
    /// font tables and shaping caches load then), and it would land in the
    /// first bundle's mount. Lays out a sample line per weight off the main
    /// thread so that happens before a bundle needs it. `register` starts it.
    /// Thread-safe (touches no shared state of this type).
    public static func prewarm(family: String?, weights: [FontWeight] = [.normal, .bold]) {
        let sample = "0123456789:.,−+° abcdefghijklmnopqrstuvwxyz ABCDEFGHIJKLMNOPQRSTUVWXYZ"
        for w in weights {
            for tabular in [false, true] {
                let f = makeFont(family: family, weight: w.numeric, tabular: tabular, size: 16)
                let s = NSAttributedString(string: sample, attributes: [.init(kCTFontAttributeName as String): f])
                let line = CTLineCreateWithAttributedString(s)
                _ = CTLineGetTypographicBounds(line, nil, nil, nil)
            }
        }
    }

    private static let prewarmQueue = DispatchQueue(label: "solid-watch.fonts.prewarm", qos: .userInitiated)
    private static var prewarmTime: Duration = .zero

    /// Waits for the background prewarm started by setting `family`; returns
    /// how long the prewarm itself took.
    @discardableResult
    public static func waitForPrewarm() -> Duration {
        prewarmQueue.sync { prewarmTime }
    }

    static func weightTrait(_ w: Int) -> CGFloat {
        // CSS weight -> Core Text weight trait (UIFont.Weight values).
        let table: [(Int, CGFloat)] = [(100, -0.8), (200, -0.6), (300, -0.4), (400, 0), (500, 0.23),
                                       (600, 0.3), (700, 0.4), (800, 0.56), (900, 0.62)]
        if w <= 100 { return -0.8 }
        if w >= 900 { return 0.62 }
        for k in 1..<table.count where w <= table[k].0 {
            let (w0, t0) = table[k - 1], (w1, t1) = table[k]
            return t0 + (t1 - t0) * CGFloat(w - w0) / CGFloat(w1 - w0)
        }
        return 0
    }

    /// Time spent creating fonts (cache misses), and how many, for profiling.
    public private(set) static var createTime: Duration = .zero
    public private(set) static var createCount = 0

    /// The font for `size` points in `style`'s weight and figure spacing
    /// (cached; main thread).
    public static func ctFont(_ style: TextStyle, size: Double) -> CTFont {
        let w = style.fontWeight.numeric
        let key = "\(family ?? "-")|\(size)|\(w)|\(style.tabular)" as NSString
        if let f = cache.object(forKey: key) { return f }
        let t0 = ContinuousClock.now
        let f = makeFont(family: family, weight: w, tabular: style.tabular, size: size)
        createTime += ContinuousClock.now - t0
        createCount += 1
        cache.setObject(f, forKey: key)
        return f
    }

    private static func makeFont(family: String?, weight w: Int, tabular: Bool, size: Double) -> CTFont {
        var attrs: [CFString: Any] = [:]
        var base: CTFont
        if let family {
            // Variable fonts: weight on the wght axis; static families: the trait.
            let wght: UInt32 = 0x7767_6874
            attrs[kCTFontFamilyNameAttribute] = family
            attrs[kCTFontTraitsAttribute] = [kCTFontWeightTrait: weightTrait(w)]
            attrs[kCTFontVariationAttribute] = [wght: w]
            base = CTFontCreateWithFontDescriptor(CTFontDescriptorCreateWithAttributes(attrs as CFDictionary), size, nil)
        } else {
            #if canImport(UIKit)
            base = UIFont.systemFont(ofSize: size, weight: UIFont.Weight(weightTrait(w))) as CTFont
            #else
            base = NSFont.systemFont(ofSize: size, weight: NSFont.Weight(weightTrait(w))) as CTFont
            #endif
        }
        if tabular {
            let feature: [CFString: Any] = [
                kCTFontFeatureTypeIdentifierKey: kNumberSpacingType,
                kCTFontFeatureSelectorIdentifierKey: kMonospacedNumbersSelector,
            ]
            let d = CTFontDescriptorCreateWithAttributes([kCTFontFeatureSettingsAttribute: [feature]] as CFDictionary)
            base = CTFontCreateCopyWithAttributes(base, size, nil, d)
        }
        return base
    }
}

/// Measures with Core Text, using the fonts the views draw with. Wraps the
/// way Core Text's typesetter does (at word boundaries, by character for a
/// word wider than the line); the drawn lines use the same breaks.
public final class CoreTextMeasurer: TextMeasurer {
    public init() {}

    /// Results are cached by (runs: text + style, scale, width); the cache is
    /// dropped when it grows past `cacheLimit` entries.
    public var cacheLimit = 512
    public private(set) var cacheHits = 0
    public private(set) var cacheMisses = 0
    /// Time in Core Text on cache misses (incl. font creation), for profiling.
    public private(set) var typesetTime: Duration = .zero
    private struct Key: Hashable {
        let runs: [TextRun]
        let scale: Double
        let maxWidth: Double
    }
    private var cache: [Key: TextLines] = [:]

    public lazy var lineHeightRatio: Double = {
        // The schema font's ratio, exactly as the browser host uses it.
        if Fonts.family == CoreSchema.fontFamily { return CoreSchema.fontLineHeightRatio }
        let f = Fonts.ctFont(TextStyle(), size: 100)
        return Double(CTFontGetAscent(f) + CTFontGetDescent(f) + CTFontGetLeading(f)) / 100
    }()

    public func lines(_ runs: [TextRun], scale: Double, maxWidth: Double) -> TextLines {
        // Color and alignment don't affect measurement: keep them out of the key.
        let k = Key(runs: runs.map { r in
            var s = r.style
            s.color = .white
            s.textAlign = .left
            return TextRun(r.text, s)
        }, scale: scale, maxWidth: maxWidth)
        if let hit = cache[k] {
            cacheHits += 1
            return hit
        }
        cacheMisses += 1
        let t0 = ContinuousClock.now
        let v = typeset(runs, scale: scale, maxWidth: maxWidth)
        typesetTime += ContinuousClock.now - t0
        if cache.count >= cacheLimit { cache.removeAll(keepingCapacity: true) }
        cache[k] = v
        return v
    }

    static func attributed(_ runs: [TextRun], scale: Double) -> NSAttributedString {
        let s = NSMutableAttributedString()
        for r in runs {
            let attrs: [NSAttributedString.Key: Any] = [
                .init(kCTFontAttributeName as String): Fonts.ctFont(r.style, size: r.style.fontSize * scale),
            ]
            s.append(NSAttributedString(string: r.text, attributes: attrs))
        }
        return s
    }

    /// Uncached `lines`.
    public func typeset(_ runs: [TextRun], scale: Double, maxWidth: Double) -> TextLines {
        let s = CoreTextMeasurer.attributed(runs, scale: scale)
        let len = s.length
        let ts = CTTypesetterCreateWithAttributedString(s)
        let width = maxWidth.isFinite ? maxWidth + TextMeasure.eps : 1e7
        var start = 0
        var widest = 0.0
        var ranges: [Range<Int>] = []
        let str = s.string as NSString
        while start < len {
            var n = CTTypesetterSuggestLineBreak(ts, start, width)
            if n <= 0 { n = 1 }
            let line = CTTypesetterCreateLine(ts, CFRange(location: start, length: n))
            let w = CTLineGetTypographicBounds(line, nil, nil, nil) - CTLineGetTrailingWhitespaceWidth(line)
            widest = max(widest, w)
            // Drop the hard break itself from the drawn range.
            var end = start + n
            if end > start, str.character(at: end - 1) == 0x0A { end -= 1 }
            ranges.append(start..<end)
            start += n
        }
        if len == 0 || str.character(at: len - 1) == 0x0A { ranges.append(len..<len) }
        return TextLines(count: ranges.count, widest: widest, ranges: ranges)
    }
}
