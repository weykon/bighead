import AppKit

@MainActor
final class SettingsWindowController: NSWindowController {
    var onSave: ((BigHeadEnvironment) -> Void)?

    private var environment = EnvironmentSettingsStore.load()
    private let keyField = NSSecureTextField()
    private let pathLabel = NSTextField(labelWithString: "")
    private let messageLabel = NSTextField(labelWithString: "")

    init() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 560, height: 260),
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false
        )
        window.title = "BigHead Settings"
        window.center()
        super.init(window: window)
        build()
        reload()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    func reload() {
        environment = EnvironmentSettingsStore.load()
        keyField.stringValue = environment.minimaxAPIKey
        pathLabel.stringValue = environment.envFile.path
        messageLabel.stringValue = environment.hasAPIKey ? "API key is configured." : "MiniMax API key is missing."
        messageLabel.textColor = environment.hasAPIKey ? .systemGreen : .systemRed
    }

    private func build() {
        guard let contentView = window?.contentView else { return }

        let root = NSStackView()
        root.orientation = .vertical
        root.spacing = 14
        root.edgeInsets = NSEdgeInsets(top: 20, left: 20, bottom: 20, right: 20)
        root.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(root)

        let title = NSTextField(labelWithString: "Provider")
        title.font = NSFont.systemFont(ofSize: 18, weight: .semibold)

        let hint = NSTextField(labelWithString: "BigHead currently uses MiniMax. The key is saved locally in Application Support.")
        hint.font = NSFont.systemFont(ofSize: 12)
        hint.textColor = .secondaryLabelColor

        let keyLabel = NSTextField(labelWithString: "MiniMax API Key")
        keyLabel.font = NSFont.systemFont(ofSize: 13, weight: .medium)
        keyField.placeholderString = "MiniMax_API_KEY"
        keyField.font = NSFont.systemFont(ofSize: 13)
        keyField.target = self
        keyField.action = #selector(save)

        let pasteButton = NSButton(title: "Paste", target: self, action: #selector(pasteKey))
        let keyRow = NSStackView(views: [keyField, pasteButton])
        keyRow.orientation = .horizontal
        keyRow.spacing = 8

        pathLabel.font = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        pathLabel.textColor = .secondaryLabelColor
        pathLabel.lineBreakMode = .byTruncatingMiddle

        messageLabel.font = NSFont.systemFont(ofSize: 12, weight: .medium)

        let saveButton = NSButton(title: "Save", target: self, action: #selector(save))
        let revealButton = NSButton(title: "Reveal Folder", target: self, action: #selector(revealFolder))
        let buttons = NSStackView(views: [revealButton, NSView(), saveButton])
        buttons.orientation = .horizontal
        buttons.spacing = 8

        root.addArrangedSubview(title)
        root.addArrangedSubview(hint)
        root.addArrangedSubview(keyLabel)
        root.addArrangedSubview(keyRow)
        root.addArrangedSubview(pathLabel)
        root.addArrangedSubview(messageLabel)
        root.addArrangedSubview(NSView())
        root.addArrangedSubview(buttons)

        NSLayoutConstraint.activate([
            root.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            root.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
            root.topAnchor.constraint(equalTo: contentView.topAnchor),
            root.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
            keyField.heightAnchor.constraint(equalToConstant: 28),
            pasteButton.widthAnchor.constraint(equalToConstant: 72)
        ])
    }

    override func showWindow(_ sender: Any?) {
        super.showWindow(sender)
        window?.makeFirstResponder(keyField)
    }

    @objc private func save() {
        environment.minimaxAPIKey = keyField.stringValue
        do {
            try EnvironmentSettingsStore.save(environment)
            reload()
            onSave?(environment)
        } catch {
            messageLabel.stringValue = "Save failed: \(error.localizedDescription)"
            messageLabel.textColor = .systemRed
        }
    }

    @objc private func revealFolder() {
        NSWorkspace.shared.activateFileViewerSelecting([environment.envFile])
    }

    @objc private func pasteKey() {
        guard let text = NSPasteboard.general.string(forType: .string) else {
            messageLabel.stringValue = "Clipboard does not contain text."
            messageLabel.textColor = .systemRed
            return
        }
        keyField.stringValue = text.trimmingCharacters(in: .whitespacesAndNewlines)
        messageLabel.stringValue = "Token pasted. Click Save to apply."
        messageLabel.textColor = .secondaryLabelColor
    }
}
