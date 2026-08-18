import SwiftUI

/// A standalone utility window: type a number in any base (using the
/// assembler's own `h`/`b`/`o`/`q` suffix convention, or bare decimal)
/// and see it in every other base at once.
struct NumberConverterView: View {
    @State private var inputText: String = "42"

    private var parsedValue: UInt32? {
        NumberConverter.parse(inputText)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Number Converter").font(.headline)

            TextField("e.g. 42, 2Ah, 101010b, 52o", text: $inputText)
                .textFieldStyle(.roundedBorder)
                .font(.system(.body, design: .monospaced))
                .accessibilityIdentifier("numberConverterInput")

            if let value = parsedValue {
                VStack(alignment: .leading, spacing: 6) {
                    row("Decimal", NumberConverter.decimalText(value), identifier: "converterDecimal")
                    row("Hex", NumberConverter.hexText(value), identifier: "converterHex")
                    row("Octal", NumberConverter.octalText(value), identifier: "converterOctal")
                    row("Binary", NumberConverter.binaryText(value), identifier: "converterBinary")
                }
                if value > 0xFFFF {
                    Text("Exceeds a 16-bit register's range.")
                        .font(.caption)
                        .foregroundColor(.orange)
                }
            } else {
                Text("Not a recognized number.")
                    .font(.caption)
                    .foregroundColor(.red)
                    .accessibilityIdentifier("converterError")
            }

            Spacer()
        }
        .padding()
        .frame(minWidth: 320, minHeight: 220)
    }

    private func row(_ label: String, _ value: String, identifier: String) -> some View {
        HStack {
            Text("\(label):").foregroundColor(.secondary).frame(width: 70, alignment: .leading)
            Text(value).font(.system(.body, design: .monospaced))
        }
        .accessibilityIdentifier(identifier)
    }
}

#Preview {
    NumberConverterView()
}
