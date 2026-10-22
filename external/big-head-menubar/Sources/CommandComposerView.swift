import AppKit

@MainActor
final class CommandComposerView: NSView {
    var onSubmit: ((String, [DesktopAttachment]) -> Void)?

    private let trayStack = NSStackView()
    private let scrollView = NSScrollView()
    private let textView = ComposerTextView()
    private let placeholderLabel = NSTextField(labelWithString: "")
    private let attachButton = NSButton(title: "Attach", target: nil, action: nil)
    private let sendButton = NSButton(title: "Send", target: nil, action: nil)
    private var heightConstraint: NSLayoutConstraint?
    private var placeholder = ""
    private var attachments: [DesktopAttachment] = []

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        translatesAutoresizingMaskIntoConstraints = false
        build()
        updatePlaceholderVisibility()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    var stringValue: String {
        get { textView.string }
        set {
            textView.string = newValue
            updatePlaceholderVisibility()
            updateHeight()
        }
    }

    override var acceptsFirstResponder: Bool { true }

    override func becomeFirstResponder() -> Bool {
        window?.makeFirstResponder(textView)
        return true
    }

    func setPlaceholder(_ text: String) {
        placeholder = text
        placeholderLabel.stringValue = text
        updatePlaceholderVisibility()
    }

    func setSendingEnabled(_ enabled: Bool, buttonTitle: String = "Send") {
        textView.isEditable = enabled
        textView.textColor = enabled ? .labelColor : .secondaryLabelColor
        sendButton.isEnabled = enabled
        sendButton.title = buttonTitle
    }

    private func build() {
        DesktopTheme.applyPanelChrome(to: self, radius: 12)
        layer?.backgroundColor = DesktopTheme.raisedBackground.cgColor
        registerForDraggedTypes([.fileURL])

        trayStack.orientation = .horizontal
        trayStack.alignment = .centerY
        trayStack.spacing = 7
        trayStack.translatesAutoresizingMaskIntoConstraints = false

        scrollView.drawsBackground = false
        scrollView.borderType = .noBorder
        scrollView.hasVerticalScroller = true
        scrollView.translatesAutoresizingMaskIntoConstraints = false

        textView.font = NSFont.systemFont(ofSize: 15)
        textView.textColor = .labelColor
        textView.backgroundColor = .clear
        textView.drawsBackground = false
        textView.isRichText = false
        textView.isAutomaticQuoteSubstitutionEnabled = false
        textView.isAutomaticDashSubstitutionEnabled = false
        textView.allowsUndo = true
        textView.textContainerInset = NSSize(width: 8, height: 8)
        textView.textContainer?.widthTracksTextView = true
        textView.textContainer?.containerSize = NSSize(width: 0, height: CGFloat.greatestFiniteMagnitude)
        textView.minSize = NSSize(width: 0, height: 36)
        textView.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.autoresizingMask = [.width]
        textView.onSubmit = { [weak self] in
            self?.submit()
        }
        textView.onAttachFiles = { [weak self] urls in
            self?.addFileURLs(urls)
        }
        textView.onAttachImageFromPasteboard = { [weak self] in
            self?.addImageFromPasteboard()
        }
        textView.onTextChanged = { [weak self] in
            self?.updatePlaceholderVisibility()
            self?.updateHeight()
        }
        scrollView.documentView = textView

        placeholderLabel.font = NSFont.systemFont(ofSize: 15)
        placeholderLabel.textColor = .placeholderTextColor
        placeholderLabel.lineBreakMode = .byTruncatingTail
        placeholderLabel.translatesAutoresizingMaskIntoConstraints = false

        attachButton.target = self
        attachButton.action = #selector(selectAttachments)
        DesktopTheme.styleToolbarButton(attachButton)
        attachButton.translatesAutoresizingMaskIntoConstraints = false

        sendButton.target = self
        sendButton.action = #selector(submitFromButton)
        DesktopTheme.styleToolbarButton(sendButton)
        sendButton.contentTintColor = DesktopTheme.accent
        sendButton.translatesAutoresizingMaskIntoConstraints = false

        let hint = NSTextField(labelWithString: "Return send  Shift-Return newline")
        hint.font = NSFont.systemFont(ofSize: 11)
        hint.textColor = .tertiaryLabelColor
        hint.translatesAutoresizingMaskIntoConstraints = false

        addSubview(trayStack)
        addSubview(scrollView)
        addSubview(placeholderLabel)
        addSubview(attachButton)
        addSubview(sendButton)
        addSubview(hint)

        heightConstraint = heightAnchor.constraint(equalToConstant: 64)
        heightConstraint?.isActive = true

        NSLayoutConstraint.activate([
            trayStack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            trayStack.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -12),
            trayStack.topAnchor.constraint(equalTo: topAnchor, constant: 9),

            scrollView.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
            scrollView.trailingAnchor.constraint(equalTo: attachButton.leadingAnchor, constant: -10),
            scrollView.topAnchor.constraint(equalTo: trayStack.bottomAnchor, constant: 7),
            scrollView.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -9),

            placeholderLabel.leadingAnchor.constraint(equalTo: scrollView.leadingAnchor, constant: 12),
            placeholderLabel.trailingAnchor.constraint(equalTo: scrollView.trailingAnchor, constant: -12),
            placeholderLabel.topAnchor.constraint(equalTo: scrollView.topAnchor, constant: 8),

            attachButton.trailingAnchor.constraint(equalTo: sendButton.leadingAnchor, constant: -8),
            attachButton.bottomAnchor.constraint(equalTo: sendButton.bottomAnchor),
            attachButton.widthAnchor.constraint(equalToConstant: 78),

            sendButton.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            sendButton.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -11),
            sendButton.widthAnchor.constraint(equalToConstant: 76),

            hint.trailingAnchor.constraint(equalTo: sendButton.trailingAnchor),
            hint.bottomAnchor.constraint(equalTo: sendButton.topAnchor, constant: -5)
        ])
    }

    private func updatePlaceholderVisibility() {
        placeholderLabel.stringValue = placeholder
        placeholderLabel.isHidden = !textView.string.isEmpty
    }

    private func updateHeight() {
        guard let textContainer = textView.textContainer else { return }
        textView.layoutManager?.ensureLayout(for: textContainer)
        let used = textView.layoutManager?.usedRect(for: textContainer).height ?? 36
        let trayHeight: CGFloat = attachments.isEmpty ? 0 : 30
        let next = min(max(64, used + 42 + trayHeight), 184)
        heightConstraint?.constant = next
        scrollView.hasVerticalScroller = next >= 150
    }

    @objc private func submitFromButton() {
        submit()
    }

    private func submit() {
        let text = textView.string.trimmingCharacters(in: .whitespacesAndNewlines)
        guard (!text.isEmpty || !attachments.isEmpty), textView.isEditable else { return }
        let outgoingAttachments = attachments
        textView.string = ""
        attachments.removeAll()
        renderTray()
        updatePlaceholderVisibility()
        updateHeight()
        onSubmit?(text, outgoingAttachments)
    }

    @objc private func selectAttachments() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.begin { [weak self] response in
            guard response == .OK else { return }
            self?.addFileURLs(panel.urls)
        }
    }

    private func addFileURLs(_ urls: [URL]) {
        let next = urls.map { AttachmentStore.attachment(fromFileURL: $0) }
        appendAttachments(next)
    }

    private func addImageFromPasteboard() {
        guard let attachment = AttachmentStore.saveImageFromPasteboard() else { return }
        appendAttachments([attachment])
    }

    private func appendAttachments(_ next: [DesktopAttachment]) {
        guard !next.isEmpty else { return }
        for attachment in next where !attachments.contains(where: { $0.path == attachment.path }) {
            attachments.append(attachment)
        }
        renderTray()
        updateHeight()
    }

    private func renderTray() {
        trayStack.arrangedSubviews.forEach { view in
            trayStack.removeArrangedSubview(view)
            view.removeFromSuperview()
        }
        trayStack.isHidden = attachments.isEmpty
        for attachment in attachments {
            trayStack.addArrangedSubview(AttachmentChipView(attachment: attachment) { [weak self] attachment in
                self?.attachments.removeAll { $0.id == attachment.id }
                self?.renderTray()
                self?.updateHeight()
            })
        }
    }

    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        sender.draggingPasteboard.canReadObject(forClasses: [NSURL.self], options: nil) ? .copy : []
    }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        guard let urls = sender.draggingPasteboard.readObjects(forClasses: [NSURL.self], options: nil) as? [URL],
              !urls.isEmpty
        else {
            return false
        }
        addFileURLs(urls)
        return true
    }
}

private final class ComposerTextView: NSTextView {
    var onSubmit: (() -> Void)?
    var onTextChanged: (() -> Void)?
    var onAttachFiles: (([URL]) -> Void)?
    var onAttachImageFromPasteboard: (() -> Void)?

    override func keyDown(with event: NSEvent) {
        let isReturn = event.keyCode == 36 || event.keyCode == 76
        let wantsNewline = event.modifierFlags.intersection(.deviceIndependentFlagsMask).contains(.shift)
        if isReturn && !wantsNewline {
            onSubmit?()
            return
        }
        super.keyDown(with: event)
    }

    override func paste(_ sender: Any?) {
        let pasteboard = NSPasteboard.general
        if let urls = pasteboard.readObjects(forClasses: [NSURL.self], options: nil) as? [URL],
           !urls.isEmpty {
            onAttachFiles?(urls)
            return
        }
        if NSImage(pasteboard: pasteboard) != nil {
            onAttachImageFromPasteboard?()
            return
        }
        if let text = pasteboard.string(forType: .string) {
            insertText(text, replacementRange: selectedRange())
            onTextChanged?()
            return
        }
        super.paste(sender)
    }

    override func didChangeText() {
        super.didChangeText()
        onTextChanged?()
    }
}

private final class AttachmentChipView: NSView {
    private let attachment: DesktopAttachment
    private let onRemove: (DesktopAttachment) -> Void

    init(attachment: DesktopAttachment, onRemove: @escaping (DesktopAttachment) -> Void) {
        self.attachment = attachment
        self.onRemove = onRemove
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        build()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    private func build() {
        wantsLayer = true
        layer?.backgroundColor = DesktopTheme.subtleFill.withAlphaComponent(0.72).cgColor
        layer?.cornerRadius = 8
        layer?.borderWidth = 1
        layer?.borderColor = DesktopTheme.hairline.cgColor

        let label = NSTextField(labelWithString: attachment.name)
        label.font = NSFont.systemFont(ofSize: 11.5, weight: .medium)
        label.textColor = .labelColor
        label.lineBreakMode = .byTruncatingMiddle
        label.translatesAutoresizingMaskIntoConstraints = false

        let remove = NSButton(title: "x", target: self, action: #selector(removeAttachment))
        remove.bezelStyle = .inline
        remove.font = NSFont.systemFont(ofSize: 11, weight: .semibold)
        remove.translatesAutoresizingMaskIntoConstraints = false

        addSubview(label)
        addSubview(remove)

        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 24),
            widthAnchor.constraint(lessThanOrEqualToConstant: 210),

            label.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 9),
            label.trailingAnchor.constraint(equalTo: remove.leadingAnchor, constant: -4),
            label.centerYAnchor.constraint(equalTo: centerYAnchor),

            remove.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            remove.centerYAnchor.constraint(equalTo: centerYAnchor),
            remove.widthAnchor.constraint(equalToConstant: 16)
        ])
    }

    @objc private func removeAttachment() {
        onRemove(attachment)
    }
}
