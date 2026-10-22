import AppKit

@MainActor
final class DesktopWindowController: NSWindowController {
    var onSend: ((String, [DesktopAttachment]) -> Void)?
    var onStartRuntime: (() -> Void)?
    var onStopRuntime: (() -> Void)?
    var onRefresh: (() -> Void)?
    var onSettings: (() -> Void)?
    var onStatusAction: ((StatusItem, String) -> Void)?
    var onDocumentReferenceAction: ((StatusDocument, StatusDocumentReference, String) -> Void)?

    private let titleLabel = NSTextField(labelWithString: "Big Head")
    private let subtitleLabel = NSTextField(labelWithString: "Connecting...")
    private let runtimeLabel = NSTextField(labelWithString: "Runtime offline")
    private let configLabel = NSTextField(labelWithString: "")
    private let connectionDot = NSView()
    private let runtimeButton = NSButton(title: "Start Runtime", target: nil, action: nil)
    private let refreshButton = NSButton(title: "Refresh", target: nil, action: nil)
    private let settingsButton = NSButton(title: "Settings", target: nil, action: nil)
    private let modeControl = NSSegmentedControl(labels: ViewMode.allCases.map(\.title), trackingMode: .selectOne, target: nil, action: nil)
    private let contentStack = NSStackView()
    private let ambientSurface = AmbientSurfaceView()
    private let statusBoard = StatusBoardView()
    private let chatView = ChatTranscriptView()
    private let documentView = DocumentSurfaceView()
    private let inspectorView = StatusInspectorView()
    private let rightContentContainer = NSView()
    private let composer = CommandComposerView()
    private let workspaceMaxWidth: CGFloat = 1440
    private var configSlot: NSView?
    private var statusPanel: DesktopPanelView?
    private var chatPanel: DesktopPanelView?
    private var statusWidthConstraint: NSLayoutConstraint?
    private var runtimeStatus: RuntimeStatus = .offline
    private var pendingMessageID: String?
    private var currentMode: ViewMode = .dwelling
    private var hasDocuments = false
    private var latestState = StatusState()
    private var selectedStatusItem: StatusItem?
    private var keyMonitor: Any?

    enum RuntimeStatus {
        case offline
        case external
        case managed
        case starting
    }

    enum ViewMode: CaseIterable {
        case dwelling
        case work
        case chat
        case notes

        var title: String {
            switch self {
            case .dwelling: return "Dwelling"
            case .work: return "Work"
            case .chat: return "Chat"
            case .notes: return "Notes"
            }
        }
    }

    init() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1240, height: 790),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "Big Head"
        window.titlebarAppearsTransparent = true
        window.minSize = NSSize(width: 1040, height: 680)
        window.center()
        super.init(window: window)
        window.delegate = self
        build()
        installKeyMonitor()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    func render(state: StatusState) {
        latestState = state
        titleLabel.stringValue = state.title ?? "Big Head"
        subtitleLabel.stringValue = state.subtitle ?? state.summary ?? "Runtime status surface"
        reconcileSelectedItem(with: state)
        ambientSurface.render(state, runtimeText: runtimeLabel.stringValue)
        statusBoard.render(state)
        chatView.render(state.messages)
        documentView.render(state.documents)
        inspectorView.render(item: selectedStatusItem, messages: state.messages, documents: state.documents)
        hasDocuments = !state.documents.isEmpty
        updateRightContentVisibility()
    }

    func setConnected(_ connected: Bool) {
        connectionDot.layer?.backgroundColor = (connected ? NSColor.systemGreen : NSColor.systemRed).cgColor
        appendLog(connected ? "connected to bighead view server" : "disconnected from bighead view server")
        if !connected, runtimeStatus != .managed && runtimeStatus != .starting {
            setRuntimeStatus(.offline)
        }
    }

    func setConfigurationMissing(_ missing: Bool) {
        configLabel.stringValue = missing
            ? "MiniMax API key is missing. Open Settings before starting the runtime."
            : ""
        configLabel.isHidden = !missing
        configSlot?.isHidden = !missing
    }

    func setRuntimeStatus(_ status: RuntimeStatus) {
        runtimeStatus = status
        switch status {
        case .offline:
            runtimeButton.title = "Start Runtime"
            runtimeButton.isEnabled = true
            runtimeLabel.stringValue = "Runtime offline"
        case .external:
            runtimeButton.title = "External Runtime"
            runtimeButton.isEnabled = false
            runtimeLabel.stringValue = "Connected to an already running Big Head runtime"
        case .managed:
            runtimeButton.title = "Stop Runtime"
            runtimeButton.isEnabled = true
            runtimeLabel.stringValue = "Runtime managed by this app"
        case .starting:
            runtimeButton.title = "Starting..."
            runtimeButton.isEnabled = false
            runtimeLabel.stringValue = "Starting local Big Head runtime"
        }
    }

    func appendLog(_ line: String) {
        let timestamp = DateFormatter.localizedString(from: Date(), dateStyle: .none, timeStyle: .medium)
        chatView.appendSystemLog("[\(timestamp)] \(line)")
    }

    func beginWaitingForReply(userText: String, attachments: [DesktopAttachment] = []) {
        composer.setSendingEnabled(false, buttonTitle: "Waiting...")
        let attachmentText = attachments.isEmpty
            ? ""
            : "\n\nAttached: " + attachments.map(\.name).joined(separator: ", ")
        chatView.appendLocalMessage(role: .user, text: userText.isEmpty ? attachmentText.trimmingCharacters(in: .whitespacesAndNewlines) : userText + attachmentText)
        let pendingID = "pending-\(UUID().uuidString)"
        pendingMessageID = pendingID
        chatView.showPendingStatus("Sent from desktop command bar. Big Head is thinking...")
    }

    func finishWaitingForReply() {
        composer.setSendingEnabled(true)
        if let pendingMessageID {
            chatView.removeLocalMessage(id: pendingMessageID)
            self.pendingMessageID = nil
        }
        chatView.clearPendingStatus()
        focusCommandBar()
    }

    func markReplyStillPending() {
        guard pendingMessageID != nil else { return }
        chatView.showPendingStatus("Still waiting for a runtime response. Restart older runtimes if desktop transcript events are unavailable.")
        self.pendingMessageID = nil
        composer.setSendingEnabled(true)
    }

    func restoreWaitingIndicatorIfNeeded() {
        guard pendingMessageID != nil else { return }
        chatView.showPendingStatus("Big Head is thinking...")
    }

    private func build() {
        guard let contentView = window?.contentView else { return }
        contentView.wantsLayer = true
        contentView.layer?.backgroundColor = DesktopTheme.windowBackground.cgColor

        let root = NSStackView()
        root.orientation = .vertical
        root.alignment = .width
        root.distribution = .fill
        root.spacing = 12
        root.edgeInsets = NSEdgeInsets(top: 24, left: 26, bottom: 20, right: 26)
        root.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(root)

        titleLabel.font = NSFont.systemFont(ofSize: 29, weight: .bold)
        subtitleLabel.textColor = .secondaryLabelColor
        subtitleLabel.font = NSFont.systemFont(ofSize: 14, weight: .regular)
        runtimeLabel.font = NSFont.systemFont(ofSize: 12.5, weight: .medium)
        runtimeLabel.textColor = .secondaryLabelColor

        connectionDot.wantsLayer = true
        connectionDot.layer?.cornerRadius = 6
        connectionDot.layer?.backgroundColor = NSColor.systemRed.cgColor
        connectionDot.translatesAutoresizingMaskIntoConstraints = false
        connectionDot.widthAnchor.constraint(equalToConstant: 12).isActive = true
        connectionDot.heightAnchor.constraint(equalToConstant: 12).isActive = true

        runtimeButton.target = self
        runtimeButton.action = #selector(toggleRuntime)
        refreshButton.target = self
        refreshButton.action = #selector(refresh)
        settingsButton.target = self
        settingsButton.action = #selector(settings)
        [runtimeButton, refreshButton, settingsButton].forEach(DesktopTheme.styleToolbarButton)
        modeControl.target = self
        modeControl.action = #selector(changeViewMode)
        modeControl.selectedSegment = 0
        modeControl.segmentStyle = .rounded
        modeControl.controlSize = .regular
        for index in ViewMode.allCases.indices {
            modeControl.setWidth(index == 0 ? 92 : 68, forSegment: index)
        }

        let titleStack = NSStackView(views: [connectionDot, titleLabel])
        titleStack.orientation = .horizontal
        titleStack.alignment = .centerY
        titleStack.spacing = 10

        let titleColumn = NSStackView(views: [titleStack, subtitleLabel, runtimeLabel])
        titleColumn.orientation = .vertical
        titleColumn.alignment = .leading
        titleColumn.spacing = 4
        titleColumn.setContentHuggingPriority(.defaultLow, for: .horizontal)

        let buttonStack = NSStackView(views: [modeControl, runtimeButton, refreshButton, settingsButton])
        buttonStack.orientation = .horizontal
        buttonStack.spacing = 8
        buttonStack.setContentHuggingPriority(.required, for: .horizontal)

        configLabel.font = NSFont.systemFont(ofSize: 12, weight: .medium)
        configLabel.textColor = .systemRed
        configLabel.isHidden = true

        let header = NSView()
        header.translatesAutoresizingMaskIntoConstraints = false
        header.wantsLayer = true
        header.layer?.backgroundColor = NSColor.clear.cgColor
        header.addSubview(titleColumn)
        header.addSubview(buttonStack)
        titleColumn.translatesAutoresizingMaskIntoConstraints = false
        buttonStack.translatesAutoresizingMaskIntoConstraints = false
        let configSlot = centeredWorkspaceSlot(for: configLabel)
        configSlot.isHidden = true
        self.configSlot = configSlot

        root.addArrangedSubview(centeredWorkspaceSlot(for: header))
        root.addArrangedSubview(configSlot)
        root.addArrangedSubview(centeredWorkspaceSlot(for: ambientSurface))
        header.setContentHuggingPriority(.required, for: .vertical)
        header.setContentCompressionResistancePriority(.required, for: .vertical)
        ambientSurface.setContentHuggingPriority(.required, for: .vertical)
        ambientSurface.setContentCompressionResistancePriority(.required, for: .vertical)

        statusBoard.onAction = { [weak self] item, action in
            self?.selectStatusItem(item)
            self?.onStatusAction?(item, action)
        }
        inspectorView.onAction = { [weak self] item, action in
            self?.selectStatusItem(item)
            self?.onStatusAction?(item, action)
        }
        documentView.onReferenceAction = { [weak self] document, reference, action in
            self?.onDocumentReferenceAction?(document, reference, action)
        }

        let statusPanel = DesktopPanelView(
            title: "Status",
            subtitle: "Agent-published work, progress, and choices",
            content: statusBoard
        )
        let chatPanel = DesktopPanelView(
            title: "Conversation",
            subtitle: "Your Spark input and Big Head replies",
            content: rightContentContainer
        )
        self.statusPanel = statusPanel
        self.chatPanel = chatPanel

        contentStack.orientation = .horizontal
        contentStack.alignment = .height
        contentStack.distribution = .fill
        contentStack.spacing = 14
        contentStack.detachesHiddenViews = true
        contentStack.translatesAutoresizingMaskIntoConstraints = false
        contentStack.addArrangedSubview(statusPanel)
        contentStack.addArrangedSubview(chatPanel)
        root.addArrangedSubview(centeredWorkspaceSlot(for: contentStack))
        statusPanel.setContentHuggingPriority(.required, for: .horizontal)
        statusPanel.setContentCompressionResistancePriority(.required, for: .horizontal)
        chatPanel.setContentHuggingPriority(.defaultLow, for: .horizontal)
        chatPanel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        contentStack.setContentHuggingPriority(.defaultLow, for: .vertical)
        contentStack.setContentCompressionResistancePriority(.defaultLow, for: .vertical)

        rightContentContainer.translatesAutoresizingMaskIntoConstraints = false
        rightContentContainer.addSubview(chatView)
        rightContentContainer.addSubview(documentView)
        rightContentContainer.addSubview(inspectorView)
        chatView.translatesAutoresizingMaskIntoConstraints = false
        documentView.translatesAutoresizingMaskIntoConstraints = false
        inspectorView.translatesAutoresizingMaskIntoConstraints = false

        composer.setPlaceholder("Ask Big Head to hold, inspect, or change something...")
        composer.onSubmit = { [weak self] text, attachments in
            self?.onSend?(text, attachments)
        }
        root.addArrangedSubview(centeredWorkspaceSlot(for: composer, maxWidth: 1080))
        composer.setContentHuggingPriority(.required, for: .vertical)
        composer.setContentCompressionResistancePriority(.required, for: .vertical)

        NSLayoutConstraint.activate([
            root.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            root.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
            root.topAnchor.constraint(equalTo: contentView.topAnchor),
            root.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
            header.heightAnchor.constraint(equalToConstant: 92),
            titleColumn.leadingAnchor.constraint(equalTo: header.leadingAnchor),
            titleColumn.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            titleColumn.trailingAnchor.constraint(lessThanOrEqualTo: buttonStack.leadingAnchor, constant: -24),
            buttonStack.trailingAnchor.constraint(equalTo: header.trailingAnchor),
            buttonStack.topAnchor.constraint(equalTo: header.topAnchor, constant: 10),
            contentStack.heightAnchor.constraint(greaterThanOrEqualToConstant: 480),
            ambientSurface.heightAnchor.constraint(equalToConstant: 98),
            chatPanel.widthAnchor.constraint(greaterThanOrEqualToConstant: 580),
            chatView.leadingAnchor.constraint(equalTo: rightContentContainer.leadingAnchor),
            chatView.trailingAnchor.constraint(equalTo: rightContentContainer.trailingAnchor),
            chatView.topAnchor.constraint(equalTo: rightContentContainer.topAnchor),
            chatView.bottomAnchor.constraint(equalTo: rightContentContainer.bottomAnchor),
            documentView.leadingAnchor.constraint(equalTo: rightContentContainer.leadingAnchor),
            documentView.trailingAnchor.constraint(equalTo: rightContentContainer.trailingAnchor),
            documentView.topAnchor.constraint(equalTo: rightContentContainer.topAnchor),
            documentView.bottomAnchor.constraint(equalTo: rightContentContainer.bottomAnchor),
            inspectorView.leadingAnchor.constraint(equalTo: rightContentContainer.leadingAnchor),
            inspectorView.trailingAnchor.constraint(equalTo: rightContentContainer.trailingAnchor),
            inspectorView.topAnchor.constraint(equalTo: rightContentContainer.topAnchor),
            inspectorView.bottomAnchor.constraint(equalTo: rightContentContainer.bottomAnchor)
        ])
        statusWidthConstraint = statusPanel.widthAnchor.constraint(equalToConstant: 390)
        statusWidthConstraint?.isActive = true
        applyMode(.dwelling)
    }

    private func centeredWorkspaceSlot(for child: NSView, maxWidth: CGFloat? = nil) -> NSView {
        let slot = NSView()
        slot.translatesAutoresizingMaskIntoConstraints = false
        child.translatesAutoresizingMaskIntoConstraints = false
        slot.addSubview(child)

        let fillWidth = child.widthAnchor.constraint(equalTo: slot.widthAnchor)
        fillWidth.priority = .defaultHigh

        NSLayoutConstraint.activate([
            child.topAnchor.constraint(equalTo: slot.topAnchor),
            child.bottomAnchor.constraint(equalTo: slot.bottomAnchor),
            child.centerXAnchor.constraint(equalTo: slot.centerXAnchor),
            child.leadingAnchor.constraint(greaterThanOrEqualTo: slot.leadingAnchor),
            child.trailingAnchor.constraint(lessThanOrEqualTo: slot.trailingAnchor),
            child.widthAnchor.constraint(lessThanOrEqualToConstant: maxWidth ?? workspaceMaxWidth),
            fillWidth
        ])
        return slot
    }

    private func applyMode(_ mode: ViewMode) {
        currentMode = mode
        if let index = ViewMode.allCases.firstIndex(of: mode) {
            modeControl.selectedSegment = index
        }
        guard let statusPanel, let chatPanel else { return }
        statusPanel.isHidden = false
        chatPanel.isHidden = false
        ambientSurface.isHidden = false

        switch mode {
        case .dwelling:
            statusPanel.setTitle("Status", subtitle: "Agent-published work, progress, and choices")
            chatPanel.setTitle("Conversation", subtitle: "Your Spark input and Big Head replies")
            statusWidthConstraint?.constant = 420
            composer.setPlaceholder("Tell Big Head what should happen in this dwelling...")
        case .work:
            statusPanel.setTitle("Work Queue", subtitle: "Linear-style active work, blockers, and progress")
            chatPanel.setTitle("Activity", subtitle: "Runtime events and agent decisions")
            statusWidthConstraint?.constant = 480
            composer.setPlaceholder("Create, advance, or inspect work...")
        case .chat:
            statusPanel.isHidden = true
            chatPanel.setTitle("Conversation", subtitle: "Chat-first companion mode")
            composer.setPlaceholder("Talk with Big Head...")
        case .notes:
            statusPanel.setTitle("Index", subtitle: "Things-like status and active anchors")
            chatPanel.setTitle("Living Notes", subtitle: "Craft-style readable output from Big Head")
            statusWidthConstraint?.constant = 360
            composer.setPlaceholder("Ask Big Head to write, summarize, or crystallize...")
        }
        updateRightContentVisibility()
    }

    private func updateRightContentVisibility() {
        let shouldShowInspector = selectedStatusItem != nil && currentMode != .chat
        let shouldShowDocuments = currentMode == .notes && hasDocuments && !shouldShowInspector
        documentView.isHidden = !shouldShowDocuments
        inspectorView.isHidden = !shouldShowInspector
        chatView.isHidden = shouldShowDocuments || shouldShowInspector
    }

    private func selectStatusItem(_ item: StatusItem) {
        selectedStatusItem = item
        inspectorView.render(item: item, messages: latestState.messages, documents: latestState.documents)
        updateRightContentVisibility()
    }

    private func reconcileSelectedItem(with state: StatusState) {
        guard let selectedStatusItem else { return }
        self.selectedStatusItem = state.items.first { $0.id == selectedStatusItem.id && $0.state != .hidden }
    }

    @objc private func toggleRuntime() {
        switch runtimeStatus {
        case .managed:
            onStopRuntime?()
        case .offline:
            onStartRuntime?()
        case .external, .starting:
            break
        }
    }

    @objc private func refresh() {
        onRefresh?()
    }

    @objc private func settings() {
        onSettings?()
    }

    @objc private func changeViewMode() {
        let index = modeControl.selectedSegment
        guard ViewMode.allCases.indices.contains(index) else { return }
        applyMode(ViewMode.allCases[index])
    }

    private func focusCommandBar() {
        window?.makeFirstResponder(composer)
    }

    private func installKeyMonitor() {
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard let self else { return event }
            return self.handleKeyDown(event) ? nil : event
        }
    }

    private func handleKeyDown(_ event: NSEvent) -> Bool {
        guard window?.isKeyWindow == true,
              event.modifierFlags.intersection(.deviceIndependentFlagsMask).contains(.command),
              let characters = event.charactersIgnoringModifiers?.lowercased()
        else {
            return false
        }

        switch characters {
        case "1", "2", "3", "4":
            guard let number = Int(characters), ViewMode.allCases.indices.contains(number - 1) else { return true }
            applyMode(ViewMode.allCases[number - 1])
            return true
        case "l":
            focusCommandBar()
            return true
        case "r":
            onRefresh?()
            return true
        default:
            return false
        }
    }
}

extension DesktopWindowController: NSWindowDelegate {
    func windowDidBecomeKey(_ notification: Notification) {
        focusCommandBar()
    }

    func windowWillClose(_ notification: Notification) {
        if let keyMonitor {
            NSEvent.removeMonitor(keyMonitor)
            self.keyMonitor = nil
        }
    }
}
