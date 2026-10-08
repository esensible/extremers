import SwiftUI

/// Confirm a start, bump, sync or finish. Cancels itself after 5 s, as the
/// Kindle's confirm does.
struct ConfirmSheet: View {
  let request: ConfirmRequest
  /// False while the link is down: the confirm cannot be sent.
  let enabled: Bool
  let onConfirm: () -> Void
  let onCancel: () -> Void

  var body: some View {
    ScrollView {
      VStack(spacing: 8) {
        Text(request.title)
          .font(.headline)
          .multilineTextAlignment(.center)
        Text(request.detail)
          .font(.footnote)
          .foregroundStyle(Palette.caption)
          .multilineTextAlignment(.center)
        Button(request.confirmLabel) { onConfirm() }
          .buttonStyle(PadButtonStyle.primary)
          .disabled(!enabled)
        Button("Cancel") { onCancel() }
          .buttonStyle(PadButtonStyle())
        Text("Cancels itself in 5 s")
          .font(.system(size: 11))
          .foregroundStyle(Palette.caption)
      }
    }
    .task {
      try? await Task.sleep(nanoseconds: 5_000_000_000)
      if !Task.isCancelled { onCancel() }
    }
  }
}
