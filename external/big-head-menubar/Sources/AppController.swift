import AppKit

@MainActor
final class AppController: NSObject, NSApplicationDelegate, BigHeadClientDelegate, RuntimeProcessDelegate {
    private let statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let client = BigHeadClient()
    private let runtime = RuntimeProcessController()
    private let windowController = DesktopWindowController()
    private lazy var settingsController = SettingsWindowController()
    private var state = StatusState()
    private var waitingForReply = false
    private var waitingBaselineMessageCount = 0
    private var waitTimeoutWorkItem: DispatchWorkItem?

    func applicationDidFinishLaunching(_ notification: Notification) {
        client.delegate = self
        runtime.delegate = self
        settingsController.onSave = { [weak self] environment in
            self?.windowController.setConfigurationMissing(!environment.hasAPIKey)
            self?.windowController.appendLog("settings saved")
        }
        configureWindow()
        windowController.setConfigurationMissing(!EnvironmentSettingsStore.load().hasAPIKey)
        rebuildMenu()
        windowController.showWindow(nil)
        NSApp.activate(ignoringOtherApps: true)
        client.start()
        probeAndMaybeStartRuntime()
    }

    func applicationWillTerminate(_ notification: Notification) {
        client.stop()
    }

    func bigHeadClient(_ client: BigHeadClient, didChangeState state: StatusState) {
        self.state = state
        windowController.render(state: state)
        finishWaitingIfAgentReplied(in: state)
        if waitingForReply {
            windowController.restoreWaitingIndicatorIfNeeded()
        }
        rebuildMenu()
    }

    func bigHeadClient(_ client: BigHeadClient, didChangeConnection isConnected: Bool) {
        if runtime.isRunning {
            windowController.setRuntimeStatus(.managed)
        } else {
            windowController.setRuntimeStatus(isConnected ? .external : .offline)
        }
        windowController.setConnected(isConnected)
        rebuildMenu()
    }

    func bigHeadClient(_ client: BigHeadClient, didReceiveLog line: String) {
        if line.hasPrefix("send failed:") {
            finishWaiting()
        }
        windowController.appendLog(line)
    }

    func runtimeProcess(_ process: RuntimeProcessController, didLog line: String) {
        guard shouldShowRuntimeLog(line) else { return }
        windowController.appendLog(line)
    }

    func runtimeProcess(_ process: RuntimeProcessController, didChangeRunning running: Bool) {
        windowController.setRuntimeStatus(running ? .managed : .offline)
        rebuildMenu()
    }

    private func configureWindow() {
        windowController.onSend = { [weak self] text, attachments in
            self?.beginWaiting(for: text, attachments: attachments)
            self?.client.sendSpark(text, attachments: attachments)
        }
        windowController.onStartRuntime = { [weak self] in
            self?.startRuntimeFromUI()
        }
        windowController.onStopRuntime = { [weak self] in
            self?.runtime.stop()
        }
        windowController.onRefresh = { [weak self] in
            self?.client.hydrate()
            self?.client.connect()
            self?.probeRuntimeOnly()
        }
        windowController.onSettings = { [weak self] in
            self?.openSettings()
        }
        windowController.onStatusAction = { [weak self] item, action in
            self?.client.sendStatusEvent(item: item, action: action)
        }
        windowController.onDocumentReferenceAction = { [weak self] document, reference, action in
            self?.client.sendDocumentReferenceEvent(document: document, reference: reference, action: action)
        }
    }

    private func beginWaiting(for text: String, attachments: [DesktopAttachment] = []) {
        if waitingForReply {
            finishWaiting()
        }
        waitingForReply = true
        waitingBaselineMessageCount = state.messages.count
        waitTimeoutWorkItem?.cancel()
        windowController.beginWaitingForReply(userText: text, attachments: attachments)
        let work = DispatchWorkItem { [weak self] in
            Task { @MainActor in
                guard let self, self.waitingForReply else { return }
                self.waitingForReply = false
                self.waitingBaselineMessageCount = self.state.messages.count
                self.waitTimeoutWorkItem = nil
                self.windowController.markReplyStillPending()
            }
        }
        waitTimeoutWorkItem = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 90, execute: work)
    }

    private func finishWaitingIfAgentReplied(in state: StatusState) {
        guard waitingForReply else { return }
        let newMessages = state.messages.dropFirst(min(waitingBaselineMessageCount, state.messages.count))
        guard newMessages.contains(where: { $0.role == .assistant || $0.role == .error }) else {
            return
        }
        finishWaiting()
    }

    private func finishWaiting() {
        waitingForReply = false
        waitingBaselineMessageCount = state.messages.count
        waitTimeoutWorkItem?.cancel()
        waitTimeoutWorkItem = nil
        windowController.finishWaitingForReply()
    }

    private func shouldShowRuntimeLog(_ line: String) -> Bool {
        let noisyFragments = [
            "runtime: Compiling ",
            "runtime: Checking ",
            "runtime: Blocking waiting for file lock",
            "runtime: warning:",
            "runtime:   |",
            "runtime:   = ",
            "runtime:    |",
            "runtime:     |"
        ]
        if noisyFragments.contains(where: { line.contains($0) }) {
            return false
        }

        let usefulFragments = [
            "started bighead",
            "runtime data directory",
            "runtime exited",
            "failed to start runtime",
            "View server enabled",
            "Big Head Runtime",
            "Restored",
            "MiniMax_API_KEY"
        ]
        return usefulFragments.contains(where: { line.contains($0) })
    }

    private func probeAndMaybeStartRuntime() {
        guard EnvironmentSettingsStore.load().hasAPIKey else {
            windowController.appendLog("MiniMax API key missing; open Settings before starting runtime")
            return
        }
        let url = URL(string: "http://127.0.0.1:3030/health")!
        URLSession.shared.dataTask(with: url) { [weak self] _, response, _ in
            Task { @MainActor in
                let ok = (response as? HTTPURLResponse)?.statusCode == 200
                if ok {
                    self?.windowController.setRuntimeStatus(.external)
                    self?.windowController.appendLog("found existing bighead runtime")
                } else {
                    self?.windowController.setRuntimeStatus(.offline)
                    self?.windowController.appendLog("no runtime found; starting local cargo runtime")
                    self?.startRuntimeFromUI()
                }
            }
        }.resume()
    }

    private func startRuntimeFromUI() {
        let environment = EnvironmentSettingsStore.load()
        windowController.setConfigurationMissing(!environment.hasAPIKey)
        guard environment.hasAPIKey else {
            windowController.appendLog("MiniMax API key missing; open Settings before starting runtime")
            openSettings()
            return
        }

        windowController.setRuntimeStatus(.starting)
        runtime.start()
        DispatchQueue.main.asyncAfter(deadline: .now() + 3.0) { [weak self] in
            self?.client.hydrate()
            self?.client.connect()
        }
    }

    private func rebuildMenu() {
        let visible = state.items.filter { $0.state != .hidden }
        let running = visible.filter { $0.state == .running }.count
        let blocked = visible.filter { $0.state == .blocked || $0.state == .failed }.count

        if !client.isConnected {
            statusItem.button?.title = "Big Head offline"
        } else if blocked > 0 {
            statusItem.button?.title = "Big Head \(blocked) blocked"
        } else if running > 0 {
            statusItem.button?.title = "Big Head \(running) running"
        } else {
            statusItem.button?.title = state.title ?? "Big Head"
        }

        let menu = NSMenu()
        let openItem = NSMenuItem(title: "Open Big Head", action: #selector(openWindow), keyEquivalent: "o")
        openItem.target = self
        menu.addItem(openItem)
        menu.addItem(.separator())

        if visible.isEmpty {
            let empty = NSMenuItem(title: "No active items", action: nil, keyEquivalent: "")
            empty.isEnabled = false
            menu.addItem(empty)
        } else {
            for item in visible.prefix(8) {
                let menuItem = NSMenuItem()
                menuItem.view = StatusMenuItemView(item: item) { [weak self] item, action in
                    self?.client.sendStatusEvent(item: item, action: action)
                }
                menu.addItem(menuItem)
            }
        }

        menu.addItem(.separator())
        let startStop = NSMenuItem(
            title: runtime.isRunning ? "Stop Runtime" : (client.isConnected ? "External Runtime" : "Start Runtime"),
            action: #selector(toggleRuntime),
            keyEquivalent: ""
        )
        startStop.target = self
        startStop.isEnabled = runtime.isRunning || !client.isConnected
        menu.addItem(startStop)

        let refreshItem = NSMenuItem(title: "Refresh", action: #selector(refresh), keyEquivalent: "r")
        refreshItem.target = self
        menu.addItem(refreshItem)

        let settingsItem = NSMenuItem(title: "Settings...", action: #selector(settings), keyEquivalent: ",")
        settingsItem.target = self
        menu.addItem(settingsItem)

        let quitItem = NSMenuItem(title: "Quit", action: #selector(quit), keyEquivalent: "q")
        quitItem.target = self
        menu.addItem(quitItem)
        statusItem.menu = menu
    }

    @objc private func openWindow() {
        windowController.showWindow(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    @objc private func toggleRuntime() {
        if runtime.isRunning {
            runtime.stop()
        } else if !client.isConnected {
            startRuntimeFromUI()
        }
    }

    @objc private func refresh() {
        client.hydrate()
        client.connect()
        probeRuntimeOnly()
    }

    @objc private func settings() {
        openSettings()
    }

    private func openSettings() {
        settingsController.reload()
        settingsController.showWindow(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    private func probeRuntimeOnly() {
        let url = URL(string: "http://127.0.0.1:3030/health")!
        URLSession.shared.dataTask(with: url) { [weak self] _, response, _ in
            Task { @MainActor in
                guard let self else { return }
                let ok = (response as? HTTPURLResponse)?.statusCode == 200
                if self.runtime.isRunning {
                    self.windowController.setRuntimeStatus(.managed)
                } else {
                    self.windowController.setRuntimeStatus(ok ? .external : .offline)
                }
                self.rebuildMenu()
            }
        }.resume()
    }

    @objc private func quit() {
        NSApp.terminate(nil)
    }
}
