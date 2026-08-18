import SwiftUI

/// A standalone reference window: every code 0-255 with its printable
/// character (where there is one), decimal, hex, and octal value.
struct AsciiTableView: View {
    private struct Row: Identifiable {
        let code: Int
        var id: Int { code }
        var characterText: String {
            switch code {
            case 32: return "space"
            case 0..<32, 127: return "·"
            default: return String(UnicodeScalar(code)!)
            }
        }
    }

    private static let rows = (0...255).map(Row.init)

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("ASCII Table").font(.headline).padding()
            header
            Divider()
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(Self.rows) { row in
                        HStack {
                            Text(row.characterText).frame(width: 60, alignment: .leading)
                            Text("\(row.code)").frame(width: 60, alignment: .leading)
                            Text(String(format: "%02Xh", row.code)).frame(width: 60, alignment: .leading)
                            Text(String(format: "%03o", row.code)).frame(width: 60, alignment: .leading)
                        }
                        .font(.system(.body, design: .monospaced))
                        .padding(.horizontal)
                        .padding(.vertical, 2)
                        .accessibilityIdentifier("asciiRow_\(row.code)")
                    }
                }
            }
        }
        .frame(minWidth: 320, minHeight: 300)
    }

    private var header: some View {
        HStack {
            Text("Char").frame(width: 60, alignment: .leading)
            Text("Dec").frame(width: 60, alignment: .leading)
            Text("Hex").frame(width: 60, alignment: .leading)
            Text("Oct").frame(width: 60, alignment: .leading)
        }
        .font(.caption.bold())
        .foregroundColor(.secondary)
        .padding(.horizontal)
    }
}

#Preview {
    AsciiTableView()
}
