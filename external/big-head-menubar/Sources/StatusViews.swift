import AppKit

final class StatusMenuItemView: NSView {
    private let item: StatusItem
    private let onAction: (StatusItem, String) -> Void

    init(item: StatusItem, onAction: @escaping (StatusItem, String) -> Void) {
        self.item = item
        self.onAction = onAction
        super.init(frame: NSRect(x: 0, y: 0, width: 380, height: item.description == nil ? 46 : 66))
        build()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override func mouseDown(with event: NSEvent) {
        onAction(item, item.control == .checkbox ? "toggle" : "open")
    }

    override func resetCursorRects() {
        addCursorRect(bounds, cursor: .pointingHand)
    }

    private func build() {
        let checkbox = NSButton(checkboxWithTitle: "", target: self, action: #selector(toggle))
        checkbox.state = (item.checked == true || item.state == .done) ? .on : .off
        checkbox.isHidden = item.control != .checkbox
        checkbox.frame = NSRect(x: 10, y: bounds.height - 30, width: 18, height: 18)
        addSubview(checkbox)

        let leading = item.control == .checkbox ? 36.0 : 12.0
        let title = NSTextField(labelWithString: "\(stateLabel(item.state)) \(item.title)")
        title.font = NSFont.systemFont(ofSize: 13, weight: .medium)
        title.lineBreakMode = .byTruncatingTail
        title.frame = NSRect(x: leading, y: bounds.height - 25, width: 300, height: 18)
        addSubview(title)

        if let progress = item.progress {
            let progressView = NSProgressIndicator(frame: NSRect(x: leading, y: bounds.height - 42, width: 250, height: 8))
            progressView.isIndeterminate = false
            progressView.minValue = 0
            progressView.maxValue = 1
            progressView.doubleValue = Double(min(max(progress, 0), 1))
            addSubview(progressView)

            let pct = NSTextField(labelWithString: "\(Int(min(max(progress, 0), 1) * 100))%")
            pct.font = NSFont.monospacedDigitSystemFont(ofSize: 11, weight: .regular)
            pct.textColor = .secondaryLabelColor
            pct.alignment = .right
            pct.frame = NSRect(x: 310, y: bounds.height - 45, width: 46, height: 14)
            addSubview(pct)
        }

        if let description = item.description, !description.isEmpty {
            let desc = NSTextField(labelWithString: description)
            desc.font = NSFont.systemFont(ofSize: 11)
            desc.textColor = .secondaryLabelColor
            desc.lineBreakMode = .byTruncatingTail
            desc.frame = NSRect(x: leading, y: 8, width: 330, height: 16)
            addSubview(desc)
        }
    }

    @objc private func toggle() {
        onAction(item, "toggle")
    }
}

final class StatusBoardView: NSScrollView {
    private let stack = NSStackView()
    private let document = FlippedDocumentView()
    var onAction: ((StatusItem, String) -> Void)?

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        drawsBackground = false
        hasVerticalScroller = true
        borderType = .noBorder

        stack.orientation = .vertical
        stack.spacing = 9
        stack.edgeInsets = NSEdgeInsets(top: 10, left: 10, bottom: 12, right: 10)
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

    func render(_ state: StatusState) {
        stack.arrangedSubviews.forEach { view in
            stack.removeArrangedSubview(view)
            view.removeFromSuperview()
        }

        let visible = state.items.filter { $0.state != .hidden }
        if visible.isEmpty {
            stack.addArrangedSubview(StatusEmptyView())
            return
        }

        let choiceItems = visible.filter { $0.group == "choice" }
        let checklistItems = visible.filter { $0.group == "checklist" }
        let progressItems = visible.filter { $0.group == "progress" }
        let normalItems = visible.filter {
            $0.group != "choice" && $0.group != "checklist" && $0.group != "progress"
        }

        if !choiceItems.isEmpty {
            stack.addArrangedSubview(StatusChoiceGroupView(items: choiceItems) { [weak self] item, action in
                self?.onAction?(item, action)
            })
        }

        if !checklistItems.isEmpty {
            stack.addArrangedSubview(StatusGroupedItemView(
                title: "Checklist",
                subtitle: "Small steps Big Head is tracking",
                items: checklistItems,
                style: .checklist
            ) { [weak self] item, action in
                self?.onAction?(item, action)
            })
        }

        if !progressItems.isEmpty {
            stack.addArrangedSubview(StatusGroupedItemView(
                title: "Progress",
                subtitle: "Active work with visible completion",
                items: progressItems,
                style: .progress
            ) { [weak self] item, action in
                self?.onAction?(item, action)
            })
        }

        for item in normalItems {
            stack.addArrangedSubview(StatusRowView(item: item) { [weak self] item, action in
                self?.onAction?(item, action)
            })
        }
    }
}

final class StatusGroupedItemView: NSView {
    enum Style {
        case checklist
        case progress
    }

    private let titleText: String
    private let subtitleText: String
    private let items: [StatusItem]
    private let style: Style
    private let onAction: (StatusItem, String) -> Void

    init(
        title: String,
        subtitle: String,
        items: [StatusItem],
        style: Style,
        onAction: @escaping (StatusItem, String) -> Void
    ) {
        self.titleText = title
        self.subtitleText = subtitle
        self.items = items.sorted { ($0.priority ?? 0) > ($1.priority ?? 0) }
        self.style = style
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

        let title = NSTextField(labelWithString: titleText)
        title.font = NSFont.systemFont(ofSize: 13.5, weight: .semibold)
        title.textColor = .labelColor

        let subtitle = NSTextField(labelWithString: subtitleText)
        subtitle.font = NSFont.systemFont(ofSize: 12)
        subtitle.textColor = .secondaryLabelColor
        subtitle.lineBreakMode = .byTruncatingTail

        let count = NSTextField(labelWithString: summaryText())
        count.font = NSFont.monospacedDigitSystemFont(ofSize: 11, weight: .semibold)
        count.textColor = style == .checklist ? DesktopTheme.positive : DesktopTheme.accent
        count.alignment = .right
        count.translatesAutoresizingMaskIntoConstraints = false

        let headerCopy = NSStackView(views: [title, subtitle])
        headerCopy.orientation = .vertical
        headerCopy.alignment = .leading
        headerCopy.spacing = 3
        headerCopy.translatesAutoresizingMaskIntoConstraints = false

        let rows = NSStackView()
        rows.orientation = .vertical
        rows.alignment = .width
        rows.spacing = 6
        rows.translatesAutoresizingMaskIntoConstraints = false

        for item in items {
            rows.addArrangedSubview(StatusMiniRowView(item: item, style: style) { [weak self] item, action in
                self?.onAction(item, action)
            })
        }

        addSubview(headerCopy)
        addSubview(count)
        addSubview(rows)

        NSLayoutConstraint.activate([
            headerCopy.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 16),
            headerCopy.trailingAnchor.constraint(lessThanOrEqualTo: count.leadingAnchor, constant: -12),
            headerCopy.topAnchor.constraint(equalTo: topAnchor, constant: 15),

            count.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -16),
            count.centerYAnchor.constraint(equalTo: title.centerYAnchor),
            count.widthAnchor.constraint(equalToConstant: 76),

            rows.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
            rows.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            rows.topAnchor.constraint(equalTo: headerCopy.bottomAnchor, constant: 12),
            rows.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -10)
        ])
    }

    private func summaryText() -> String {
        switch style {
        case .checklist:
            let done = items.filter { $0.checked == true || $0.state == .done }.count
            return "\(done)/\(items.count)"
        case .progress:
            let values = items.compactMap(\.progress)
            guard !values.isEmpty else { return "\(items.count)" }
            let average = values.reduce(0, +) / Float(values.count)
            return "\(Int(min(max(average, 0), 1) * 100))%"
        }
    }
}

final class StatusMiniRowView: NSView {
    private let item: StatusItem
    private let style: StatusGroupedItemView.Style
    private let onAction: (StatusItem, String) -> Void

    init(
        item: StatusItem,
        style: StatusGroupedItemView.Style,
        onAction: @escaping (StatusItem, String) -> Void
    ) {
        self.item = item
        self.style = style
        self.onAction = onAction
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        heightAnchor.constraint(greaterThanOrEqualToConstant: style == .checklist ? 34 : 46).isActive = true
        build()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override func mouseDown(with event: NSEvent) {
        onAction(item, style == .checklist ? "toggle" : "open")
    }

    override func resetCursorRects() {
        addCursorRect(bounds, cursor: .pointingHand)
    }

    private func build() {
        wantsLayer = true
        layer?.backgroundColor = DesktopTheme.cardBackground.withAlphaComponent(0.88).cgColor
        layer?.cornerRadius = 7
        layer?.borderWidth = 1
        layer?.borderColor = DesktopTheme.hairline.cgColor

        let leadingIcon = NSTextField(labelWithString: iconText())
        leadingIcon.font = NSFont.monospacedSystemFont(ofSize: 12, weight: .semibold)
        leadingIcon.textColor = iconColor()
        leadingIcon.alignment = .center
        leadingIcon.translatesAutoresizingMaskIntoConstraints = false

        let title = NSTextField(labelWithString: item.title)
        title.font = NSFont.systemFont(ofSize: 12.5, weight: .medium)
        title.textColor = .labelColor
        title.lineBreakMode = .byTruncatingTail
        title.translatesAutoresizingMaskIntoConstraints = false

        addSubview(leadingIcon)
        addSubview(title)

        var constraints: [NSLayoutConstraint] = [
            leadingIcon.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
            leadingIcon.centerYAnchor.constraint(equalTo: centerYAnchor),
            leadingIcon.widthAnchor.constraint(equalToConstant: 20),

            title.leadingAnchor.constraint(equalTo: leadingIcon.trailingAnchor, constant: 8),
            title.topAnchor.constraint(equalTo: topAnchor, constant: 8)
        ]

        if style == .progress {
            let progress = NSProgressIndicator()
            progress.isIndeterminate = false
            progress.minValue = 0
            progress.maxValue = 1
            progress.doubleValue = Double(min(max(item.progress ?? 0, 0), 1))
            progress.translatesAutoresizingMaskIntoConstraints = false

            let pct = NSTextField(labelWithString: item.progress.map { "\(Int(min(max($0, 0), 1) * 100))%" } ?? "--")
            pct.font = NSFont.monospacedDigitSystemFont(ofSize: 10.5, weight: .regular)
            pct.textColor = .secondaryLabelColor
            pct.alignment = .right
            pct.translatesAutoresizingMaskIntoConstraints = false

            addSubview(progress)
            addSubview(pct)
            constraints.append(contentsOf: [
                title.trailingAnchor.constraint(lessThanOrEqualTo: pct.leadingAnchor, constant: -10),
                pct.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
                pct.centerYAnchor.constraint(equalTo: title.centerYAnchor),
                pct.widthAnchor.constraint(equalToConstant: 38),

                progress.leadingAnchor.constraint(equalTo: title.leadingAnchor),
                progress.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
                progress.topAnchor.constraint(equalTo: title.bottomAnchor, constant: 5)
            ])
        } else {
            constraints.append(title.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10))
        }

        NSLayoutConstraint.activate(constraints)
    }

    private func iconText() -> String {
        switch style {
        case .checklist:
            return (item.checked == true || item.state == .done) ? "✓" : "○"
        case .progress:
            return "▰"
        }
    }

    private func iconColor() -> NSColor {
        switch style {
        case .checklist:
            return (item.checked == true || item.state == .done) ? DesktopTheme.positive : .tertiaryLabelColor
        case .progress:
            return DesktopTheme.accent
        }
    }
}

final class StatusChoiceGroupView: NSView {
    private let items: [StatusItem]
    private let onAction: (StatusItem, String) -> Void

    init(items: [StatusItem], onAction: @escaping (StatusItem, String) -> Void) {
        self.items = items.sorted { ($0.priority ?? 0) > ($1.priority ?? 0) }
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
        layer?.backgroundColor = DesktopTheme.accent.withAlphaComponent(0.055).cgColor
        layer?.borderColor = DesktopTheme.accent.withAlphaComponent(0.20).cgColor

        let title = NSTextField(labelWithString: "Choose next")
        title.font = NSFont.systemFont(ofSize: 13.5, weight: .semibold)
        title.textColor = .labelColor

        let subtitle = NSTextField(labelWithString: "Big Head is waiting for a direction.")
        subtitle.font = NSFont.systemFont(ofSize: 12.5)
        subtitle.textColor = .secondaryLabelColor
        subtitle.lineBreakMode = .byTruncatingTail

        let header = NSStackView(views: [title, subtitle])
        header.orientation = .vertical
        header.alignment = .leading
        header.spacing = 4
        header.translatesAutoresizingMaskIntoConstraints = false

        let choices = NSStackView()
        choices.orientation = .vertical
        choices.alignment = .width
        choices.spacing = 8
        choices.translatesAutoresizingMaskIntoConstraints = false

        for item in items {
            choices.addArrangedSubview(StatusChoiceRowView(item: item) { [weak self] item in
                self?.onAction(item, "choose")
            })
        }

        addSubview(header)
        addSubview(choices)

        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: CGFloat(72 + max(1, items.count) * 58)),

            header.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 16),
            header.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -16),
            header.topAnchor.constraint(equalTo: topAnchor, constant: 16),

            choices.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            choices.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            choices.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 14),
            choices.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -12)
        ])
    }
}

final class StatusChoiceRowView: NSView {
    private let item: StatusItem
    private let onChoose: (StatusItem) -> Void

    init(item: StatusItem, onChoose: @escaping (StatusItem) -> Void) {
        self.item = item
        self.onChoose = onChoose
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        heightAnchor.constraint(greaterThanOrEqualToConstant: 52).isActive = true
        build()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override func mouseDown(with event: NSEvent) {
        onChoose(item)
    }

    override func resetCursorRects() {
        addCursorRect(bounds, cursor: .pointingHand)
    }

    private func build() {
        DesktopTheme.applyCardChrome(to: self, radius: 8)
        layer?.backgroundColor = DesktopTheme.cardBackground.withAlphaComponent(0.96).cgColor

        let title = NSTextField(labelWithString: item.title)
        title.font = NSFont.systemFont(ofSize: 13.5, weight: .semibold)
        title.textColor = .labelColor
        title.lineBreakMode = .byTruncatingTail
        title.translatesAutoresizingMaskIntoConstraints = false

        let description = NSTextField(labelWithString: item.description ?? "Choose this direction")
        description.font = NSFont.systemFont(ofSize: 12)
        description.textColor = .secondaryLabelColor
        description.lineBreakMode = .byTruncatingTail
        description.translatesAutoresizingMaskIntoConstraints = false

        let action = NSTextField(labelWithString: "CHOOSE")
        action.font = NSFont.monospacedSystemFont(ofSize: 10, weight: .bold)
        action.textColor = DesktopTheme.accent
        action.alignment = .right
        action.translatesAutoresizingMaskIntoConstraints = false

        addSubview(title)
        addSubview(description)
        addSubview(action)

        NSLayoutConstraint.activate([
            title.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 14),
            title.trailingAnchor.constraint(lessThanOrEqualTo: action.leadingAnchor, constant: -10),
            title.topAnchor.constraint(equalTo: topAnchor, constant: 9),

            description.leadingAnchor.constraint(equalTo: title.leadingAnchor),
            description.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -14),
            description.topAnchor.constraint(equalTo: title.bottomAnchor, constant: 4),

            action.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -14),
            action.centerYAnchor.constraint(equalTo: title.centerYAnchor),
            action.widthAnchor.constraint(equalToConstant: 70)
        ])
    }
}

final class StatusEmptyView: NSView {
    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        translatesAutoresizingMaskIntoConstraints = false
        heightAnchor.constraint(greaterThanOrEqualToConstant: 240).isActive = true
        build()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    private func build() {
        let title = NSTextField(labelWithString: "No active surface")
        title.font = NSFont.systemFont(ofSize: 14, weight: .semibold)
        title.textColor = .labelColor
        title.alignment = .center

        let description = NSTextField(labelWithString: "Agent-published tasks, progress, and choices will appear here.")
        description.font = NSFont.systemFont(ofSize: 12)
        description.textColor = .secondaryLabelColor
        description.alignment = .center
        description.lineBreakMode = .byWordWrapping
        description.maximumNumberOfLines = 0

        let stack = NSStackView(views: [title, description])
        stack.orientation = .vertical
        stack.alignment = .centerX
        stack.spacing = 6
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)

        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 18),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -18),
            stack.centerYAnchor.constraint(equalTo: centerYAnchor)
        ])
    }
}

final class StatusRowView: NSView {
    private let item: StatusItem
    private let onAction: (StatusItem, String) -> Void

    init(item: StatusItem, onAction: @escaping (StatusItem, String) -> Void) {
        self.item = item
        self.onAction = onAction
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        heightAnchor.constraint(greaterThanOrEqualToConstant: 86).isActive = true
        build()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override func mouseDown(with event: NSEvent) {
        onAction(item, item.control == .checkbox ? "toggle" : "open")
    }

    override func resetCursorRects() {
        addCursorRect(bounds, cursor: .pointingHand)
    }

    private func build() {
        DesktopTheme.applyCardChrome(to: self, radius: 8)
        layer?.backgroundColor = DesktopTheme.cardBackground.withAlphaComponent(0.96).cgColor

        let stateBar = NSView()
        stateBar.wantsLayer = true
        stateBar.layer?.backgroundColor = stateColor(item.state).withAlphaComponent(0.88).cgColor
        stateBar.layer?.cornerRadius = 2
        stateBar.translatesAutoresizingMaskIntoConstraints = false

        let title = NSTextField(labelWithString: item.title)
        title.font = NSFont.systemFont(ofSize: 14, weight: .semibold)
        title.lineBreakMode = .byTruncatingTail
        title.translatesAutoresizingMaskIntoConstraints = false

        let state = NSTextField(labelWithString: stateLabel(item.state))
        state.font = NSFont.monospacedSystemFont(ofSize: 10, weight: .semibold)
        state.textColor = stateColor(item.state)
        state.alignment = .right
        state.translatesAutoresizingMaskIntoConstraints = false

        let description = NSTextField(labelWithString: item.description ?? "")
        description.font = NSFont.systemFont(ofSize: 12.5)
        description.textColor = .secondaryLabelColor
        description.lineBreakMode = .byTruncatingTail
        description.translatesAutoresizingMaskIntoConstraints = false

        let progress = NSProgressIndicator()
        progress.isIndeterminate = false
        progress.minValue = 0
        progress.maxValue = 1
        progress.doubleValue = Double(min(max(item.progress ?? 0, 0), 1))
        progress.isHidden = item.progress == nil && item.control != .progress
        progress.translatesAutoresizingMaskIntoConstraints = false

        let progressLabel = NSTextField(labelWithString: item.progress.map { "\(Int(min(max($0, 0), 1) * 100))%" } ?? "")
        progressLabel.font = NSFont.monospacedDigitSystemFont(ofSize: 10.5, weight: .semibold)
        progressLabel.textColor = .secondaryLabelColor
        progressLabel.alignment = .right
        progressLabel.isHidden = item.progress == nil
        progressLabel.translatesAutoresizingMaskIntoConstraints = false

        let checkbox = NSButton(checkboxWithTitle: "", target: self, action: #selector(toggle))
        checkbox.state = (item.checked == true || item.state == .done) ? .on : .off
        checkbox.isHidden = item.control != .checkbox
        checkbox.translatesAutoresizingMaskIntoConstraints = false

        addSubview(stateBar)
        addSubview(checkbox)
        addSubview(title)
        addSubview(state)
        addSubview(description)
        addSubview(progress)
        addSubview(progressLabel)

        NSLayoutConstraint.activate([
            stateBar.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 0),
            stateBar.topAnchor.constraint(equalTo: topAnchor, constant: 12),
            stateBar.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -12),
            stateBar.widthAnchor.constraint(equalToConstant: 4),

            checkbox.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 14),
            checkbox.topAnchor.constraint(equalTo: topAnchor, constant: 18),

            title.leadingAnchor.constraint(equalTo: item.control == .checkbox ? checkbox.trailingAnchor : leadingAnchor, constant: 16),
            title.trailingAnchor.constraint(lessThanOrEqualTo: state.leadingAnchor, constant: -10),
            title.topAnchor.constraint(equalTo: topAnchor, constant: 16),

            state.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -16),
            state.widthAnchor.constraint(equalToConstant: 66),
            state.centerYAnchor.constraint(equalTo: title.centerYAnchor),

            description.leadingAnchor.constraint(equalTo: title.leadingAnchor),
            description.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -16),
            description.topAnchor.constraint(equalTo: title.bottomAnchor, constant: 6),

            progress.leadingAnchor.constraint(equalTo: title.leadingAnchor),
            progress.trailingAnchor.constraint(equalTo: progressLabel.leadingAnchor, constant: -10),
            progress.topAnchor.constraint(equalTo: description.bottomAnchor, constant: 10),

            progressLabel.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -16),
            progressLabel.centerYAnchor.constraint(equalTo: progress.centerYAnchor),
            progressLabel.widthAnchor.constraint(equalToConstant: 42)
        ])
    }

    @objc private func toggle() {
        onAction(item, "toggle")
    }
}

func stateLabel(_ state: ItemState) -> String {
    switch state {
    case .pending: return "PENDING"
    case .running: return "RUN"
    case .blocked: return "BLOCKED"
    case .done: return "DONE"
    case .failed: return "FAILED"
    case .hidden: return "HIDDEN"
    }
}

func stateColor(_ state: ItemState) -> NSColor {
    switch state {
    case .running: return DesktopTheme.accent
    case .blocked, .failed: return DesktopTheme.danger
    case .done: return DesktopTheme.positive
    case .pending: return .secondaryLabelColor
    case .hidden: return .tertiaryLabelColor
    }
}
