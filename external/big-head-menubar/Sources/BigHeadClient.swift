import Foundation

@MainActor
protocol BigHeadClientDelegate: AnyObject {
    func bigHeadClient(_ client: BigHeadClient, didChangeState state: StatusState)
    func bigHeadClient(_ client: BigHeadClient, didChangeConnection isConnected: Bool)
    func bigHeadClient(_ client: BigHeadClient, didReceiveLog line: String)
}

@MainActor
final class BigHeadClient {
    weak var delegate: BigHeadClientDelegate?

    private(set) var state = StatusState()
    private(set) var isConnected = false

    private let baseURL = URL(string: "http://127.0.0.1:3030")!
    private let wsURL = URL(string: "ws://127.0.0.1:3030/status/ws")!
    private lazy var session = URLSession(configuration: .default)
    private var wsTask: URLSessionWebSocketTask?
    private var reconnectWorkItem: DispatchWorkItem?

    func start() {
        hydrate()
        connect()
    }

    func stop() {
        reconnectWorkItem?.cancel()
        reconnectWorkItem = nil
        wsTask?.cancel(with: .goingAway, reason: nil)
        wsTask = nil
        setConnected(false)
    }

    func hydrate() {
        let url = baseURL.appendingPathComponent("api/status")
        URLSession.shared.dataTask(with: url) { [weak self] data, response, error in
            Task { @MainActor in
                guard let self else { return }
                if let error {
                    self.delegate?.bigHeadClient(self, didReceiveLog: "status hydrate failed: \(error.localizedDescription)")
                    self.setConnected(false)
                    return
                }
                if let code = (response as? HTTPURLResponse)?.statusCode, code >= 400 {
                    self.delegate?.bigHeadClient(self, didReceiveLog: "status hydrate failed: HTTP \(code)")
                    self.setConnected(false)
                    return
                }
                guard let data else { return }
                do {
                    self.state = try JSONDecoder().decode(StatusState.self, from: data)
                    self.sortItems()
                    self.delegate?.bigHeadClient(self, didChangeState: self.state)
                    self.setConnected(true)
                } catch {
                    self.delegate?.bigHeadClient(self, didReceiveLog: "status decode failed: \(error)")
                }
            }
        }.resume()
    }

    func connect() {
        reconnectWorkItem?.cancel()
        wsTask?.cancel()
        let task = session.webSocketTask(with: wsURL)
        wsTask = task
        task.resume()
        setConnected(true)
        receiveNext()
    }

    func sendSpark(_ text: String, attachments: [DesktopAttachment] = []) {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty || !attachments.isEmpty else { return }
        var payload: [String: Any] = [
            "type": "spark",
            "text": trimmed,
            "source_client": "macos_app",
            "input_surface": "desktop_command_bar"
        ]
        if !attachments.isEmpty {
            payload["attachments"] = attachments.map(\.dictionary)
        }
        sendJSON(payload)
    }

    func sendStatusEvent(item: StatusItem, action: String) {
        sendJSON([
            "type": "status_event",
            "item_id": item.id,
            "action": action,
            "title": item.title,
            "source_client": "macos_app"
        ])
    }

    func sendDocumentReferenceEvent(
        document: StatusDocument,
        reference: StatusDocumentReference,
        action: String
    ) {
        sendJSON([
            "type": "status_event",
            "item_id": "\(document.id):\(reference.kind):\(reference.id)",
            "action": action,
            "title": reference.title,
            "document_id": document.id,
            "document_title": document.title,
            "reference_kind": reference.kind,
            "reference_id": reference.id,
            "reference_title": reference.title,
            "source_client": "macos_app"
        ])
    }

    private func sendJSON(_ payload: [String: Any]) {
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let text = String(data: data, encoding: .utf8)
        else {
            return
        }

        if wsTask == nil {
            connect()
        }
        wsTask?.send(.string(text)) { [weak self] error in
            guard let self else { return }
            if let error {
                Task { @MainActor in
                    self.delegate?.bigHeadClient(self, didReceiveLog: "send failed: \(error.localizedDescription)")
                    self.scheduleReconnect()
                }
            }
        }
    }

    private func receiveNext() {
        wsTask?.receive { [weak self] result in
            guard let self else { return }
            Task { @MainActor in
                switch result {
                case .success(let message):
                    if case .string(let text) = message {
                        self.apply(text)
                    }
                    self.receiveNext()
                case .failure(let error):
                    self.delegate?.bigHeadClient(self, didReceiveLog: "status websocket closed: \(error.localizedDescription)")
                    self.scheduleReconnect()
                }
            }
        }
    }

    private func scheduleReconnect() {
        wsTask?.cancel()
        wsTask = nil
        setConnected(false)
        reconnectWorkItem?.cancel()
        let work = DispatchWorkItem { [weak self] in
            Task { @MainActor in
                self?.hydrate()
                self?.connect()
            }
        }
        reconnectWorkItem = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 2.0, execute: work)
    }

    private func apply(_ text: String) {
        guard let data = text.data(using: .utf8),
              let decoded = try? JSONDecoder().decode(AnyStatusCommand.self, from: data)
        else {
            return
        }

        switch decoded.command {
        case .setState(let next):
            state = next
        case .upsertItem(let item):
            if let idx = state.items.firstIndex(where: { $0.id == item.id }) {
                state.items[idx] = item
            } else {
                state.items.append(item)
            }
        case .removeItem(let id):
            state.items.removeAll { $0.id == id }
        case .appendMessage(let message):
            state.messages.append(message)
            if state.messages.count > 80 {
                state.messages.removeFirst(state.messages.count - 80)
            }
        case .upsertDocument(let document):
            if let idx = state.documents.firstIndex(where: { $0.id == document.id }) {
                state.documents[idx] = document
            } else {
                state.documents.append(document)
            }
            sortDocuments()
        case .removeDocument(let id):
            state.documents.removeAll { $0.id == id }
        case .clear:
            state = StatusState()
        }

        sortItems()
        delegate?.bigHeadClient(self, didChangeState: state)
    }

    private func sortItems() {
        state.items.sort { left, right in
            let lp = left.priority ?? 0
            let rp = right.priority ?? 0
            if lp != rp {
                return lp > rp
            }
            return left.title < right.title
        }
    }

    private func sortDocuments() {
        state.documents.sort { left, right in
            (left.updatedAt ?? 0) > (right.updatedAt ?? 0)
        }
    }

    private func setConnected(_ next: Bool) {
        guard isConnected != next else { return }
        isConnected = next
        delegate?.bigHeadClient(self, didChangeConnection: next)
    }
}
