import AppKit

struct DesktopAttachment: Codable, Equatable {
    var id: String
    var name: String
    var path: String
    var mimeType: String
    var byteCount: UInt64?

    enum CodingKeys: String, CodingKey {
        case id, name, path
        case mimeType = "mime_type"
        case byteCount = "byte_count"
    }

    var dictionary: [String: Any] {
        var payload: [String: Any] = [
            "id": id,
            "name": name,
            "path": path,
            "mime_type": mimeType
        ]
        if let byteCount {
            payload["byte_count"] = byteCount
        }
        return payload
    }
}

enum AttachmentStore {
    static func attachment(fromFileURL url: URL) -> DesktopAttachment {
        DesktopAttachment(
            id: stableID(for: url),
            name: url.lastPathComponent,
            path: url.path,
            mimeType: mimeType(for: url),
            byteCount: fileSize(url)
        )
    }

    static func saveImageFromPasteboard(_ pasteboard: NSPasteboard = .general) -> DesktopAttachment? {
        guard let image = NSImage(pasteboard: pasteboard),
              let tiff = image.tiffRepresentation,
              let rep = NSBitmapImageRep(data: tiff),
              let png = rep.representation(using: .png, properties: [:])
        else {
            return nil
        }

        let directory = attachmentsDirectory()
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let filename = "clipboard-\(Int(Date().timeIntervalSince1970))-\(UUID().uuidString.prefix(8)).png"
        let url = directory.appendingPathComponent(filename)
        do {
            try png.write(to: url)
            return attachment(fromFileURL: url)
        } catch {
            return nil
        }
    }

    private static func attachmentsDirectory() -> URL {
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? FileManager.default.homeDirectoryForCurrentUser
        return support.appendingPathComponent("BigHead", isDirectory: true)
            .appendingPathComponent("Attachments", isDirectory: true)
    }

    private static func stableID(for url: URL) -> String {
        "att-\(abs(url.path.hashValue))"
    }

    private static func fileSize(_ url: URL) -> UInt64? {
        guard let values = try? url.resourceValues(forKeys: [.fileSizeKey]),
              let size = values.fileSize
        else {
            return nil
        }
        return UInt64(size)
    }

    private static func mimeType(for url: URL) -> String {
        switch url.pathExtension.lowercased() {
        case "png": return "image/png"
        case "jpg", "jpeg": return "image/jpeg"
        case "gif": return "image/gif"
        case "webp": return "image/webp"
        case "pdf": return "application/pdf"
        case "txt", "md": return "text/plain"
        case "json": return "application/json"
        case "csv": return "text/csv"
        default: return "application/octet-stream"
        }
    }
}
