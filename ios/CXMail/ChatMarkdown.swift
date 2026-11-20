import SwiftUI

enum ChatMarkdownBlock: Equatable {
    case paragraph(String), heading(String), list(String, String), quote(String), code(String, String)

    static func parse(_ text: String) -> [Self] {
        let fence = "```"
        var blocks: [Self] = []
        var paragraph: [String] = []
        var code: [String]? = nil
        var language = ""
        func flush() {
            if !paragraph.isEmpty { blocks.append(.paragraph(paragraph.joined(separator: "\n"))); paragraph = [] }
        }
        for line in text.components(separatedBy: "\n") {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.hasPrefix(fence) {
                flush()
                if let lines = code { blocks.append(.code(language, lines.joined(separator: "\n"))); code = nil }
                else { code = []; language = String(trimmed.dropFirst(3)).trimmingCharacters(in: .whitespaces) }
            } else if code != nil { code?.append(line) }
            else if trimmed.isEmpty { flush() }
            else if trimmed.hasPrefix("#"), let space = trimmed.firstIndex(of: " "), trimmed[..<space].allSatisfy({ $0 == "#" }) {
                flush(); blocks.append(.heading(String(trimmed[trimmed.index(after: space)...])))
            } else if trimmed.hasPrefix("> ") {
                flush(); blocks.append(.quote(String(trimmed.dropFirst(2))))
            } else if ["- ", "* ", "+ "].contains(where: { trimmed.hasPrefix($0) }) {
                flush(); blocks.append(.list("•", String(trimmed.dropFirst(2))))
            } else if let dot = trimmed.firstIndex(of: "."), trimmed[..<dot].allSatisfy(\.isNumber), dot != trimmed.startIndex,
                      trimmed[trimmed.index(after: dot)...].hasPrefix(" ") {
                flush(); blocks.append(.list(String(trimmed[...dot]), String(trimmed[trimmed.index(after: dot)...]).trimmingCharacters(in: .whitespaces)))
            } else { paragraph.append(line) }
        }
        flush()
        if let code { blocks.append(.code(language, code.joined(separator: "\n"))) }
        return blocks
    }
}

struct ChatMarkdown: View {
    let text: String
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            ForEach(Array(ChatMarkdownBlock.parse(text).enumerated()), id: \.offset) { _, block in
                switch block {
                case .paragraph(let text): inline(text).font(.body).textSelection(.enabled)
                case .heading(let text): inline(text).font(.headline).textSelection(.enabled)
                case .list(let marker, let text):
                    HStack(alignment: .top, spacing: 8) {
                        Text(marker).monospacedDigit()
                        inline(text).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
                    }
                case .quote(let text):
                    HStack(spacing: 12) {
                        Rectangle().fill(Color(uiColor: .separator)).frame(width: 2)
                        inline(text).foregroundStyle(.secondary).textSelection(.enabled)
                    }.fixedSize(horizontal: false, vertical: true)
                case .code(let language, let code):
                    VStack(alignment: .leading, spacing: 0) {
                        HStack {
                            Text(language.isEmpty ? "Code" : language).font(.caption).foregroundStyle(.secondary)
                            Spacer()
                            Button("Copy", systemImage: "doc.on.doc") { UIPasteboard.general.string = code }.font(.caption).frame(minHeight: 44)
                        }.padding(.horizontal, 12)
                        ScrollView(.horizontal) { Text(code).font(.footnote.monospaced()).textSelection(.enabled).padding(12) }
                    }
                    .background(Color.mailSurface, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .environment(\.openURL, OpenURLAction { url in
            ["https", "http", "mailto"].contains(url.scheme?.lowercased() ?? "") ? .systemAction : .discarded
        })
    }
    private func inline(_ text: String) -> Text {
        Text((try? AttributedString(markdown: text, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(text))
    }
}
