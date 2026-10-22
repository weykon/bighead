import Foundation

struct StatusState: Codable {
    var title: String?
    var subtitle: String?
    var summary: String?
    var items: [StatusItem]
    var messages: [ChatMessage]
    var documents: [StatusDocument]

    init(
        title: String? = "Big Head",
        subtitle: String? = nil,
        summary: String? = nil,
        items: [StatusItem] = [],
        messages: [ChatMessage] = [],
        documents: [StatusDocument] = []
    ) {
        self.title = title
        self.subtitle = subtitle
        self.summary = summary
        self.items = items
        self.messages = messages
        self.documents = documents
    }

    enum CodingKeys: String, CodingKey {
        case title, subtitle, summary, items, messages, documents
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        title = try container.decodeIfPresent(String.self, forKey: .title)
        subtitle = try container.decodeIfPresent(String.self, forKey: .subtitle)
        summary = try container.decodeIfPresent(String.self, forKey: .summary)
        items = try container.decodeIfPresent([StatusItem].self, forKey: .items) ?? []
        messages = try container.decodeIfPresent([ChatMessage].self, forKey: .messages) ?? []
        documents = try container.decodeIfPresent([StatusDocument].self, forKey: .documents) ?? []
    }
}

struct StatusDocument: Codable, Equatable {
    var id: String
    var title: String
    var subtitle: String?
    var text: String
    var kind: String?
    var references: [StatusDocumentReference]
    var updatedAt: UInt64?

    enum CodingKeys: String, CodingKey {
        case id, title, subtitle, text, kind, references
        case updatedAt = "updated_at"
    }

    init(
        id: String,
        title: String,
        subtitle: String? = nil,
        text: String,
        kind: String? = nil,
        references: [StatusDocumentReference] = [],
        updatedAt: UInt64? = nil
    ) {
        self.id = id
        self.title = title
        self.subtitle = subtitle
        self.text = text
        self.kind = kind
        self.references = references
        self.updatedAt = updatedAt
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(String.self, forKey: .id)
        title = try container.decode(String.self, forKey: .title)
        subtitle = try container.decodeIfPresent(String.self, forKey: .subtitle)
        text = try container.decode(String.self, forKey: .text)
        kind = try container.decodeIfPresent(String.self, forKey: .kind)
        references = try container.decodeIfPresent([StatusDocumentReference].self, forKey: .references) ?? []
        updatedAt = try container.decodeIfPresent(UInt64.self, forKey: .updatedAt)
    }
}

struct StatusDocumentReference: Codable, Equatable {
    var kind: String
    var id: String
    var title: String
    var description: String?
}

struct ChatMessage: Codable, Equatable {
    var id: String
    var role: ChatRole
    var text: String
    var createdAt: UInt64?

    enum CodingKeys: String, CodingKey {
        case id, role, text
        case createdAt = "created_at"
    }
}

enum ChatRole: String, Codable {
    case user
    case assistant
    case system
    case error
}

struct StatusItem: Codable {
    var id: String
    var title: String
    var state: ItemState
    var control: ItemControl?
    var description: String?
    var progress: Float?
    var checked: Bool?
    var group: String?
    var priority: Int?
    var updatedAt: UInt64?

    enum CodingKeys: String, CodingKey {
        case id, title, state, control, description, progress, checked, group, priority
        case updatedAt = "updated_at"
    }
}

enum ItemState: String, Codable {
    case pending
    case running
    case blocked
    case done
    case failed
    case hidden
}

enum ItemControl: String, Codable {
    case item
    case checkbox
    case progress
}

enum StatusCommand {
    case setState(StatusState)
    case upsertItem(StatusItem)
    case removeItem(String)
    case appendMessage(ChatMessage)
    case upsertDocument(StatusDocument)
    case removeDocument(String)
    case clear
}

final class AnyStatusCommand: Decodable {
    let command: StatusCommand

    enum CodingKeys: String, CodingKey {
        case op, state, item, id, message, document
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let op = try container.decode(String.self, forKey: .op)
        switch op {
        case "set_state":
            command = .setState(try container.decode(StatusState.self, forKey: .state))
        case "upsert_item":
            command = .upsertItem(try container.decode(StatusItem.self, forKey: .item))
        case "remove_item":
            command = .removeItem(try container.decode(String.self, forKey: .id))
        case "append_message":
            command = .appendMessage(try container.decode(ChatMessage.self, forKey: .message))
        case "upsert_document":
            command = .upsertDocument(try container.decode(StatusDocument.self, forKey: .document))
        case "remove_document":
            command = .removeDocument(try container.decode(String.self, forKey: .id))
        case "clear":
            command = .clear
        default:
            throw DecodingError.dataCorruptedError(
                forKey: .op,
                in: container,
                debugDescription: "Unknown status op: \(op)"
            )
        }
    }
}
