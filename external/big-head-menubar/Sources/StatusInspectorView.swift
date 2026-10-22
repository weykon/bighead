import AppKit

final class StatusInspectorView: NSScrollView {
    private let stack = NSStackView()
    private let document = FlippedDocumentView()
    var onAction: ((StatusItem, String) -> Void)?

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        drawsBackground = false
        hasVerticalScroller = true
        borderType = .noBorder

        stack.orientation = .vertical
        stack.alignment = .width
        stack.spacing = 10
        stack.edgeInsets = NSEdgeInsets(top: 12, left: 12, bottom: 14, right: 12)
        stack.translatesAutoresizingMaskIntoConstraints = false

        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(stack)
        documentView = document

        NSLayoutConstraint.activate([
            document.widthAnchor.constraint(equalTo: contentView.widthAnchor),
            document.heightAnchor.constraint(greaterThanOrEqualTo: contentView.heightAnchor),
            stack.leadingAnchor.constraint(equalTo: document.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: document.trailingAnchor),
            stack.topAnchor.constraint(equalTo: document.topAnchor),
            stack.bottomAnchor.constraint(equalTo: document.bottomAnchor)
        ])
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    func render(item: StatusItem?, messages: [ChatMessage], documents: [StatusDocument]) {
        stack.arrangedSubviews.forEach { view in
            stack.removeArrangedSubview(view)
            view.removeFromSuperview()
        }

        guard let item else {
            stack.addArrangedSubview(StatusInspectorEmptyView())
            return
        }

        stack.addArrangedSubview(StatusInspectorHeroView(item: item) { [weak self] item, action in
            self?.onAction?(item, action)
        })
        stack.addArrangedSubview(StatusInspectorFactsView(item: item))

        if let description = item.description, !description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            stack.addArrangedSubview(StatusInspectorTextSection(title: "Description", text: description))
        }

        let relatedMessages = messages
            .reversed()
            .filter { message in
                message.text.localizedCaseInsensitiveContains(item.id)
                    || message.text.localizedCaseInsensitiveContains(item.title)
            }
            .prefix(3)
        if !relatedMessages.isEmpty {
            stack.addArrangedSubview(StatusInspectorTextSection(
                title: "Related Conversation",
                text: relatedMessages.map { message in
                    "\(message.role.rawValue): \(message.text)"
                }.joined(separator: "\n\n")
            ))
        }

        let relatedDocuments = documents
            .filter { document in
                document.id.localizedCaseInsensitiveContains(item.id)
                    || document.title.localizedCaseInsensitiveContains(item.title)
                    || document.text.localizedCaseInsensitiveContains(item.id)
                    || document.text.localizedCaseInsensitiveContains(item.title)
                    || document.references.contains { reference in
                        reference.id.localizedCaseInsensitiveContains(item.id)
                            || reference.title.localizedCaseInsensitiveContains(item.title)
                    }
            }
            .prefix(3)
        if !relatedDocuments.isEmpty {
            stack.addArrangedSubview(StatusInspectorTextSection(
                title: "Related Notes",
                text: relatedDocuments.map { document in
                    "\(document.title)\n\(document.subtitle ?? document.kind ?? document.id)"
                }.joined(separator: "\n\n")
            ))
        }
    }
}

private final class StatusInspectorEmptyView: NSView {
    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        translatesAutoresizingMaskIntoConstraints = false
        heightAnchor.constraint(greaterThanOrEqualToConstant: 280).isActive = true
        build()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    private func build() {
        let title = NSTextField(labelWithString: "Select a status item")
        title.font = NSFont.systemFont(ofSize: 16, weight: .semibold)
        title.textColor = .labelColor
        title.alignment = .center

        let subtitle = NSTextField(labelWithString: "Click a task, choice, checklist row, or progress row to inspect its protocol fields and send an action back to Big Head.")
        subtitle.font = NSFont.systemFont(ofSize: 12.5, weight: .regular)
        subtitle.textColor = .secondaryLabelColor
        subtitle.alignment = .center
        subtitle.lineBreakMode = .byWordWrapping
        subtitle.maximumNumberOfLines = 0

        let stack = NSStackView(views: [title, subtitle])
        stack.orientation = .vertical
        stack.alignment = .centerX
        stack.spacing = 7
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)

        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 34),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -34),
            stack.centerYAnchor.constraint(equalTo: centerYAnchor)
        ])
    }
}

private final class StatusInspectorHeroView: NSView {
    private let item: StatusItem
    private let onAction: (StatusItem, String) -> Void

    init(item: StatusItem, onAction: @escaping (StatusItem, String) -> Void) {
        self.item = item
        self.onAction = onAction
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        build()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    private func build() {
        DesktopTheme.applyCardChrome(to: self, radius: 8)
        layer?.backgroundColor = DesktopTheme.raisedBackground.cgColor

        let caption = DesktopTheme.makeCaption("Status Inspector")
        let title = NSTextField(labelWithString: item.title)
        title.font = NSFont.systemFont(ofSize: 22, weight: .bold)
        title.textColor = .labelColor
        title.lineBreakMode = .byWordWrapping
        title.maximumNumberOfLines = 3

        let pill = StatusPillView(text: stateLabel(item.state), color: stateColor(item.state))
        let openButton = NSButton(title: "Open", target: self, action: #selector(open))
        DesktopTheme.styleToolbarButton(openButton)

        let controls = NSStackView(views: [pill, openButton])
        controls.orientation = .horizontal
        controls.alignment = .centerY
        controls.spacing = 8

        let titleStack = NSStackView(views: [caption, title])
        titleStack.orientation = .vertical
        titleStack.alignment = .leading
        titleStack.spacing = 7

        addSubview(titleStack)
        addSubview(controls)
        titleStack.translatesAutoresizingMaskIntoConstraints = false
        controls.translatesAutoresizingMaskIntoConstraints = false

        var bottomAnchorTarget = titleStack.bottomAnchor
        if item.control == .checkbox {
            let toggleButton = NSButton(
                title: (item.checked == true || item.state == .done) ? "Checked" : "Toggle",
                target: self,
                action: #selector(toggle)
            )
            DesktopTheme.styleToolbarButton(toggleButton)
            controls.addArrangedSubview(toggleButton)
        }

        if let progressValue = item.progress {
            let progress = NSProgressIndicator()
            progress.isIndeterminate = false
            progress.minValue = 0
            progress.maxValue = 1
            progress.doubleValue = Double(min(max(progressValue, 0), 1))
            progress.translatesAutoresizingMaskIntoConstraints = false

            let label = NSTextField(labelWithString: "\(Int(min(max(progressValue, 0), 1) * 100))% complete")
            label.font = NSFont.monospacedDigitSystemFont(ofSize: 11.5, weight: .semibold)
            label.textColor = .secondaryLabelColor
            label.translatesAutoresizingMaskIntoConstraints = false

            addSubview(progress)
            addSubview(label)
            bottomAnchorTarget = label.bottomAnchor

            NSLayoutConstraint.activate([
                progress.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 18),
                progress.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -18),
                progress.topAnchor.constraint(equalTo: titleStack.bottomAnchor, constant: 18),
                label.leadingAnchor.constraint(equalTo: progress.leadingAnchor),
                label.topAnchor.constraint(equalTo: progress.bottomAnchor, constant: 7)
            ])
        }

        NSLayoutConstraint.activate([
            titleStack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 18),
            titleStack.trailingAnchor.constraint(lessThanOrEqualTo: controls.leadingAnchor, constant: -16),
            titleStack.topAnchor.constraint(equalTo: topAnchor, constant: 18),

            controls.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -18),
            controls.topAnchor.constraint(equalTo: topAnchor, constant: 18),

            bottomAnchor.constraint(equalTo: bottomAnchorTarget, constant: 18)
        ])
    }

    @objc private func open() {
        onAction(item, "open")
    }

    @objc private func toggle() {
        onAction(item, "toggle")
    }
}

private final class StatusInspectorFactsView: NSView {
    private let item: StatusItem

    init(item: StatusItem) {
        self.item = item
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        build()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    private func build() {
        DesktopTheme.applyCardChrome(to: self, radius: 8)
        layer?.backgroundColor = DesktopTheme.cardBackground.withAlphaComponent(0.96).cgColor

        let title = DesktopTheme.makeCaption("Protocol Fields")
        title.translatesAutoresizingMaskIntoConstraints = false

        let rows = NSStackView()
        rows.orientation = .vertical
        rows.alignment = .width
        rows.spacing = 8
        rows.translatesAutoresizingMaskIntoConstraints = false

        [
            ("id", item.id),
            ("state", item.state.rawValue),
            ("control", item.control?.rawValue ?? "item"),
            ("group", item.group ?? "none"),
            ("priority", item.priority.map(String.init) ?? "none"),
            ("updated", formattedTimestamp(item.updatedAt))
        ].forEach { key, value in
            rows.addArrangedSubview(StatusInspectorFactRow(key: key, value: value))
        }

        addSubview(title)
        addSubview(rows)

        NSLayoutConstraint.activate([
            title.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 16),
            title.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -16),
            title.topAnchor.constraint(equalTo: topAnchor, constant: 14),
            rows.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 16),
            rows.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -16),
            rows.topAnchor.constraint(equalTo: title.bottomAnchor, constant: 12),
            rows.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -14)
        ])
    }
}

private final class StatusInspectorFactRow: NSView {
    init(key: String, value: String) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        heightAnchor.constraint(greaterThanOrEqualToConstant: 24).isActive = true

        let keyLabel = NSTextField(labelWithString: key)
        keyLabel.font = NSFont.monospacedSystemFont(ofSize: 11.5, weight: .semibold)
        keyLabel.textColor = .secondaryLabelColor
        keyLabel.translatesAutoresizingMaskIntoConstraints = false

        let valueLabel = NSTextField(labelWithString: value)
        valueLabel.font = NSFont.systemFont(ofSize: 12.5, weight: .regular)
        valueLabel.textColor = .labelColor
        valueLabel.lineBreakMode = .byTruncatingMiddle
        valueLabel.translatesAutoresizingMaskIntoConstraints = false

        addSubview(keyLabel)
        addSubview(valueLabel)

        NSLayoutConstraint.activate([
            keyLabel.leadingAnchor.constraint(equalTo: leadingAnchor),
            keyLabel.centerYAnchor.constraint(equalTo: centerYAnchor),
            keyLabel.widthAnchor.constraint(equalToConstant: 78),

            valueLabel.leadingAnchor.constraint(equalTo: keyLabel.trailingAnchor, constant: 10),
            valueLabel.trailingAnchor.constraint(equalTo: trailingAnchor),
            valueLabel.centerYAnchor.constraint(equalTo: centerYAnchor)
        ])
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }
}

private final class StatusInspectorTextSection: NSView {
    init(title: String, text: String) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        build(title: title, text: text)
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    private func build(title: String, text: String) {
        DesktopTheme.applyCardChrome(to: self, radius: 8)
        layer?.backgroundColor = DesktopTheme.cardBackground.withAlphaComponent(0.96).cgColor

        let titleLabel = DesktopTheme.makeCaption(title)
        titleLabel.translatesAutoresizingMaskIntoConstraints = false

        let body = NSTextField(wrappingLabelWithString: text)
        body.font = NSFont.systemFont(ofSize: 13, weight: .regular)
        body.textColor = .labelColor
        body.lineBreakMode = .byWordWrapping
        body.maximumNumberOfLines = 12
        body.translatesAutoresizingMaskIntoConstraints = false

        addSubview(titleLabel)
        addSubview(body)

        NSLayoutConstraint.activate([
            titleLabel.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 16),
            titleLabel.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -16),
            titleLabel.topAnchor.constraint(equalTo: topAnchor, constant: 14),
            body.leadingAnchor.constraint(equalTo: titleLabel.leadingAnchor),
            body.trailingAnchor.constraint(equalTo: titleLabel.trailingAnchor),
            body.topAnchor.constraint(equalTo: titleLabel.bottomAnchor, constant: 10),
            body.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -14)
        ])
    }
}

private final class StatusPillView: NSView {
    init(text: String, color: NSColor) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        wantsLayer = true
        layer?.backgroundColor = color.withAlphaComponent(0.12).cgColor
        layer?.cornerRadius = 12

        let label = NSTextField(labelWithString: text)
        label.font = NSFont.monospacedSystemFont(ofSize: 10.5, weight: .bold)
        label.textColor = color
        label.alignment = .center
        label.translatesAutoresizingMaskIntoConstraints = false
        addSubview(label)

        NSLayoutConstraint.activate([
            widthAnchor.constraint(greaterThanOrEqualToConstant: 72),
            heightAnchor.constraint(equalToConstant: 24),
            label.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
            label.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            label.centerYAnchor.constraint(equalTo: centerYAnchor)
        ])
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }
}

private func formattedTimestamp(_ value: UInt64?) -> String {
    guard let value else { return "none" }
    return DateFormatter.localizedString(
        from: Date(timeIntervalSince1970: TimeInterval(value)),
        dateStyle: .short,
        timeStyle: .short
    )
}
