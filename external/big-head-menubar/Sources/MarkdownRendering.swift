import AppKit

enum MarkdownRendering {
    static func render(_ markdown: String, role: ChatRole) -> NSAttributedString {
        let output = NSMutableAttributedString()
        var inCodeBlock = false

        for rawLine in markdown.components(separatedBy: .newlines) {
            let trimmed = rawLine.trimmingCharacters(in: .whitespaces)

            if trimmed.hasPrefix("```") {
                inCodeBlock.toggle()
                if !inCodeBlock {
                    appendBlankLine(to: output, spacing: 8)
                }
                continue
            }

            if inCodeBlock {
                append(rawLine.isEmpty ? " " : rawLine, to: output, attributes: codeBlockAttributes())
                continue
            }

            if trimmed.isEmpty {
                appendBlankLine(to: output, spacing: 6)
                continue
            }

            if trimmed == "---" || trimmed == "***" {
                append("----------------", to: output, attributes: dividerAttributes(role: role))
                continue
            }

            let parsed = parseBlock(rawLine, role: role)
            appendInline(parsed.text, to: output, attributes: parsed.attributes)
        }

        trimTrailingNewlines(output)
        return output
    }

    private static func parseBlock(_ rawLine: String, role: ChatRole) -> (text: String, attributes: [NSAttributedString.Key: Any]) {
        let line = rawLine.trimmingCharacters(in: .whitespaces)

        if line.hasPrefix("# ") {
            return (String(line.dropFirst(2)), baseAttributes(size: 20, weight: .bold, role: role, spacing: 8))
        }
        if line.hasPrefix("## ") {
            return (String(line.dropFirst(3)), baseAttributes(size: 17, weight: .semibold, role: role, spacing: 7))
        }
        if line.hasPrefix("### ") {
            return (String(line.dropFirst(4)), baseAttributes(size: 15, weight: .semibold, role: role, spacing: 6))
        }
        if line.hasPrefix("> ") {
            return ("| " + String(line.dropFirst(2)), quoteAttributes(role: role))
        }
        if let checkbox = parseCheckbox(line, role: role) {
            return checkbox
        }
        if line.hasPrefix("- ") || line.hasPrefix("* ") {
            return ("• " + String(line.dropFirst(2)), listAttributes(role: role))
        }
        if let ordered = parseOrderedList(line, role: role) {
            return ordered
        }
        if looksLikeTable(line) {
            return (line, tableAttributes(role: role))
        }

        return (rawLine, baseAttributes(size: 13.5, weight: .regular, role: role, spacing: 5))
    }

    private static func parseCheckbox(_ line: String, role: ChatRole) -> (text: String, attributes: [NSAttributedString.Key: Any])? {
        if line.hasPrefix("- [ ] ") {
            return ("☐ " + String(line.dropFirst(6)), listAttributes(role: role))
        }
        if line.hasPrefix("- [x] ") || line.hasPrefix("- [X] ") {
            return ("☑ " + String(line.dropFirst(6)), listAttributes(role: role))
        }
        return nil
    }

    private static func parseOrderedList(_ line: String, role: ChatRole) -> (text: String, attributes: [NSAttributedString.Key: Any])? {
        guard let range = line.range(of: #"^\d+\.\s+"#, options: .regularExpression) else {
            return nil
        }
        let prefix = String(line[range]).trimmingCharacters(in: .whitespaces)
        let body = String(line[range.upperBound...])
        return ("\(prefix) \(body)", listAttributes(role: role))
    }

    private static func looksLikeTable(_ line: String) -> Bool {
        line.contains("|") && line.split(separator: "|", omittingEmptySubsequences: false).count >= 3
    }

    private static func appendInline(_ text: String, to output: NSMutableAttributedString, attributes: [NSAttributedString.Key: Any]) {
        output.append(inlineRender(text, attributes: attributes))
        output.append(NSAttributedString(string: "\n", attributes: attributes))
    }

    private static func append(_ text: String, to output: NSMutableAttributedString, attributes: [NSAttributedString.Key: Any]) {
        output.append(NSAttributedString(string: text + "\n", attributes: attributes))
    }

    private static func appendBlankLine(to output: NSMutableAttributedString, spacing: CGFloat) {
        let style = NSMutableParagraphStyle()
        style.paragraphSpacing = spacing
        output.append(NSAttributedString(string: "\n", attributes: [.paragraphStyle: style]))
    }

    private static func trimTrailingNewlines(_ output: NSMutableAttributedString) {
        while output.string.hasSuffix("\n\n") || output.string.hasSuffix("\n") {
            output.deleteCharacters(in: NSRange(location: output.length - 1, length: 1))
        }
    }

    private static func baseAttributes(
        size: CGFloat,
        weight: NSFont.Weight,
        role: ChatRole,
        spacing: CGFloat
    ) -> [NSAttributedString.Key: Any] {
        [
            .font: NSFont.systemFont(ofSize: size, weight: weight),
            .foregroundColor: role == .error ? NSColor.systemRed : NSColor.labelColor,
            .paragraphStyle: paragraphStyle(lineSpacing: 3.5, paragraphSpacing: spacing)
        ]
    }

    private static func listAttributes(role: ChatRole) -> [NSAttributedString.Key: Any] {
        let style = NSMutableParagraphStyle()
        style.lineSpacing = 3
        style.paragraphSpacing = 4
        style.headIndent = 18
        style.firstLineHeadIndent = 0
        return [
            .font: NSFont.systemFont(ofSize: 13.5, weight: .regular),
            .foregroundColor: role == .error ? NSColor.systemRed : NSColor.labelColor,
            .paragraphStyle: style
        ]
    }

    private static func quoteAttributes(role: ChatRole) -> [NSAttributedString.Key: Any] {
        [
            .font: NSFont.systemFont(ofSize: 13.5, weight: .regular),
            .foregroundColor: role == .error ? NSColor.systemRed : NSColor.secondaryLabelColor,
            .paragraphStyle: paragraphStyle(lineSpacing: 3, paragraphSpacing: 5)
        ]
    }

    private static func tableAttributes(role: ChatRole) -> [NSAttributedString.Key: Any] {
        [
            .font: NSFont.monospacedSystemFont(ofSize: 12.5, weight: .regular),
            .foregroundColor: role == .error ? NSColor.systemRed : NSColor.labelColor,
            .backgroundColor: DesktopTheme.subtleFill.withAlphaComponent(0.45),
            .paragraphStyle: paragraphStyle(lineSpacing: 3, paragraphSpacing: 2)
        ]
    }

    private static func codeBlockAttributes() -> [NSAttributedString.Key: Any] {
        [
            .font: NSFont.monospacedSystemFont(ofSize: 12.5, weight: .regular),
            .foregroundColor: NSColor.labelColor,
            .backgroundColor: DesktopTheme.subtleFill,
            .paragraphStyle: paragraphStyle(lineSpacing: 2.5, paragraphSpacing: 1)
        ]
    }

    private static func dividerAttributes(role: ChatRole) -> [NSAttributedString.Key: Any] {
        [
            .font: NSFont.monospacedSystemFont(ofSize: 11, weight: .regular),
            .foregroundColor: NSColor.tertiaryLabelColor,
            .paragraphStyle: paragraphStyle(lineSpacing: 1, paragraphSpacing: 7)
        ]
    }

    private static func paragraphStyle(lineSpacing: CGFloat, paragraphSpacing: CGFloat) -> NSParagraphStyle {
        let style = NSMutableParagraphStyle()
        style.lineSpacing = lineSpacing
        style.paragraphSpacing = paragraphSpacing
        return style
    }

    private static func inlineRender(_ text: String, attributes: [NSAttributedString.Key: Any]) -> NSAttributedString {
        let result = NSMutableAttributedString()
        var remaining = text[...]

        while !remaining.isEmpty {
            let boldStart = remaining.range(of: "**")
            let codeStart = remaining.range(of: "`")

            let next = earliest(boldStart, codeStart)
            guard let start = next else {
                result.append(NSAttributedString(string: String(remaining), attributes: attributes))
                break
            }

            result.append(NSAttributedString(string: String(remaining[..<start.lowerBound]), attributes: attributes))

            if start == boldStart, let end = remaining[start.upperBound...].range(of: "**") {
                var bold = attributes
                bold[.font] = boldFont(from: attributes)
                result.append(NSAttributedString(string: String(remaining[start.upperBound..<end.lowerBound]), attributes: bold))
                remaining = remaining[end.upperBound...]
            } else if start == codeStart, let end = remaining[start.upperBound...].range(of: "`") {
                var code = attributes
                code[.font] = NSFont.monospacedSystemFont(ofSize: fontSize(from: attributes), weight: .regular)
                code[.backgroundColor] = DesktopTheme.subtleFill
                result.append(NSAttributedString(string: String(remaining[start.upperBound..<end.lowerBound]), attributes: code))
                remaining = remaining[end.upperBound...]
            } else {
                let token = start == boldStart ? "**" : "`"
                result.append(NSAttributedString(string: token, attributes: attributes))
                remaining = remaining[start.upperBound...]
            }
        }

        return result
    }

    private static func earliest(_ left: Range<Substring.Index>?, _ right: Range<Substring.Index>?) -> Range<Substring.Index>? {
        guard let left else { return right }
        guard let right else { return left }
        return left.lowerBound <= right.lowerBound ? left : right
    }

    private static func fontSize(from attributes: [NSAttributedString.Key: Any]) -> CGFloat {
        (attributes[.font] as? NSFont)?.pointSize ?? 13.5
    }

    private static func boldFont(from attributes: [NSAttributedString.Key: Any]) -> NSFont {
        NSFont.systemFont(ofSize: fontSize(from: attributes), weight: .semibold)
    }
}
