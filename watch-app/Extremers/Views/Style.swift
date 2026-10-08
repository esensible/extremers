import SwiftUI

/// The shell's own screens' colours (the bundles have the same ones by
/// name, watch-js/extremers.schema.json).
enum Palette {
  /// #FF9F0A: the primary action.
  static let accent = Color(red: 1.0, green: 159.0 / 255.0, blue: 10.0 / 255.0)
  static let button = Color(white: 0.2)
  static let caption = Color(white: 0.6)
}

/// A filled, full-width watch button. Dims itself when disabled (the link
/// is down).
struct PadButtonStyle: ButtonStyle {
  var fill: Color = Palette.button
  var foreground: Color = .white
  var height: CGFloat = 36
  var fontSize: CGFloat = 17

  @Environment(\.isEnabled) var isEnabled

  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .font(.system(size: fontSize, weight: .semibold))
      .foregroundStyle(foreground)
      .lineLimit(1)
      .minimumScaleFactor(0.6)
      .frame(maxWidth: .infinity, minHeight: height)
      .background(fill, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
      .opacity(isEnabled ? (configuration.isPressed ? 0.6 : 1) : 0.35)
  }
}

extension PadButtonStyle {
  /// The orange primary action.
  static var primary: PadButtonStyle {
    PadButtonStyle(fill: Palette.accent, foreground: .black, height: 44, fontSize: 20)
  }
}

/// The Bluetooth rune, drawn: SF Symbols has no Bluetooth logo.
struct BluetoothGlyph: Shape {
  func path(in rect: CGRect) -> Path {
    func point(_ x: CGFloat, _ y: CGFloat) -> CGPoint {
      CGPoint(x: rect.minX + x * rect.width, y: rect.minY + y * rect.height)
    }
    var path = Path()
    path.move(to: point(0.15, 0.28))
    path.addLine(to: point(0.85, 0.72))
    path.addLine(to: point(0.5, 0.95))
    path.addLine(to: point(0.5, 0.05))
    path.addLine(to: point(0.85, 0.28))
    path.addLine(to: point(0.15, 0.72))
    return path
  }
}
