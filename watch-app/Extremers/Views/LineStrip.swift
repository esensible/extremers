import SwiftUI

/// With both ends set: time to the line, how that falls against the gun,
/// and where along the line the boat will cross (port end left).
struct LineStrip: View {
  let lineDate: Date
  let start: Date?
  /// 0-100, 100 the stbd end.
  let cross: UInt8

  var body: some View {
    VStack(spacing: 3) {
      TimelineView(.periodic(from: lineDate.addingTimeInterval(-86_400), by: 1)) { context in
        HStack(alignment: .firstTextBaseline, spacing: 4) {
          Text("LINE")
            .font(.system(size: 11, weight: .semibold))
            .foregroundStyle(Palette.caption)
          Text(Format.toLine(lineDate, now: context.date))
            .font(.system(size: 15, weight: .semibold).monospacedDigit())
          Spacer(minLength: 0)
          if let start {
            Text(Format.lineVersusGun(line: lineDate, start: start))
              .font(.system(size: 12))
              .foregroundStyle(Palette.caption)
          }
        }
      }
      CrossBar(cross: cross)
        .frame(height: 10)
    }
  }
}

/// A thin bar for the line with a marker where the boat will cross.
struct CrossBar: View {
  let cross: UInt8
  let marker: CGFloat = 8

  var body: some View {
    GeometryReader { geo in
      let fraction = CGFloat(min(cross, 100)) / 100
      let x = min(max(geo.size.width * fraction - marker / 2, 0), max(geo.size.width - marker, 0))
      ZStack(alignment: .leading) {
        Capsule()
          .fill(Color(white: 0.35))
          .frame(height: 3)
        Circle()
          .fill(Palette.accent)
          .frame(width: marker, height: marker)
          .offset(x: x)
      }
      .frame(width: geo.size.width, height: geo.size.height)
    }
  }
}
