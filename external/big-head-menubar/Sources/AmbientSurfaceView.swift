import AppKit

@MainActor
final class AmbientSurfaceView: NSView {
    private let captionLabel = DesktopTheme.makeCaption("Ambient Dwelling")
    private let titleLabel = NSTextField(labelWithString: "Waiting for a spark")
    private let summaryLabel = NSTextField(labelWithString: "Big Head is ready to hold state, work, and conversation.")
    private let metricStack = NSStackView()

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        translatesAutoresizingMaskIntoConstraints = false
        build()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    func render(_ state: StatusState, runtimeText: String) {
        titleLabel.stringValue = state.title ?? "Big Head"
        summaryLabel.stringValue = bestSummary(from: state, runtimeText: runtimeText)

        metricStack.arrangedSubviews.forEach { view in
            metricStack.removeArrangedSubview(view)
            view.removeFromSuperview()
        }

        let visible = state.items.filter { $0.state != .hidden }
        metricStack.addArrangedSubview(makeMetric(
            label: "RUNNING",
            value: "\(visible.filter { $0.state == .running }.count)",
            color: DesktopTheme.accent
        ))
        metricStack.addArrangedSubview(makeMetric(
            label: "PENDING",
            value: "\(visible.filter { $0.state == .pending }.count)",
            color: DesktopTheme.warning
        ))
        metricStack.addArrangedSubview(makeMetric(
            label: "NOTES",
            value: "\(state.documents.count)",
            color: DesktopTheme.positive
        ))

        if let latest = state.messages.last(where: { $0.role == .assistant || $0.role == .system }) {
            metricStack.addArrangedSubview(makeMetric(
                label: "LATEST",
                value: latest.text.singleLinePreview(limit: 38),
                color: .secondaryLabelColor
            ))
        }
    }

    private func build() {
        DesktopTheme.applyPanelChrome(to: self, radius: 12)
        layer?.backgroundColor = DesktopTheme.raisedBackground.cgColor
        layer?.borderColor = DesktopTheme.strongHairline.cgColor

        titleLabel.font = NSFont.systemFont(ofSize: 20, weight: .bold)
        titleLabel.textColor = .labelColor
        titleLabel.lineBreakMode = .byTruncatingTail

        summaryLabel.font = NSFont.systemFont(ofSize: 13, weight: .medium)
        summaryLabel.textColor = .secondaryLabelColor
        summaryLabel.lineBreakMode = .byTruncatingTail

        let copyStack = NSStackView(views: [captionLabel, titleLabel, summaryLabel])
        copyStack.orientation = .vertical
        copyStack.alignment = .leading
        copyStack.spacing = 4
        copyStack.translatesAutoresizingMaskIntoConstraints = false

        metricStack.orientation = .horizontal
        metricStack.alignment = .centerY
        metricStack.spacing = 8
        metricStack.translatesAutoresizingMaskIntoConstraints = false

        addSubview(copyStack)
        addSubview(metricStack)

        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 98),
            copyStack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 18),
            copyStack.centerYAnchor.constraint(equalTo: centerYAnchor),
            copyStack.trailingAnchor.constraint(lessThanOrEqualTo: metricStack.leadingAnchor, constant: -18),

            metricStack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -16),
            metricStack.centerYAnchor.constraint(equalTo: centerYAnchor)
        ])
    }

    private func makeMetric(label: String, value: String, color: NSColor) -> NSView {
        let dot = NSView()
        dot.wantsLayer = true
        dot.layer?.backgroundColor = color.withAlphaComponent(0.82).cgColor
        dot.layer?.cornerRadius = 3
        dot.translatesAutoresizingMaskIntoConstraints = false
        dot.widthAnchor.constraint(equalToConstant: 6).isActive = true
        dot.heightAnchor.constraint(equalToConstant: 6).isActive = true

        let labelView = NSTextField(labelWithString: label)
        labelView.font = NSFont.monospacedSystemFont(ofSize: 10, weight: .semibold)
        labelView.textColor = .secondaryLabelColor

        let labelRow = NSStackView(views: [dot, labelView])
        labelRow.orientation = .horizontal
        labelRow.alignment = .centerY
        labelRow.spacing = 6

        let valueView = NSTextField(labelWithString: value)
        valueView.font = NSFont.systemFont(ofSize: 13, weight: .semibold)
        valueView.textColor = .labelColor
        valueView.lineBreakMode = .byTruncatingTail
        valueView.maximumNumberOfLines = 1

        let stack = NSStackView(views: [labelRow, valueView])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 3
        stack.edgeInsets = NSEdgeInsets(top: 9, left: 12, bottom: 9, right: 12)
        stack.wantsLayer = true
        stack.layer?.backgroundColor = DesktopTheme.subtleFill.withAlphaComponent(0.55).cgColor
        stack.layer?.cornerRadius = 8
        stack.layer?.borderWidth = 1
        stack.layer?.borderColor = DesktopTheme.hairline.cgColor
        stack.translatesAutoresizingMaskIntoConstraints = false
        stack.widthAnchor.constraint(lessThanOrEqualToConstant: label == "LATEST" ? 260 : 92).isActive = true
        return stack
    }

    private func bestSummary(from state: StatusState, runtimeText: String) -> String {
        if let subtitle = state.subtitle, !subtitle.isEmpty {
            return subtitle
        }
        if let summary = state.summary, !summary.isEmpty {
            return summary
        }
        return runtimeText
    }
}

private extension String {
    func singleLinePreview(limit: Int) -> String {
        let normalized = replacingOccurrences(of: "\n", with: " ")
            .replacingOccurrences(of: "\t", with: " ")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard normalized.count > limit else { return normalized }
        let end = normalized.index(normalized.startIndex, offsetBy: limit)
        return String(normalized[..<end]) + "..."
    }
}
