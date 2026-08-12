// Fixture docs mention docOnlySwift and should not become code.
import Foundation

let stringMention = "stringOnlySwift()"
let multilineMention = """
multilineOnlySwift()
"""
/* blockOnlySwift() */

protocol GoldenRenderable {
    func render(_ value: String) -> String
}

struct GoldenWidget: GoldenRenderable {
    func render(_ value: String) -> String {
        formatWidget(value)
    }
}

actor GoldenWorker {}

enum GoldenState {
    case ready
    case spent
}

func makeWidget(_ value: String) -> GoldenWidget {
    GoldenWidget()
}

func formatWidget(_ value: String) -> String {
    value.trimmingCharacters(in: .whitespaces)
}
