import AppKit

enum DesktopTheme {
    static let windowBackground = adaptive(
        light: NSColor(calibratedRed: 0.938, green: 0.942, blue: 0.948, alpha: 1),
        dark: NSColor(calibratedRed: 0.075, green: 0.078, blue: 0.084, alpha: 1)
    )
    static let panelBackground = adaptive(
        light: NSColor(calibratedRed: 0.976, green: 0.978, blue: 0.982, alpha: 1),
        dark: NSColor(calibratedRed: 0.112, green: 0.116, blue: 0.126, alpha: 1)
    )
    static let cardBackground = adaptive(
        light: NSColor(calibratedRed: 1.0, green: 1.0, blue: 1.0, alpha: 1),
        dark: NSColor(calibratedRed: 0.150, green: 0.154, blue: 0.166, alpha: 1)
    )
    static let raisedBackground = adaptive(
        light: NSColor(calibratedRed: 0.990, green: 0.991, blue: 0.994, alpha: 1),
        dark: NSColor(calibratedRed: 0.130, green: 0.134, blue: 0.145, alpha: 1)
    )
    static let subtleFill = adaptive(
        light: NSColor(calibratedRed: 0.918, green: 0.925, blue: 0.936, alpha: 1),
        dark: NSColor(calibratedRed: 0.205, green: 0.211, blue: 0.227, alpha: 1)
    )
    static let mutedFill = adaptive(
        light: NSColor(calibratedRed: 0.870, green: 0.882, blue: 0.902, alpha: 1),
        dark: NSColor(calibratedRed: 0.250, green: 0.258, blue: 0.278, alpha: 1)
    )
    static let hairline = adaptive(
        light: NSColor(calibratedWhite: 0.50, alpha: 0.18),
        dark: NSColor(calibratedWhite: 1.0, alpha: 0.105)
    )
    static let strongHairline = adaptive(
        light: NSColor(calibratedWhite: 0.38, alpha: 0.22),
        dark: NSColor(calibratedWhite: 1.0, alpha: 0.165)
    )
    static let accent = adaptive(
        light: NSColor(calibratedRed: 0.050, green: 0.392, blue: 0.850, alpha: 1),
        dark: NSColor(calibratedRed: 0.310, green: 0.610, blue: 1.000, alpha: 1)
    )
    static let positive = adaptive(
        light: NSColor(calibratedRed: 0.070, green: 0.620, blue: 0.310, alpha: 1),
        dark: NSColor(calibratedRed: 0.220, green: 0.780, blue: 0.420, alpha: 1)
    )
    static let warning = adaptive(
        light: NSColor(calibratedRed: 0.880, green: 0.520, blue: 0.120, alpha: 1),
        dark: NSColor(calibratedRed: 1.000, green: 0.670, blue: 0.260, alpha: 1)
    )
    static let danger = adaptive(
        light: NSColor(calibratedRed: 0.850, green: 0.160, blue: 0.140, alpha: 1),
        dark: NSColor(calibratedRed: 1.000, green: 0.360, blue: 0.330, alpha: 1)
    )

    private static func adaptive(light: NSColor, dark: NSColor) -> NSColor {
        NSColor(name: nil) { appearance in
            let match = appearance.bestMatch(from: [.darkAqua, .aqua])
            return match == .darkAqua ? dark : light
        }
    }

    static func applyPanelChrome(to view: NSView, radius: CGFloat = 12) {
        view.wantsLayer = true
        view.layer?.backgroundColor = panelBackground.cgColor
        view.layer?.cornerRadius = radius
        view.layer?.borderWidth = 1
        view.layer?.borderColor = hairline.cgColor
        view.layer?.shadowColor = NSColor.black.cgColor
        view.layer?.shadowOpacity = 0.10
        view.layer?.shadowRadius = 18
        view.layer?.shadowOffset = CGSize(width: 0, height: -2)
    }

    static func applyCardChrome(to view: NSView, radius: CGFloat = 8) {
        view.wantsLayer = true
        view.layer?.backgroundColor = cardBackground.cgColor
        view.layer?.cornerRadius = radius
        view.layer?.borderWidth = 1
        view.layer?.borderColor = hairline.cgColor
        view.layer?.shadowColor = NSColor.black.cgColor
        view.layer?.shadowOpacity = 0.055
        view.layer?.shadowRadius = 6
        view.layer?.shadowOffset = CGSize(width: 0, height: -1)
    }

    static func applyInsetChrome(to view: NSView, radius: CGFloat = 10) {
        view.wantsLayer = true
        view.layer?.backgroundColor = subtleFill.withAlphaComponent(0.62).cgColor
        view.layer?.cornerRadius = radius
        view.layer?.borderWidth = 1
        view.layer?.borderColor = hairline.cgColor
    }

    static func styleToolbarButton(_ button: NSButton) {
        button.bezelStyle = .rounded
        button.controlSize = .regular
        button.font = NSFont.systemFont(ofSize: 13, weight: .medium)
        button.contentTintColor = .labelColor
    }

    static func makeSectionTitle(_ text: String) -> NSTextField {
        let label = NSTextField(labelWithString: text)
        label.font = NSFont.systemFont(ofSize: 13.5, weight: .semibold)
        label.textColor = .labelColor
        return label
    }

    static func makeSectionSubtitle(_ text: String) -> NSTextField {
        let label = NSTextField(labelWithString: text)
        label.font = NSFont.systemFont(ofSize: 12, weight: .regular)
        label.textColor = .secondaryLabelColor
        label.lineBreakMode = .byTruncatingTail
        return label
    }

    static func makeCaption(_ text: String) -> NSTextField {
        let label = NSTextField(labelWithString: text.uppercased())
        label.font = NSFont.systemFont(ofSize: 10.5, weight: .bold)
        label.textColor = .secondaryLabelColor
        return label
    }
}

final class FlippedDocumentView: NSView {
    override var isFlipped: Bool { true }
}

final class DesktopPanelView: NSView {
    private let titleLabel: NSTextField
    private let subtitleLabel: NSTextField

    init(title: String, subtitle: String, content: NSView) {
        titleLabel = DesktopTheme.makeSectionTitle(title)
        subtitleLabel = DesktopTheme.makeSectionSubtitle(subtitle)
        super.init(frame: .zero)
        build(content: content)
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    func setTitle(_ title: String, subtitle: String) {
        titleLabel.stringValue = title
        subtitleLabel.stringValue = subtitle
    }

    private func build(content: NSView) {
        DesktopTheme.applyPanelChrome(to: self, radius: 12)
        translatesAutoresizingMaskIntoConstraints = false

        let header = NSStackView(views: [titleLabel, subtitleLabel])
        header.orientation = .vertical
        header.alignment = .leading
        header.spacing = 3
        header.translatesAutoresizingMaskIntoConstraints = false

        content.translatesAutoresizingMaskIntoConstraints = false
        addSubview(header)
        addSubview(content)

        NSLayoutConstraint.activate([
            header.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 18),
            header.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -18),
            header.topAnchor.constraint(equalTo: topAnchor, constant: 14),

            content.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 6),
            content.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            content.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 10),
            content.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -6)
        ])
    }
}
