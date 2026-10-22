import AppKit

@MainActor
final class DocumentSurfaceView: NSScrollView {
    var onReferenceAction: ((StatusDocument, StatusDocumentReference, String) -> Void)?

    private let stack = NSStackView()

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        drawsBackground = false
        hasVerticalScroller = true
        borderType = .noBorder

        stack.orientation = .vertical
        stack.alignment = .width
        stack.spacing = 14
        stack.edgeInsets = NSEdgeInsets(top: 16, left: 18, bottom: 22, right: 18)
        stack.translatesAutoresizingMaskIntoConstraints = false

        let document = FlippedDocumentView()
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

        render([])
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    func render(_ documents: [StatusDocument]) {
        stack.arrangedSubviews.forEach { view in
            stack.removeArrangedSubview(view)
            view.removeFromSuperview()
        }

        guard !documents.isEmpty else {
            addEmptyState()
            return
        }

        for document in documents {
            addDocument(document)
        }
    }

    private func addEmptyState() {
        let empty = DocumentEmptyView()
        stack.addArrangedSubview(empty)
        empty.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
    }

    private func addDocument(_ document: StatusDocument) {
        let view = DocumentCardView(document: document) { [weak self] document, reference, action in
            self?.onReferenceAction?(document, reference, action)
        }
        stack.addArrangedSubview(view)
        view.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
    }
}

final class DocumentCardView: NSView {
    private let document: StatusDocument
    private let onReferenceAction: (StatusDocument, StatusDocumentReference, String) -> Void

    init(
        document: StatusDocument,
        onReferenceAction: @escaping (StatusDocument, StatusDocumentReference, String) -> Void
    ) {
        self.document = document
        self.onReferenceAction = onReferenceAction
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        build()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    private func build() {
        DesktopTheme.applyCardChrome(to: self, radius: 8)
        layer?.backgroundColor = DesktopTheme.cardBackground.withAlphaComponent(0.98).cgColor

        let kind = NSTextField(labelWithString: (document.kind ?? "note").uppercased())
        kind.font = NSFont.monospacedSystemFont(ofSize: 10.5, weight: .semibold)
        kind.textColor = .secondaryLabelColor

        let title = NSTextField(labelWithString: document.title)
        title.font = NSFont.systemFont(ofSize: 21, weight: .bold)
        title.textColor = .labelColor
        title.lineBreakMode = .byWordWrapping
        title.maximumNumberOfLines = 0

        let headerViews: [NSView]
        if let subtitle = document.subtitle, !subtitle.isEmpty {
            let subtitleView = NSTextField(labelWithString: subtitle)
            subtitleView.font = NSFont.systemFont(ofSize: 13, weight: .medium)
            subtitleView.textColor = .secondaryLabelColor
            subtitleView.lineBreakMode = .byWordWrapping
            subtitleView.maximumNumberOfLines = 0
            headerViews = [kind, title, subtitleView]
        } else {
            headerViews = [kind, title]
        }

        let header = NSStackView(views: headerViews)
        header.orientation = .vertical
        header.alignment = .leading
        header.spacing = 6
        header.translatesAutoresizingMaskIntoConstraints = false

        let body = NSTextField(labelWithAttributedString: MarkdownRendering.render(document.text, role: .assistant))
        body.isSelectable = true
        body.lineBreakMode = .byWordWrapping
        body.maximumNumberOfLines = 0
        body.translatesAutoresizingMaskIntoConstraints = false

        addSubview(header)
        addSubview(body)

        let references = makeReferencesView(document.references)
        addSubview(references)

        NSLayoutConstraint.activate([
            header.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 22),
            header.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -22),
            header.topAnchor.constraint(equalTo: topAnchor, constant: 20),

            body.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 22),
            body.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -22),
            body.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 18),
            references.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 22),
            references.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -22),
            references.topAnchor.constraint(equalTo: body.bottomAnchor, constant: document.references.isEmpty ? 0 : 18),
            references.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -22)
        ])
    }

    private func makeReferencesView(_ references: [StatusDocumentReference]) -> NSView {
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .width
        stack.spacing = 8
        stack.translatesAutoresizingMaskIntoConstraints = false
        stack.isHidden = references.isEmpty

        guard !references.isEmpty else { return stack }

        let title = NSTextField(labelWithString: "References")
        title.font = NSFont.systemFont(ofSize: 12, weight: .semibold)
        title.textColor = .secondaryLabelColor
        stack.addArrangedSubview(title)

        let rows = NSStackView()
        rows.orientation = .vertical
        rows.alignment = .width
        rows.spacing = 6
        rows.translatesAutoresizingMaskIntoConstraints = false

        for reference in references {
            rows.addArrangedSubview(
                DocumentReferenceRowView(reference: reference) { [weak self] reference, action in
                    guard let self else { return }
                    self.onReferenceAction(self.document, reference, action)
                }
            )
        }
        stack.addArrangedSubview(rows)
        return stack
    }
}

final class DocumentReferenceRowView: NSView {
    private let reference: StatusDocumentReference
    private let onAction: (StatusDocumentReference, String) -> Void

    init(
        reference: StatusDocumentReference,
        onAction: @escaping (StatusDocumentReference, String) -> Void
    ) {
        self.reference = reference
        self.onAction = onAction
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
        build()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    private func build() {
        wantsLayer = true
        layer?.backgroundColor = DesktopTheme.subtleFill.withAlphaComponent(0.58).cgColor
        layer?.cornerRadius = 7
        layer?.borderWidth = 1
        layer?.borderColor = DesktopTheme.hairline.cgColor

        let kind = NSTextField(labelWithString: reference.kind.uppercased())
        kind.font = NSFont.monospacedSystemFont(ofSize: 10, weight: .semibold)
        kind.textColor = .secondaryLabelColor
        kind.translatesAutoresizingMaskIntoConstraints = false

        let title = NSTextField(labelWithString: reference.title)
        title.font = NSFont.systemFont(ofSize: 13, weight: .semibold)
        title.textColor = .labelColor
        title.lineBreakMode = .byTruncatingTail
        title.translatesAutoresizingMaskIntoConstraints = false

        let identifier = NSTextField(labelWithString: reference.id)
        identifier.font = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        identifier.textColor = .secondaryLabelColor
        identifier.lineBreakMode = .byTruncatingTail
        identifier.translatesAutoresizingMaskIntoConstraints = false

        let description = NSTextField(labelWithString: reference.description ?? "")
        description.font = NSFont.systemFont(ofSize: 11.5)
        description.textColor = .secondaryLabelColor
        description.lineBreakMode = .byTruncatingTail
        description.translatesAutoresizingMaskIntoConstraints = false
        description.isHidden = reference.description?.isEmpty ?? true

        let action = NSTextField(labelWithString: "OPEN")
        action.font = NSFont.monospacedSystemFont(ofSize: 10, weight: .bold)
        action.textColor = DesktopTheme.accent
        action.translatesAutoresizingMaskIntoConstraints = false

        addSubview(kind)
        addSubview(title)
        addSubview(identifier)
        addSubview(description)
        addSubview(action)

        NSLayoutConstraint.activate([
            kind.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            kind.topAnchor.constraint(equalTo: topAnchor, constant: 9),
            kind.widthAnchor.constraint(equalToConstant: 74),

            title.leadingAnchor.constraint(equalTo: kind.trailingAnchor, constant: 10),
            title.trailingAnchor.constraint(equalTo: identifier.leadingAnchor, constant: -10),
            title.topAnchor.constraint(equalTo: topAnchor, constant: 8),

            identifier.trailingAnchor.constraint(equalTo: action.leadingAnchor, constant: -10),
            identifier.centerYAnchor.constraint(equalTo: title.centerYAnchor),
            identifier.widthAnchor.constraint(lessThanOrEqualToConstant: 130),

            description.leadingAnchor.constraint(equalTo: title.leadingAnchor),
            description.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            description.topAnchor.constraint(equalTo: title.bottomAnchor, constant: 3),
            description.bottomAnchor.constraint(lessThanOrEqualTo: bottomAnchor, constant: -8),

            action.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            action.centerYAnchor.constraint(equalTo: title.centerYAnchor),
            action.widthAnchor.constraint(equalToConstant: 34)
        ])
    }

    override func mouseDown(with event: NSEvent) {
        onAction(reference, "open_reference")
    }

    override func resetCursorRects() {
        addCursorRect(bounds, cursor: .pointingHand)
    }
}

final class DocumentEmptyView: NSView {
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
        let title = NSTextField(labelWithString: "No living notes yet")
        title.font = NSFont.systemFont(ofSize: 16, weight: .semibold)
        title.textColor = .labelColor
        title.alignment = .center

        let description = NSTextField(labelWithString: "Ask Big Head to publish a desktop document with render_status upsert_document.")
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
