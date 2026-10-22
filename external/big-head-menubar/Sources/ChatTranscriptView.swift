import AppKit

@MainActor
final class ChatTranscriptView: NSScrollView {
    private let activityStrip = ChatActivityStripView()
    private let diagnosticsPanel = ChatDiagnosticLogView()
    private let stack = NSStackView()
    private var diagnosticsHeightConstraint: NSLayoutConstraint?
    private var localMessageIDs = Set<String>()
    private var diagnosticEvents: [String] = []
    private var diagnosticsVisible = false

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        drawsBackground = false
        hasVerticalScroller = true
        borderType = .noBorder

        stack.orientation = .vertical
        stack.alignment = .width
        stack.spacing = 12
        stack.edgeInsets = NSEdgeInsets(top: 10, left: 16, bottom: 18, right: 16)
        stack.translatesAutoresizingMaskIntoConstraints = false

        let document = FlippedDocumentView()
        document.translatesAutoresizingMaskIntoConstraints = false
        activityStrip.translatesAutoresizingMaskIntoConstraints = false
        diagnosticsPanel.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(activityStrip)
        document.addSubview(diagnosticsPanel)
        document.addSubview(stack)
        documentView = document
        diagnosticsPanel.isHidden = true
        diagnosticsHeightConstraint = diagnosticsPanel.heightAnchor.constraint(equalToConstant: 0)
        activityStrip.onToggleLog = { [weak self] in
            self?.toggleDiagnostics()
        }
        diagnosticsPanel.onClear = { [weak self] in
            self?.clearDiagnostics()
        }

        NSLayoutConstraint.activate([
            document.widthAnchor.constraint(equalTo: contentView.widthAnchor),
            document.heightAnchor.constraint(greaterThanOrEqualTo: contentView.heightAnchor),
            activityStrip.leadingAnchor.constraint(equalTo: document.leadingAnchor, constant: 16),
            activityStrip.trailingAnchor.constraint(equalTo: document.trailingAnchor, constant: -16),
            activityStrip.topAnchor.constraint(equalTo: document.topAnchor, constant: 12),
            diagnosticsPanel.leadingAnchor.constraint(equalTo: activityStrip.leadingAnchor),
            diagnosticsPanel.trailingAnchor.constraint(equalTo: activityStrip.trailingAnchor),
            diagnosticsPanel.topAnchor.constraint(equalTo: activityStrip.bottomAnchor, constant: 8),
            diagnosticsHeightConstraint!,
            stack.leadingAnchor.constraint(equalTo: document.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: document.trailingAnchor),
            stack.topAnchor.constraint(equalTo: diagnosticsPanel.bottomAnchor, constant: 10),
            stack.bottomAnchor.constraint(equalTo: document.bottomAnchor)
        ])

        render([])
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    func render(_ messages: [ChatMessage]) {
        let shouldFollowBottom = isNearBottom() || stack.arrangedSubviews.isEmpty
        stack.arrangedSubviews.forEach { view in
            stack.removeArrangedSubview(view)
            view.removeFromSuperview()
        }
        localMessageIDs.removeAll()

        if messages.isEmpty {
            addEmptyState()
        } else {
            for message in messages {
                addBubble(ChatMessageBubble(message: message))
            }
        }

        if shouldFollowBottom {
            scrollToBottom()
        }
    }

    func appendLocalMessage(role: ChatRole, text: String, id: String = "local-\(UUID().uuidString)") {
        removeLocalMessage(id: id)
        if role == .system {
            recordDiagnostic(text)
            return
        }
        if stack.arrangedSubviews.count == 1,
           stack.arrangedSubviews.first is ChatEmptyView {
            stack.arrangedSubviews.forEach { view in
                stack.removeArrangedSubview(view)
                view.removeFromSuperview()
            }
        }
        localMessageIDs.insert(id)
        addBubble(ChatMessageBubble(
            message: ChatMessage(id: id, role: role, text: text, createdAt: nil),
            compact: role == .system
        ))
        scrollToBottom()
    }

    func removeLocalMessage(id: String) {
        guard localMessageIDs.remove(id) != nil else { return }
        for view in stack.arrangedSubviews {
            guard let bubble = view as? ChatMessageBubble, bubble.messageID == id else { continue }
            stack.removeArrangedSubview(bubble)
            bubble.removeFromSuperview()
            break
        }
    }

    func replaceLocalMessage(id: String, role: ChatRole, text: String) {
        if role == .system, !localMessageIDs.contains(id) {
            activityStrip.setPending(text)
            return
        }
        removeLocalMessage(id: id)
        appendLocalMessage(role: role, text: text, id: id)
    }

    func appendSystemLog(_ line: String) {
        recordDiagnostic(line)
    }

    func showPendingStatus(_ text: String) {
        recordDiagnostic(text)
        activityStrip.setPending(text)
    }

    func clearPendingStatus() {
        activityStrip.clearPending()
    }

    private func recordDiagnostic(_ text: String) {
        diagnosticEvents.append(text)
        if diagnosticEvents.count > 80 {
            diagnosticEvents.removeFirst(diagnosticEvents.count - 80)
        }
        activityStrip.append(text)
        diagnosticsPanel.render(diagnosticEvents)
    }

    private func toggleDiagnostics() {
        diagnosticsVisible.toggle()
        diagnosticsPanel.isHidden = !diagnosticsVisible
        diagnosticsHeightConstraint?.constant = diagnosticsVisible ? 180 : 0
        activityStrip.setExpanded(diagnosticsVisible)
    }

    private func clearDiagnostics() {
        diagnosticEvents.removeAll()
        diagnosticsPanel.render(diagnosticEvents)
        activityStrip.clearEvents()
    }

    private func addEmptyState() {
        let empty = ChatEmptyView()
        stack.addArrangedSubview(empty)
        empty.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
    }

    private func addBubble(_ bubble: ChatMessageBubble) {
        stack.addArrangedSubview(bubble)
        bubble.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
    }

    private func scrollToBottom() {
        guard let documentView else { return }
        DispatchQueue.main.async {
            let visibleHeight = self.contentView.bounds.height
            let maxY = max(0, documentView.bounds.height - visibleHeight)
            documentView.scroll(NSPoint(x: 0, y: maxY))
        }
    }

    private func isNearBottom() -> Bool {
        guard let documentView else { return true }
        let visibleHeight = contentView.bounds.height
        let maxY = max(0, documentView.bounds.height - visibleHeight)
        return contentView.bounds.origin.y >= maxY - 48
    }
}

final class ChatMessageBubble: NSView {
    var messageID: String { message.id }
    private let message: ChatMessage
    private let compact: Bool

    init(message: ChatMessage, compact: Bool = false) {
        self.message = message
        self.compact = compact
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        build()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    private func build() {
        let label = NSTextField(labelWithString: roleTitle(message.role))
        label.font = NSFont.systemFont(ofSize: 11, weight: .semibold)
        label.textColor = roleColor(message.role)
        label.translatesAutoresizingMaskIntoConstraints = false

        let textView = NSTextField(labelWithAttributedString: MarkdownRendering.render(message.text, role: message.role))
        textView.isSelectable = true
        textView.lineBreakMode = .byWordWrapping
        textView.maximumNumberOfLines = 0
        textView.translatesAutoresizingMaskIntoConstraints = false

        let bubble = NSView()
        bubble.wantsLayer = true
        bubble.layer?.cornerRadius = 12
        bubble.layer?.backgroundColor = bubbleColor(message.role).cgColor
        bubble.layer?.borderWidth = message.role == .assistant ? 1 : 0
        bubble.layer?.borderColor = DesktopTheme.hairline.cgColor
        bubble.translatesAutoresizingMaskIntoConstraints = false
        bubble.addSubview(label)
        bubble.addSubview(textView)

        addSubview(bubble)

        NSLayoutConstraint.activate([
            bubble.topAnchor.constraint(equalTo: topAnchor),
            bubble.bottomAnchor.constraint(equalTo: bottomAnchor),
            bubble.widthAnchor.constraint(lessThanOrEqualTo: widthAnchor, multiplier: bubbleMaxWidthMultiplier(for: message.role)),
            bubble.widthAnchor.constraint(greaterThanOrEqualToConstant: minBubbleWidth(for: message.role)),

            label.leadingAnchor.constraint(equalTo: bubble.leadingAnchor, constant: compact ? 10 : 12),
            label.trailingAnchor.constraint(equalTo: bubble.trailingAnchor, constant: compact ? -10 : -12),
            label.topAnchor.constraint(equalTo: bubble.topAnchor, constant: compact ? 8 : 10),

            textView.leadingAnchor.constraint(equalTo: bubble.leadingAnchor, constant: compact ? 12 : 14),
            textView.trailingAnchor.constraint(equalTo: bubble.trailingAnchor, constant: compact ? -12 : -14),
            textView.topAnchor.constraint(equalTo: label.bottomAnchor, constant: 5),
            textView.bottomAnchor.constraint(equalTo: bubble.bottomAnchor, constant: compact ? -10 : -13),
            textView.heightAnchor.constraint(greaterThanOrEqualToConstant: compact ? 20 : 28)
        ])

        if message.role == .user {
            bubble.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10).isActive = true
            bubble.leadingAnchor.constraint(greaterThanOrEqualTo: leadingAnchor, constant: 70).isActive = true
        } else if message.role == .assistant || message.role == .error {
            bubble.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10).isActive = true
            bubble.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -18).isActive = true
        } else {
            bubble.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10).isActive = true
            bubble.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -18).isActive = true
        }
    }
}

final class ChatActivityStripView: NSView {
    private let statusDot = NSView()
    private let statusLabel = NSTextField(labelWithString: "No runtime events yet")
    private let logButton = NSButton(title: "Log", target: nil, action: nil)
    private let progress = NSProgressIndicator()
    private var lastEvents: [String] = []
    var onToggleLog: (() -> Void)?

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        translatesAutoresizingMaskIntoConstraints = false
        build()
        render()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    func append(_ text: String) {
        lastEvents.append(text)
        if lastEvents.count > 5 {
            lastEvents.removeFirst(lastEvents.count - 5)
        }
        render()
    }

    func setPending(_ text: String) {
        progress.isHidden = false
        progress.startAnimation(nil)
        statusDot.layer?.backgroundColor = NSColor.systemBlue.cgColor
        statusLabel.stringValue = text
    }

    func clearPending() {
        progress.stopAnimation(nil)
        progress.isHidden = true
        render()
    }

    func clearEvents() {
        lastEvents.removeAll()
        render()
    }

    func setExpanded(_ expanded: Bool) {
        logButton.title = expanded ? "Hide Log" : "Log"
    }

    private func build() {
        wantsLayer = true
        layer?.cornerRadius = 8
        layer?.backgroundColor = DesktopTheme.subtleFill.withAlphaComponent(0.42).cgColor
        layer?.borderWidth = 1
        layer?.borderColor = DesktopTheme.hairline.cgColor

        statusDot.wantsLayer = true
        statusDot.layer?.cornerRadius = 4
        statusDot.translatesAutoresizingMaskIntoConstraints = false

        statusLabel.font = NSFont.systemFont(ofSize: 12, weight: .medium)
        statusLabel.textColor = .secondaryLabelColor
        statusLabel.lineBreakMode = .byTruncatingMiddle
        statusLabel.translatesAutoresizingMaskIntoConstraints = false

        DesktopTheme.styleToolbarButton(logButton)
        logButton.font = NSFont.systemFont(ofSize: 11, weight: .medium)
        logButton.target = self
        logButton.action = #selector(toggleLog)
        logButton.translatesAutoresizingMaskIntoConstraints = false

        progress.style = .spinning
        progress.controlSize = .small
        progress.isIndeterminate = true
        progress.isDisplayedWhenStopped = false
        progress.isHidden = true
        progress.translatesAutoresizingMaskIntoConstraints = false

        addSubview(statusDot)
        addSubview(statusLabel)
        addSubview(logButton)
        addSubview(progress)

        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 34),

            statusDot.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            statusDot.centerYAnchor.constraint(equalTo: centerYAnchor),
            statusDot.widthAnchor.constraint(equalToConstant: 8),
            statusDot.heightAnchor.constraint(equalToConstant: 8),

            statusLabel.leadingAnchor.constraint(equalTo: statusDot.trailingAnchor, constant: 10),
            statusLabel.trailingAnchor.constraint(equalTo: logButton.leadingAnchor, constant: -10),
            statusLabel.centerYAnchor.constraint(equalTo: centerYAnchor),

            logButton.trailingAnchor.constraint(equalTo: progress.leadingAnchor, constant: -10),
            logButton.centerYAnchor.constraint(equalTo: centerYAnchor),
            logButton.widthAnchor.constraint(equalToConstant: 70),

            progress.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            progress.centerYAnchor.constraint(equalTo: centerYAnchor),
            progress.widthAnchor.constraint(equalToConstant: 16),
            progress.heightAnchor.constraint(equalToConstant: 16)
        ])
    }

    private func render() {
        progress.stopAnimation(nil)
        progress.isHidden = true
        statusDot.layer?.backgroundColor = lastEvents.isEmpty
            ? NSColor.tertiaryLabelColor.cgColor
            : NSColor.systemGreen.cgColor
        statusLabel.stringValue = lastEvents.last ?? "No runtime events yet"
    }

    @objc private func toggleLog() {
        onToggleLog?()
    }
}

final class ChatDiagnosticLogView: NSView {
    private let stack = NSStackView()
    var onClear: (() -> Void)?

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        translatesAutoresizingMaskIntoConstraints = false
        build()
        render([])
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    func render(_ events: [String]) {
        stack.arrangedSubviews.forEach { view in
            stack.removeArrangedSubview(view)
            view.removeFromSuperview()
        }

        if events.isEmpty {
            stack.addArrangedSubview(makeRow("No diagnostic events recorded."))
            return
        }

        for event in events.suffix(12).reversed() {
            stack.addArrangedSubview(makeRow(event))
        }
    }

    private func build() {
        wantsLayer = true
        layer?.cornerRadius = 11
        layer?.backgroundColor = DesktopTheme.cardBackground.withAlphaComponent(0.92).cgColor
        layer?.borderWidth = 1
        layer?.borderColor = DesktopTheme.hairline.cgColor

        let title = DesktopTheme.makeCaption("Local Diagnostics")
        title.translatesAutoresizingMaskIntoConstraints = false

        let clearButton = NSButton(title: "Clear", target: self, action: #selector(clear))
        DesktopTheme.styleToolbarButton(clearButton)
        clearButton.font = NSFont.systemFont(ofSize: 11, weight: .medium)
        clearButton.translatesAutoresizingMaskIntoConstraints = false

        stack.orientation = .vertical
        stack.alignment = .width
        stack.spacing = 6
        stack.translatesAutoresizingMaskIntoConstraints = false

        addSubview(title)
        addSubview(clearButton)
        addSubview(stack)

        NSLayoutConstraint.activate([
            title.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 14),
            title.topAnchor.constraint(equalTo: topAnchor, constant: 12),
            title.trailingAnchor.constraint(lessThanOrEqualTo: clearButton.leadingAnchor, constant: -10),

            clearButton.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            clearButton.centerYAnchor.constraint(equalTo: title.centerYAnchor),
            clearButton.widthAnchor.constraint(equalToConstant: 64),

            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 14),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -14),
            stack.topAnchor.constraint(equalTo: title.bottomAnchor, constant: 10),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -12)
        ])
    }

    private func makeRow(_ text: String) -> NSTextField {
        let row = NSTextField(labelWithString: text)
        row.font = NSFont.monospacedSystemFont(ofSize: 11.5, weight: .regular)
        row.textColor = .secondaryLabelColor
        row.lineBreakMode = .byTruncatingMiddle
        row.translatesAutoresizingMaskIntoConstraints = false
        row.heightAnchor.constraint(greaterThanOrEqualToConstant: 18).isActive = true
        return row
    }

    @objc private func clear() {
        onClear?()
    }
}

final class ChatEmptyView: NSView {
    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        translatesAutoresizingMaskIntoConstraints = false
        heightAnchor.constraint(greaterThanOrEqualToConstant: 320).isActive = true
        build()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    private func build() {
        let title = NSTextField(labelWithString: "Ready for a spark")
        title.font = NSFont.systemFont(ofSize: 16, weight: .semibold)
        title.textColor = .labelColor
        title.alignment = .center

        let description = NSTextField(labelWithString: "Send a message below. Replies, waiting state, and runtime feedback will collect here.")
        description.font = NSFont.systemFont(ofSize: 12.5)
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
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 48),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -48),
            stack.centerYAnchor.constraint(equalTo: centerYAnchor)
        ])
    }
}

private func roleTitle(_ role: ChatRole) -> String {
    switch role {
    case .user: return "You"
    case .assistant: return "Big Head"
    case .system: return "System"
    case .error: return "Error"
    }
}

private func roleColor(_ role: ChatRole) -> NSColor {
    switch role {
    case .user: return .systemBlue
    case .assistant: return .labelColor
    case .system: return .secondaryLabelColor
    case .error: return .systemRed
    }
}

private func bubbleColor(_ role: ChatRole) -> NSColor {
    switch role {
    case .user: return DesktopTheme.accent.withAlphaComponent(0.13)
    case .assistant: return DesktopTheme.cardBackground
    case .system: return DesktopTheme.subtleFill.withAlphaComponent(0.72)
    case .error: return NSColor.systemRed.withAlphaComponent(0.10)
    }
}

private func minBubbleWidth(for role: ChatRole) -> CGFloat {
    switch role {
    case .system: return 240
    case .user: return 180
    case .assistant, .error: return 300
    }
}

private func bubbleMaxWidthMultiplier(for role: ChatRole) -> CGFloat {
    switch role {
    case .user: return 0.68
    case .system: return 0.62
    case .assistant, .error: return 0.84
    }
}
