import Foundation
import CryptoKit
import Observation
import SwiftUI
import UIKit
@preconcurrency import WebRTC

enum StreamState: String {
    case idle = "Idle"
    case connecting = "Connecting"
    case connected = "Connected"
    case disconnected = "Disconnected"
    case failed = "Failed"
}

enum HardwareButtonPhase: String {
    case down
    case up
}

private struct HardwareButtonControlPayload: Encodable {
    let button: String
    let durationMs: Int?
    let phase: String?
    let usagePage: Int?
    let usage: Int?
}

private struct KeyControlPayload: Encodable {
    let keyCode: Int
    let modifiers: Int
}

private struct EmptyControlPayload: Encodable {}

private struct ChromeAssets {
    var profile: ChromeProfile?
    var image: UIImage?
    var screenMask: UIImage?

    var isEmpty: Bool {
        profile == nil && image == nil && screenMask == nil
    }
}

@MainActor
@Observable
final class AppModel {
    let discovery = SimDeckDiscovery()
    private static let savedEndpointsKey = "savedEndpoints"
    private static let legacyRecentEndpointsKey = "recentEndpoints"
    private static let selectedEndpointKey = "selectedEndpoint"
    private static let streamConfigKey = "streamConfig"
    private static let hapticsEnabledKey = "hapticsEnabled"
    private static let lastFrameCacheDirectoryName = "LastStreamFrames"

    var endpoint: SimDeckEndpoint?
    var savedEndpoints: [SimDeckEndpoint] = []
    var simulators: [SimulatorMetadata] = []
    var selectedSimulatorID: String?
    var manualAddress = ""
    var manualToken = ""
    var pairingCode = ""
    var authEndpoint: SimDeckEndpoint?
    var status = "Ready"
    var isBusy = false
    var streamState: StreamState = .idle
    var videoSize: CGSize = .zero
    var chromeProfile: ChromeProfile?
    var chromeImage: UIImage?
    var chromeScreenMask: UIImage?
    var streamDiagnostics = StreamDiagnostics()
    var bootingSimulatorID: String?
    var streamDisplayToken = 0
    var hasCurrentStreamFrame = false
    var lastStreamFrame: UIImage?
    var streamConfig = AppModel.loadStreamConfig()
    var hapticsEnabled = AppModel.loadHapticsEnabled() {
        didSet {
            UserDefaults.standard.set(hapticsEnabled, forKey: Self.hapticsEnabledKey)
        }
    }

    @ObservationIgnored private var streamClient: WebRTCClient?
    @ObservationIgnored private var hasAutoConnected = false
    @ObservationIgnored private var isAutoConnecting = false
    @ObservationIgnored private var streamRequestGeneration = 0
    @ObservationIgnored private var reconnectTask: Task<Void, Never>?
    @ObservationIgnored private var lastReconnectStartedAt = Date.distantPast
    @ObservationIgnored private var chromeCache: [String: ChromeAssets] = [:]
    @ObservationIgnored private var chromeCacheOrder: [String] = []
    @ObservationIgnored private var lastStreamFrameKey: String?
    private static let chromeCacheLimit = 24

    init() {
        discovery.onEndpoint = { [weak self] endpoint in
            Task { @MainActor in
                await self?.autoConnectIfNeeded(endpoint)
            }
        }
    }

    var selectedSimulator: SimulatorMetadata? {
        simulators.first { $0.udid == selectedSimulatorID }
    }

    var currentStreamClient: WebRTCClient? { streamClient }
    var canStopStream: Bool {
        streamState != .idle || streamClient != nil
    }

    var isSelectedSimulatorBooting: Bool {
        bootingSimulatorID == selectedSimulatorID
    }

    var availableEndpoints: [SimDeckEndpoint] {
        savedEndpoints + automaticEndpoints
    }

    var automaticEndpoints: [SimDeckEndpoint] {
        var endpoints = discovery.endpoints.filter { discovered in
            !savedEndpoints.contains { endpointsRepresentSameServer($0, discovered) }
        }
        if let endpoint,
           !savedEndpoints.contains(where: { endpointsRepresentSameServer($0, endpoint) }),
           !endpoints.contains(where: { endpointsRepresentSameServer($0, endpoint) }) {
            endpoints.insert(endpoint, at: 0)
        }
        return endpoints
    }

    var selectedEndpointTitle: String {
        endpoint?.name ?? "Select Server"
    }

    var selectedEndpointSubtitle: String {
        endpoint?.baseURL.host(percentEncoded: false) ?? "No SimDeck connected"
    }

    var streamNavigationSubtitle: String {
        endpoint?.name ?? "No SimDeck connected"
    }

    func start() {
        loadSavedEndpoints()
        if let lastSelectedEndpoint = loadSelectedEndpoint() {
            isAutoConnecting = true
            discovery.upsert(lastSelectedEndpoint)
            Task {
                let connected = await connect(
                    lastSelectedEndpoint,
                    autoStart: false,
                    saveEndpoint: false,
                    presentPairingOnAuth: false
                )
                isAutoConnecting = false
                if connected {
                    hasAutoConnected = true
                } else {
                    await autoConnectToAvailableEndpointIfNeeded()
                }
            }
        }
        discovery.start()
    }

    @discardableResult
    func connectManual() async -> Bool {
        guard let endpoint = StudioLinkResolver.endpointFromAddress(manualAddress, token: manualToken) else {
            status = "Enter a SimDeck URL or host."
            return false
        }
        return await connect(endpoint, autoStart: false, saveEndpoint: true)
    }

    func handle(url: URL) {
        guard let route = StudioLinkResolver.route(for: url) else {
            status = "Unsupported link."
            return
        }
        switch route {
        case let .endpoint(endpoint, autoStart):
            Task { await connect(endpoint, autoStart: autoStart, saveEndpoint: true) }
        case let .pairing(link, autoStart):
            Task { await pair(link, autoStart: autoStart) }
        }
    }

    @discardableResult
    func connect(
        _ endpoint: SimDeckEndpoint,
        autoStart: Bool,
        saveEndpoint: Bool = false,
        presentPairingOnAuth: Bool = true
    ) async -> Bool {
        let connectionEndpoint = endpointWithReusableToken(endpoint)
        isBusy = true
        status = "Connecting to \(connectionEndpoint.name)"
        defer { isBusy = false }

        var pendingAuthEndpoint: SimDeckEndpoint?
        var lastError: Error?
        for candidate in connectionCandidates(for: connectionEndpoint) {
            do {
                let api = SimDeckAPI(endpoint: candidate)
                let health = try await api.health()
                var resolvedCandidate = candidate
                resolvedCandidate.serverID = health.serverId ?? resolvedCandidate.serverID
                resolvedCandidate.alternateBaseURLs = uniquedURLs(
                    resolvedCandidate.alternateBaseURLs + alternateURLs(from: health, fallbackPort: normalizedPort(for: resolvedCandidate.baseURL))
                ).filter { $0 != resolvedCandidate.baseURL }
                let simulators = try await SimDeckAPI(endpoint: resolvedCandidate).simulators()
                stopStream()
                self.endpoint = resolvedCandidate
                self.authEndpoint = nil
                self.simulators = simulators
                selectedSimulatorID = autoStart
                    ? resolvedCandidate.preferredSimulatorID
                        ?? simulators.first(where: \.isBooted)?.udid
                        ?? simulators.first?.udid
                    : resolvedCandidate.preferredSimulatorID
                if saveEndpoint {
                    saveUserEndpoint(resolvedCandidate)
                }
                saveSelectedEndpoint(resolvedCandidate)
                status = simulators.isEmpty ? "Connected. No simulators found." : "Connected."
                hapticSuccess()
                if autoStart, selectedSimulatorID != nil {
                    await prepareSelectedSimulator()
                }
                return true
            } catch SimDeckAPIError.authRequired {
                var pendingEndpoint = candidate
                pendingEndpoint.requiresPairing = true
                pendingAuthEndpoint = pendingEndpoint
                discovery.upsert(pendingEndpoint)
                lastError = SimDeckAPIError.authRequired
            } catch {
                lastError = error
            }
        }

        if let pendingAuthEndpoint {
            status = "Pairing required."
            hapticWarning()
            guard presentPairingOnAuth else {
                return false
            }
            self.endpoint = pendingAuthEndpoint
            self.authEndpoint = pendingAuthEndpoint
            self.simulators = []
            self.selectedSimulatorID = nil
            manualAddress = pendingAuthEndpoint.baseURL.absoluteString
            manualToken = pendingAuthEndpoint.token ?? ""
            saveSelectedEndpoint(pendingAuthEndpoint)
            return false
        }

        if let lastError {
            status = lastError.localizedDescription
            hapticWarning()
            return false
        }

        status = "Unable to connect."
        hapticWarning()
        return false

    }

    @discardableResult
    func pair() async -> Bool {
        guard let authEndpoint else { return false }
        return await pair(endpoint: authEndpoint, code: pairingCode, alternateEndpoints: [], autoStart: false)
    }

    @discardableResult
    func pair(_ link: SimDeckPairingLink, autoStart: Bool) async -> Bool {
        let candidates = uniquedByBaseURL([link.endpoint] + link.alternateEndpoints)
        if let token = link.endpoint.token?.nilIfBlank {
            savePairedEndpoints(primary: link.endpoint, alternates: link.alternateEndpoints, token: token)
            for candidate in candidates {
                var pairedEndpoint = candidate
                pairedEndpoint.token = token
                if await connect(pairedEndpoint, autoStart: autoStart, saveEndpoint: true) {
                    return true
                }
            }
            return false
        }
        guard let code = link.pairingCode?.nilIfBlank else {
            authEndpoint = link.endpoint
            pairingCode = ""
            status = "Pairing code missing."
            hapticWarning()
            return false
        }
        for candidate in candidates {
            let alternates = candidates.filter { $0.baseURL != candidate.baseURL }
            if await pair(endpoint: candidate, code: code, alternateEndpoints: alternates, autoStart: autoStart) {
                return true
            }
        }
        return false
    }

    @discardableResult
    private func pair(
        endpoint authEndpoint: SimDeckEndpoint,
        code: String,
        alternateEndpoints: [SimDeckEndpoint],
        autoStart: Bool
    ) async -> Bool {
        isBusy = true
        defer { isBusy = false }
        do {
            let token = try await SimDeckAPI(endpoint: authEndpoint).pair(code: code)
            var pairedEndpoint = authEndpoint
            if let token {
                pairedEndpoint.token = token
                manualToken = token
                savePairedEndpoints(primary: pairedEndpoint, alternates: alternateEndpoints, token: token)
            }
            pairingCode = ""
            let connected = await connect(pairedEndpoint, autoStart: autoStart, saveEndpoint: true)
            if connected {
                hapticSuccess()
            }
            return connected
        } catch {
            status = error.localizedDescription
            hapticWarning()
            return false
        }
    }

    @discardableResult
    func useToken() async -> Bool {
        guard var authEndpoint else { return false }
        authEndpoint.token = manualToken.nilIfBlank
        let connected = await connect(authEndpoint, autoStart: false, saveEndpoint: true)
        if connected {
            hapticSuccess()
        }
        return connected
    }

    func handleScannedPairingPayload(_ value: String) {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        if let url = URL(string: trimmed), let route = StudioLinkResolver.route(for: url) {
            switch route {
            case let .pairing(link, autoStart):
                Task { await pair(link, autoStart: autoStart) }
            case let .endpoint(endpoint, autoStart):
                Task { await connect(endpoint, autoStart: autoStart, saveEndpoint: true) }
            }
            return
        }
        let digits = trimmed.filter(\.isNumber)
        if !digits.isEmpty {
            pairingCode = String(digits.prefix(6))
            hapticSelection()
        } else {
            status = "That QR code is not a SimDeck pairing link."
            hapticWarning()
        }
    }

    func refreshSimulators() async {
        guard let endpoint else { return }
        do {
            simulators = try await SimDeckAPI(endpoint: endpoint).simulators()
            if selectedSimulatorID == nil {
                selectedSimulatorID = simulators.first(where: \.isBooted)?.udid ?? simulators.first?.udid
            }
            status = "Updated."
            hapticSelection()
        } catch {
            status = error.localizedDescription
            hapticWarning()
        }
    }

    func selectSimulator(_ udid: String?) {
        guard selectedSimulatorID != udid else { return }
        hapticSelection()
        selectedSimulatorID = udid
        resetStreamPresentation()
        guard endpoint != nil, udid != nil else {
            stopStream()
            return
        }
        Task { await prepareSelectedSimulator() }
    }

    func prepareSelectedSimulator() async {
        guard let selectedSimulator else { return }
        if selectedSimulator.isBooted {
            await startStream()
        } else {
            await loadSelectedSimulatorChrome()
        }
    }

    @discardableResult
    func startStream(automaticReconnect: Bool = false) async -> Bool {
        guard let endpoint, let selectedSimulatorID else { return false }
        guard selectedSimulator?.isBooted == true else {
            await loadSelectedSimulatorChrome()
            return false
        }
        streamRequestGeneration += 1
        let generation = streamRequestGeneration
        stopCurrentStream(resetState: false)
        resetStreamPresentation()
        streamState = .connecting
        status = automaticReconnect ? "Reconnecting WebRTC." : "Starting WebRTC."
        do {
            let api = SimDeckAPI(endpoint: endpoint)
            async let health = try api.health(timeout: 8)
            let client = WebRTCClient()
            client.onConnectionState = { [weak self] state in
                Task { @MainActor in
                    guard self?.isCurrentStreamRequest(generation, simulatorID: selectedSimulatorID) == true else { return }
                    self?.streamState = StreamState(peerState: state)
                }
            }
            client.onVideoSize = { [weak self] size in
                Task { @MainActor in
                    guard self?.isCurrentStreamRequest(generation, simulatorID: selectedSimulatorID) == true else { return }
                    if self?.videoSize != size {
                        self?.videoSize = size
                    }
                }
            }
            client.onDiagnostics = { [weak self] diagnostics in
                Task { @MainActor in
                    guard self?.isCurrentStreamRequest(generation, simulatorID: selectedSimulatorID) == true else { return }
                    self?.streamDiagnostics = diagnostics
                }
            }
            let clientToken = ObjectIdentifier(client)
            client.onReconnectNeeded = { [weak self] reason in
                Task { @MainActor in
                    guard let self,
                          let activeClient = self.streamClient,
                          self.isCurrentStreamRequest(generation, simulatorID: selectedSimulatorID),
                          ObjectIdentifier(activeClient) == clientToken else { return }
                    self.scheduleStreamReconnect(reason: reason)
                }
            }
            let loadedChromeAssets = await chromeAssets(api: api, endpoint: endpoint, simulatorID: selectedSimulatorID)
            guard isCurrentStreamRequest(generation, simulatorID: selectedSimulatorID) else {
                client.disconnect()
                return false
            }
            applyChromeAssets(loadedChromeAssets)
            let loadedHealth = try await health
            guard isCurrentStreamRequest(generation, simulatorID: selectedSimulatorID) else {
                client.disconnect()
                return false
            }
            let answer = try await client.connect(
                api: api,
                simulatorID: selectedSimulatorID,
                health: loadedHealth,
                streamConfig: streamConfig
            )
            guard isCurrentStreamRequest(generation, simulatorID: selectedSimulatorID) else {
                client.disconnect()
                return false
            }
            streamClient = client
            if let video = answer.video, video.width > 0, video.height > 0 {
                videoSize = CGSize(width: video.width, height: video.height)
            }
            status = "WebRTC connected."
            if !automaticReconnect {
                hapticSuccess()
            }
            return true
        } catch {
            guard streamRequestGeneration == generation else { return false }
            streamState = .failed
            status = error.localizedDescription
            if !automaticReconnect {
                hapticWarning()
                scheduleStreamReconnect(reason: "connect-failed")
            }
            stopCurrentStream(resetState: false)
            return false
        }
    }

    func loadSelectedSimulatorChrome() async {
        guard let endpoint, let selectedSimulatorID else { return }
        streamRequestGeneration += 1
        let generation = streamRequestGeneration
        stopCurrentStream(resetState: false)
        resetStreamPresentation()
        streamState = .idle
        status = "Loading device chrome."

        let api = SimDeckAPI(endpoint: endpoint)
        let loadedChromeAssets = await chromeAssets(api: api, endpoint: endpoint, simulatorID: selectedSimulatorID)
        guard isCurrentStreamRequest(generation, simulatorID: selectedSimulatorID) else { return }
        applyChromeAssets(loadedChromeAssets)
        status = selectedSimulator?.isBooted == true ? "Ready." : "Ready to boot."
    }

    func bootSelectedSimulator() async {
        guard let endpoint, let selectedSimulatorID, let selectedSimulator else { return }
        guard !selectedSimulator.isBooted else {
            await startStream()
            return
        }
        bootingSimulatorID = selectedSimulatorID
        streamState = .connecting
        status = "Booting \(selectedSimulator.name)."
        hapticSelection()
        do {
            let api = SimDeckAPI(endpoint: endpoint)
            try await api.bootSimulator(udid: selectedSimulatorID)
            simulators = try await api.simulators()
            status = "Booted."
            bootingSimulatorID = nil
            hapticSuccess()
            await startStream()
        } catch {
            streamState = .failed
            status = error.localizedDescription
            bootingSimulatorID = nil
            hapticWarning()
        }
    }

    func stopStream() {
        streamRequestGeneration += 1
        reconnectTask?.cancel()
        reconnectTask = nil
        bootingSimulatorID = nil
        stopCurrentStream(resetState: true)
        hapticSelection()
    }

    @discardableResult
    func createSimulator(_ request: CreateSimulatorRequest) async -> Bool {
        guard let endpoint else {
            status = "Select a SimDeck server first."
            hapticWarning()
            return false
        }
        isBusy = true
        status = "Creating simulator."
        defer { isBusy = false }
        do {
            let api = SimDeckAPI(endpoint: endpoint)
            let response = try await api.createSimulator(request)
            let refreshed = (try? await api.simulators()) ?? []
            if refreshed.isEmpty {
                upsertSimulator(response.simulator)
                if let pairedWatchSimulator = response.pairedWatchSimulator {
                    upsertSimulator(pairedWatchSimulator)
                }
            } else {
                simulators = refreshed
            }
            selectedSimulatorID = response.simulator.udid
            resetStreamPresentation()
            status = "Created \(response.simulator.name)."
            hapticSuccess()
            await prepareSelectedSimulator()
            return true
        } catch {
            status = error.localizedDescription
            hapticWarning()
            return false
        }
    }

    private func stopCurrentStream(resetState: Bool) {
        streamClient?.disconnect()
        streamClient = nil
        if resetState {
            streamState = .idle
            resetStreamPresentation()
        }
    }

    func sendTouch(location: CGPoint, in screenFrame: CGRect, phase: String) {
        guard let point = normalizedTouchPoint(location: location, in: screenFrame) else { return }
        streamClient?.sendTouch(x: Double(point.x), y: Double(point.y), phase: phase)
    }

    func sendEdgeTouch(location: CGPoint, in screenFrame: CGRect, phase: String, edge: String) {
        guard let point = normalizedTouchPoint(location: location, in: screenFrame) else { return }
        streamClient?.sendEdgeTouch(x: Double(point.x), y: Double(point.y), phase: phase, edge: edge)
    }

    func normalizedTouchPoint(location: CGPoint, in screenFrame: CGRect) -> CGPoint? {
        guard screenFrame.width > 0, screenFrame.height > 0 else { return nil }
        let x = ((location.x - screenFrame.minX) / screenFrame.width).clamped(to: 0...1)
        let y = ((location.y - screenFrame.minY) / screenFrame.height).clamped(to: 0...1)
        return CGPoint(x: x, y: y)
    }

    func markStreamFrameRendered(displayToken: Int) {
        guard displayToken == streamDisplayToken else { return }
        hasCurrentStreamFrame = true
    }

    func updateLastStreamFrame(_ image: UIImage, displayToken: Int) {
        guard displayToken == streamDisplayToken,
              let endpoint,
              let selectedSimulatorID else {
            return
        }
        lastStreamFrameKey = lastFrameCacheKey(endpoint: endpoint, simulatorID: selectedSimulatorID)
        lastStreamFrame = image
        if videoSize == .zero {
            videoSize = image.size
        }
        persistLastStreamFrame(image, endpoint: endpoint, simulatorID: selectedSimulatorID)
    }

    func sendTouch(x: Double, y: Double, phase: String) {
        streamClient?.sendTouch(x: x, y: y, phase: phase)
    }

    func sendKeyboardText(_ text: String) {
        for character in text {
            guard let key = Self.keyControl(for: character) else {
                status = "Unsupported keyboard input."
                hapticWarning()
                continue
            }
            sendKey(keyCode: key.keyCode, modifiers: key.modifiers)
        }
    }

    func sendKeyboardBackspace() {
        sendKey(keyCode: 42, modifiers: 0)
    }

    func dismissSimulatorKeyboard() {
        let sent = streamClient?.dismissSimulatorKeyboard() ?? false
        guard !sent else { return }
        Task {
            await postDismissKeyboard()
        }
    }

    @discardableResult
    func sendKey(keyCode: Int, modifiers: Int = 0) -> Bool {
        guard selectedSimulatorID != nil, (0...65_535).contains(keyCode) else { return false }
        let sent = streamClient?.sendKey(keyCode: keyCode, modifiers: modifiers) ?? false
        guard !sent else { return true }
        Task {
            await postKey(keyCode: keyCode, modifiers: modifiers)
        }
        return false
    }

    func sendHome() {
        tapHardwareButton(named: "home")
    }

    func sendAppSwitcher() {
        hapticImpact()
        streamClient?.sendAppSwitcher()
    }

    func sendLock() {
        tapHardwareButton(named: "power")
    }

    func sendHardwareButton(named button: String, phase: HardwareButtonPhase, usagePage: Int? = nil, usage: Int? = nil) {
        guard selectedSimulatorID != nil else { return }
        switch phase {
        case .down:
            hapticImpact()
        case .up:
            hapticSelection()
        }
        let sent = streamClient?.sendHardwareButton(
            button: button,
            phase: phase.rawValue,
            usagePage: usagePage,
            usage: usage
        ) ?? false
        guard !sent else { return }
        Task {
            await postHardwareButton(
                named: button,
                durationMs: nil,
                phase: phase,
                usagePage: usagePage,
                usage: usage
            )
        }
    }

    func tapHardwareButton(named button: String, usagePage: Int? = nil, usage: Int? = nil, durationMs: Int = 80) {
        guard selectedSimulatorID != nil else { return }
        hapticImpact()
        let sent = streamClient?.pressHardwareButton(
            button: button,
            durationMs: durationMs,
            usagePage: usagePage,
            usage: usage
        ) ?? false
        guard !sent else { return }
        Task {
            await postHardwareButton(
                named: button,
                durationMs: durationMs,
                phase: nil,
                usagePage: usagePage,
                usage: usage
            )
        }
    }

    func rotateLeft() {
        hapticSelection()
        streamClient?.sendRotateLeft()
    }

    func rotateRight() {
        hapticSelection()
        streamClient?.sendRotateRight()
    }

    func toggleAppearance() {
        guard selectedSimulatorID != nil else { return }
        hapticSelection()
        let sent = streamClient?.sendToggleAppearance() ?? false
        guard !sent else { return }
        Task {
            await postToggleAppearance()
        }
    }

    func requestKeyframe() {
        hapticImpact()
        streamClient?.requestKeyframe()
    }

    func retryStream() {
        reconnectTask?.cancel()
        reconnectTask = nil
        hapticSelection()
        Task {
            await startStream()
        }
    }

    func setStreamEncoder(_ encoder: StreamEncoder) {
        updateStreamConfig { $0.encoder = encoder }
    }

    func setStreamFPS(_ fps: Int) {
        updateStreamConfig { $0.fps = fps }
    }

    func setStreamQuality(_ quality: StreamQualityPreset) {
        updateStreamConfig { $0.quality = quality }
    }

    private func autoConnectIfNeeded(_ endpoint: SimDeckEndpoint) async {
        guard !hasAutoConnected, !isAutoConnecting, self.endpoint == nil, authEndpoint == nil else { return }
        await autoConnectToAvailableEndpointIfNeeded(preferredEndpoint: endpoint)
    }

    private func autoConnectToAvailableEndpointIfNeeded(preferredEndpoint: SimDeckEndpoint? = nil) async {
        guard !hasAutoConnected, !isAutoConnecting, self.endpoint == nil, authEndpoint == nil else { return }
        let candidates = autoConnectCandidates(preferredEndpoint: preferredEndpoint)
        guard !candidates.isEmpty else { return }

        isAutoConnecting = true
        var connected = false
        for candidate in candidates {
            connected = await connect(
                candidate,
                autoStart: false,
                saveEndpoint: false,
                presentPairingOnAuth: false
            )
            if connected {
                break
            }
        }
        isAutoConnecting = false
        if connected {
            hasAutoConnected = true
        }
    }

    private func autoConnectCandidates(preferredEndpoint: SimDeckEndpoint?) -> [SimDeckEndpoint] {
        let orderedEndpoints = [preferredEndpoint].compactMap(\.self)
            + discovery.endpoints
            + savedEndpoints
        return uniqued(orderedEndpoints)
            .map(endpointWithReusableToken)
            .filter { endpoint in
                !endpoint.requiresPairing || endpoint.token?.nilIfBlank != nil
            }
    }

    private func isCurrentStreamRequest(_ generation: Int, simulatorID: String) -> Bool {
        streamRequestGeneration == generation && selectedSimulatorID == simulatorID
    }

    func handleScenePhase(_ phase: ScenePhase) {
        switch phase {
        case .active:
            streamClient?.appDidBecomeActive()
            if streamClient == nil, streamState == .disconnected || streamState == .failed {
                scheduleStreamReconnect(reason: "foreground")
            }
        case .background:
            streamClient?.appDidEnterBackground()
        case .inactive:
            break
        @unknown default:
            break
        }
    }

    func scheduleStreamReconnect(reason: String) {
        guard endpoint != nil, selectedSimulatorID != nil, selectedSimulator?.isBooted == true else { return }
        guard streamState != .connecting else { return }
        reconnectTask?.cancel()
        reconnectTask = Task { @MainActor [weak self] in
            guard let self else { return }
            let elapsed = Date().timeIntervalSince(self.lastReconnectStartedAt)
            if elapsed < 1.5 {
                try? await Task.sleep(for: .milliseconds(Int((1.5 - elapsed) * 1000)))
            }
            var attempt = 0
            while !Task.isCancelled,
                  self.endpoint != nil,
                  self.selectedSimulatorID != nil,
                  self.selectedSimulator?.isBooted == true {
                attempt += 1
                self.lastReconnectStartedAt = Date()
                self.status = attempt == 1
                    ? (reason == "foreground" ? "Resuming stream." : "Recovering stream.")
                    : "Retrying stream."
                let connected = await self.startStream(automaticReconnect: true)
                guard !connected else { return }
                let delay = min(10.0, pow(1.8, Double(attempt)))
                self.status = "Retrying stream in \(Int(delay.rounded(.up)))s."
                try? await Task.sleep(for: .milliseconds(Int(delay * 1000)))
            }
        }
    }

    private func postHardwareButton(
        named button: String,
        durationMs: Int?,
        phase: HardwareButtonPhase?,
        usagePage: Int?,
        usage: Int?
    ) async {
        guard let endpoint, let selectedSimulatorID else { return }
        do {
            let payload = HardwareButtonControlPayload(
                button: button,
                durationMs: durationMs,
                phase: phase?.rawValue,
                usagePage: usagePage,
                usage: usage
            )
            let encodedID = selectedSimulatorID.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? selectedSimulatorID
            try await SimDeckAPI(endpoint: endpoint).postControl(payload, path: "/api/simulators/\(encodedID)/button")
        } catch {
            status = error.localizedDescription
            hapticWarning()
        }
    }

    private func postKey(keyCode: Int, modifiers: Int) async {
        guard let endpoint, let selectedSimulatorID else { return }
        do {
            let payload = KeyControlPayload(keyCode: keyCode, modifiers: modifiers)
            let encodedID = selectedSimulatorID.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? selectedSimulatorID
            try await SimDeckAPI(endpoint: endpoint).postControl(payload, path: "/api/simulators/\(encodedID)/key")
        } catch {
            status = error.localizedDescription
            hapticWarning()
        }
    }

    private func postDismissKeyboard() async {
        guard let endpoint, let selectedSimulatorID else { return }
        do {
            let encodedID = selectedSimulatorID.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? selectedSimulatorID
            try await SimDeckAPI(endpoint: endpoint).postControl(
                EmptyControlPayload(),
                path: "/api/simulators/\(encodedID)/dismiss-keyboard"
            )
        } catch {
            status = error.localizedDescription
            hapticWarning()
        }
    }

    private func postToggleAppearance() async {
        guard let endpoint, let selectedSimulatorID else { return }
        do {
            let encodedID = selectedSimulatorID.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? selectedSimulatorID
            try await SimDeckAPI(endpoint: endpoint).postControl(
                EmptyControlPayload(),
                path: "/api/simulators/\(encodedID)/toggle-appearance"
            )
        } catch {
            status = error.localizedDescription
            hapticWarning()
        }
    }

    private func resetStreamPresentation() {
        streamDisplayToken &+= 1
        if !applyCachedChromeAssetsForSelection() {
            chromeProfile = nil
            chromeImage = nil
            chromeScreenMask = nil
        }
        if !applyCachedLastStreamFrameForSelection() {
            lastStreamFrameKey = nil
            lastStreamFrame = nil
            videoSize = .zero
        } else if let lastStreamFrame {
            videoSize = lastStreamFrame.size
        }
        hasCurrentStreamFrame = false
        streamDiagnostics = StreamDiagnostics()
    }

    private func chromeAssets(
        api: SimDeckAPI,
        endpoint: SimDeckEndpoint,
        simulatorID: String
    ) async -> ChromeAssets {
        if let cached = cachedChromeAssets(endpoint: endpoint, simulatorID: simulatorID) {
            return cached
        }

        let loadedProfile = try? await api.chromeProfile(udid: simulatorID)
        let loadedImage = try? await api.chromeImage(udid: simulatorID)
        let loadedScreenMask: UIImage?
        if loadedProfile?.hasScreenMask == true {
            loadedScreenMask = try? await api.screenMaskImage(udid: simulatorID)
        } else {
            loadedScreenMask = nil
        }
        let loadedAssets = ChromeAssets(profile: loadedProfile, image: loadedImage, screenMask: loadedScreenMask)
        cacheChromeAssets(loadedAssets, endpoint: endpoint, simulatorID: simulatorID)
        return loadedAssets
    }

    @discardableResult
    private func applyCachedChromeAssetsForSelection() -> Bool {
        guard let endpoint, let selectedSimulatorID,
              let cached = cachedChromeAssets(endpoint: endpoint, simulatorID: selectedSimulatorID) else {
            return false
        }
        applyChromeAssets(cached)
        return true
    }

    private func applyChromeAssets(_ assets: ChromeAssets) {
        chromeProfile = assets.profile
        chromeImage = assets.image
        chromeScreenMask = assets.screenMask
    }

    private func cachedChromeAssets(endpoint: SimDeckEndpoint, simulatorID: String) -> ChromeAssets? {
        let key = chromeCacheKey(endpoint: endpoint, simulatorID: simulatorID)
        guard let cached = chromeCache[key] else { return nil }
        markChromeCacheKeyUsed(key)
        return cached
    }

    private func cacheChromeAssets(_ assets: ChromeAssets, endpoint: SimDeckEndpoint, simulatorID: String) {
        guard !assets.isEmpty else { return }
        let key = chromeCacheKey(endpoint: endpoint, simulatorID: simulatorID)
        chromeCache[key] = assets
        markChromeCacheKeyUsed(key)
        while chromeCacheOrder.count > Self.chromeCacheLimit, let evictedKey = chromeCacheOrder.first {
            chromeCacheOrder.removeFirst()
            chromeCache[evictedKey] = nil
        }
    }

    private func markChromeCacheKeyUsed(_ key: String) {
        chromeCacheOrder.removeAll { $0 == key }
        chromeCacheOrder.append(key)
    }

    private func chromeCacheKey(endpoint: SimDeckEndpoint, simulatorID: String) -> String {
        "\(endpoint.baseURL.absoluteString)|\(simulatorID)"
    }

    @discardableResult
    private func applyCachedLastStreamFrameForSelection() -> Bool {
        guard let endpoint, let selectedSimulatorID else {
            return false
        }
        let cacheKey = lastFrameCacheKey(endpoint: endpoint, simulatorID: selectedSimulatorID)
        if lastStreamFrameKey == cacheKey, lastStreamFrame != nil {
            return true
        }
        guard let image = loadLastStreamFrame(endpoint: endpoint, simulatorID: selectedSimulatorID) else {
            return false
        }
        lastStreamFrameKey = cacheKey
        lastStreamFrame = image
        return true
    }

    private func loadLastStreamFrame(endpoint: SimDeckEndpoint, simulatorID: String) -> UIImage? {
        guard let url = lastFrameCacheURL(endpoint: endpoint, simulatorID: simulatorID),
              let data = try? Data(contentsOf: url) else {
            return nil
        }
        return UIImage(data: data)
    }

    private func persistLastStreamFrame(_ image: UIImage, endpoint: SimDeckEndpoint, simulatorID: String) {
        guard let url = lastFrameCacheURL(endpoint: endpoint, simulatorID: simulatorID),
              let data = image.jpegData(compressionQuality: 0.78) else {
            return
        }
        Task.detached(priority: .utility) {
            do {
                try FileManager.default.createDirectory(
                    at: url.deletingLastPathComponent(),
                    withIntermediateDirectories: true
                )
                try data.write(to: url, options: [.atomic])
            } catch {
                #if DEBUG
                print("Unable to persist SimDeck frame cache: \(error.localizedDescription)")
                #endif
            }
        }
    }

    private func lastFrameCacheURL(endpoint: SimDeckEndpoint, simulatorID: String) -> URL? {
        guard let baseURL = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first else {
            return nil
        }
        return baseURL
            .appendingPathComponent(Self.lastFrameCacheDirectoryName, isDirectory: true)
            .appendingPathComponent("\(lastFrameCacheKey(endpoint: endpoint, simulatorID: simulatorID)).jpg")
    }

    private func lastFrameCacheKey(endpoint: SimDeckEndpoint, simulatorID: String) -> String {
        let source = "\(endpoint.baseURL.absoluteString)|\(simulatorID)"
        let digest = SHA256.hash(data: Data(source.utf8))
        return digest.map { String(format: "%02x", $0) }.joined()
    }

    private func updateStreamConfig(_ update: (inout StreamConfig) -> Void) {
        var next = streamConfig
        update(&next)
        guard next != streamConfig else { return }
        streamConfig = next
        saveStreamConfig(next)
        streamClient?.applyStreamQuality(next)
        if streamClient != nil {
            status = "Stream set to \(next.summary)."
        }
        hapticSelection()
    }

    private func upsertSimulator(_ simulator: SimulatorMetadata) {
        if let index = simulators.firstIndex(where: { $0.udid == simulator.udid }) {
            simulators[index] = simulator
        } else {
            simulators.insert(simulator, at: 0)
        }
    }

    private static func keyControl(for character: Character) -> (keyCode: Int, modifiers: Int)? {
        let shift = 1 << 0
        let value = String(character)
        if let keyCode = unshiftedHIDUsage[value] {
            return (keyCode, 0)
        }
        if let keyCode = shiftedHIDUsage[value] {
            return (keyCode, shift)
        }
        return nil
    }

    private static let unshiftedHIDUsage: [String: Int] = [
        "a": 4, "b": 5, "c": 6, "d": 7, "e": 8, "f": 9, "g": 10, "h": 11, "i": 12,
        "j": 13, "k": 14, "l": 15, "m": 16, "n": 17, "o": 18, "p": 19, "q": 20,
        "r": 21, "s": 22, "t": 23, "u": 24, "v": 25, "w": 26, "x": 27, "y": 28, "z": 29,
        "1": 30, "2": 31, "3": 32, "4": 33, "5": 34, "6": 35, "7": 36, "8": 37, "9": 38, "0": 39,
        "\n": 40, "\r": 40, "\u{1B}": 41, "\t": 43, " ": 44,
        "-": 45, "=": 46, "[": 47, "]": 48, "\\": 49, ";": 51, "'": 52,
        "`": 53, ",": 54, ".": 55, "/": 56,
        "\u{2019}": 52, "\u{2018}": 52, "\u{2013}": 45, "\u{2014}": 45
    ]

    private static let shiftedHIDUsage: [String: Int] = [
        "A": 4, "B": 5, "C": 6, "D": 7, "E": 8, "F": 9, "G": 10, "H": 11, "I": 12,
        "J": 13, "K": 14, "L": 15, "M": 16, "N": 17, "O": 18, "P": 19, "Q": 20,
        "R": 21, "S": 22, "T": 23, "U": 24, "V": 25, "W": 26, "X": 27, "Y": 28, "Z": 29,
        "!": 30, "@": 31, "#": 32, "$": 33, "%": 34, "^": 35, "&": 36, "*": 37, "(": 38, ")": 39,
        "_": 45, "+": 46, "{": 47, "}": 48, "|": 49, ":": 51, "\"": 52,
        "~": 53, "<": 54, ">": 55, "?": 56,
        "\u{201C}": 52, "\u{201D}": 52
    ]

    private func loadSavedEndpoints() {
        let data = UserDefaults.standard.data(forKey: Self.savedEndpointsKey)
            ?? UserDefaults.standard.data(forKey: Self.legacyRecentEndpointsKey)
        guard let data,
              let endpoints = try? JSONDecoder().decode([SimDeckEndpoint].self, from: data) else {
            return
        }
        savedEndpoints = uniqued(endpoints).map { endpoint in
            var saved = endpoint
            if saved.source == .recent {
                saved.source = .manual
            }
            return saved
        }
        persistSavedEndpoints()
    }

    func saveUserEndpoint(_ endpoint: SimDeckEndpoint) {
        var saved = endpoint
        if saved.source == .recent {
            saved.source = .manual
        }
        saved.requiresPairing = false
        if let existing = savedEndpoints.first(where: { endpointsRepresentSameServer($0, saved) }) {
            saved = mergedEndpoint(existing, saved)
            saved.source = .manual
            saved.requiresPairing = false
        }
        savedEndpoints.removeAll { endpointsRepresentSameServer($0, saved) }
        savedEndpoints.insert(saved, at: 0)
        savedEndpoints = Array(uniqued(savedEndpoints).prefix(12))
        persistSavedEndpoints()
    }

    func renameSavedEndpoint(_ endpoint: SimDeckEndpoint, to name: String) {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty,
              let index = savedEndpoints.firstIndex(where: { endpointsRepresentSameServer($0, endpoint) }) else {
            return
        }
        savedEndpoints[index].name = trimmed
        if var current = self.endpoint, endpointsRepresentSameServer(current, endpoint) {
            current.name = trimmed
            self.endpoint = current
            saveSelectedEndpoint(current)
        }
        if var pending = authEndpoint, endpointsRepresentSameServer(pending, endpoint) {
            pending.name = trimmed
            authEndpoint = pending
        }
        persistSavedEndpoints()
    }

    func deleteSavedEndpoint(_ endpoint: SimDeckEndpoint) {
        savedEndpoints.removeAll { endpointsRepresentSameServer($0, endpoint) }
        if let current = self.endpoint, endpointsRepresentSameServer(current, endpoint) {
            UserDefaults.standard.removeObject(forKey: Self.selectedEndpointKey)
        }
        persistSavedEndpoints()
        hapticSelection()
    }

    private func savePairedEndpoints(primary: SimDeckEndpoint, alternates: [SimDeckEndpoint], token: String) {
        for endpoint in Array(alternates.reversed()) + [primary] {
            var saved = endpoint
            saved.token = token
            saved.requiresPairing = false
            saveUserEndpoint(saved)
        }
    }

    private func endpointWithReusableToken(_ endpoint: SimDeckEndpoint) -> SimDeckEndpoint {
        guard endpoint.token?.nilIfBlank == nil,
              let token = reusableToken(for: endpoint) else {
            return endpoint
        }
        var endpoint = endpoint
        endpoint.token = token
        endpoint.requiresPairing = false
        return endpoint
    }

    private func reusableToken(for endpoint: SimDeckEndpoint) -> String? {
        let storedEndpoints = savedEndpoints + [self.endpoint, loadSelectedEndpoint()].compactMap(\.self)
        if let serverID = endpoint.serverID?.nilIfBlank,
           let token = storedEndpoints
            .first(where: { $0.serverID == serverID })?
            .token?
            .nilIfBlank {
            return token
        }
        if let exactToken = storedEndpoints
            .first(where: { endpointsRepresentSameServer($0, endpoint) })?
            .token?
            .nilIfBlank {
            return exactToken
        }

        guard hostCanShareSimDeckToken(endpoint.baseURL.host(percentEncoded: false)) else {
            return nil
        }
        let port = normalizedPort(for: endpoint.baseURL)
        return storedEndpoints
            .first { stored in
                stored.token?.nilIfBlank != nil
                    && normalizedPort(for: stored.baseURL) == port
                    && hostCanShareSimDeckToken(stored.baseURL.host(percentEncoded: false))
            }?
            .token?
            .nilIfBlank
    }

    private func connectionCandidates(for endpoint: SimDeckEndpoint) -> [SimDeckEndpoint] {
        let primary = endpointWithReusableToken(endpoint)
        let alternateEndpoints = preferredAlternateURLs(for: primary).map { url in
            var alternate = primary
            alternate.baseURL = url.normalizedSimDeckBaseURL()
            alternate.source = endpointSource(for: alternate.baseURL)
            alternate.alternateBaseURLs = ([primary.baseURL] + primary.alternateBaseURLs)
                .map { $0.normalizedSimDeckBaseURL() }
                .filter { $0 != alternate.baseURL }
            return endpointWithReusableToken(alternate)
        }
        return uniquedByBaseURL([primary] + alternateEndpoints)
    }

    private func preferredAlternateURLs(for endpoint: SimDeckEndpoint) -> [URL] {
        let urls = endpoint.alternateBaseURLs.filter { $0 != endpoint.baseURL }
        let preferred = urls.sorted {
            endpointSourceRank(endpointSource(for: $0)) < endpointSourceRank(endpointSource(for: $1))
        }
        return preferred
    }

    private func endpointSource(for url: URL) -> EndpointSource {
        guard let host = url.host(percentEncoded: false)?.lowercased() else {
            return .manual
        }
        let parts = host.split(separator: ".").compactMap { UInt8($0) }
        if parts.count == 4 && parts[0] == 100 && (parts[1] & 0b1100_0000) == 0b0100_0000 {
            return .tailscale
        }
        if host.hasSuffix(".local") {
            return .bonjour
        }
        if hostCanShareSimDeckToken(host) {
            return .lan
        }
        return .manual
    }

    private func endpointSourceRank(_ source: EndpointSource) -> Int {
        switch source {
        case .bonjour: 0
        case .lan: 1
        case .tailscale: 2
        case .studioLink: 3
        case .manual: 4
        case .recent: 5
        }
    }

    private func endpointsRepresentSameServer(_ lhs: SimDeckEndpoint, _ rhs: SimDeckEndpoint) -> Bool {
        if let lhsID = lhs.serverID?.nilIfBlank,
           let rhsID = rhs.serverID?.nilIfBlank {
            return lhsID == rhsID
        }
        return lhs.baseURL == rhs.baseURL
            || lhs.alternateBaseURLs.contains(rhs.baseURL)
            || rhs.alternateBaseURLs.contains(lhs.baseURL)
    }

    private func mergedEndpoint(_ lhs: SimDeckEndpoint, _ rhs: SimDeckEndpoint) -> SimDeckEndpoint {
        let preferred = endpointSourceRank(lhs.source) <= endpointSourceRank(rhs.source) ? lhs : rhs
        let other = preferred.baseURL == lhs.baseURL ? rhs : lhs
        var merged = preferred
        merged.serverID = preferred.serverID ?? other.serverID
        merged.token = preferred.token ?? other.token
        merged.preferredSimulatorID = preferred.preferredSimulatorID ?? other.preferredSimulatorID
        merged.requiresPairing = preferred.requiresPairing && other.requiresPairing
        merged.alternateBaseURLs = uniquedURLs(
            [lhs.baseURL, rhs.baseURL] + lhs.alternateBaseURLs + rhs.alternateBaseURLs
        )
        .filter { $0 != merged.baseURL }
        return merged
    }

    private func uniquedURLs(_ urls: [URL]) -> [URL] {
        var seen = Set<URL>()
        var result: [URL] = []
        for url in urls.map({ $0.normalizedSimDeckBaseURL() }) where seen.insert(url).inserted {
            result.append(url)
        }
        return result
    }

    private func uniquedByBaseURL(_ endpoints: [SimDeckEndpoint]) -> [SimDeckEndpoint] {
        var seen = Set<URL>()
        var result: [SimDeckEndpoint] = []
        for endpoint in endpoints where seen.insert(endpoint.baseURL).inserted {
            result.append(endpoint)
        }
        return result
    }

    private func alternateURLs(from health: HealthResponse, fallbackPort: Int) -> [URL] {
        guard let advertiseHost = health.advertiseHost?.nilIfBlank else { return [] }
        var components = URLComponents()
        components.scheme = "http"
        components.host = advertiseHost
        components.port = health.httpPort ?? fallbackPort
        return components.url.map { [$0] } ?? []
    }

    private func normalizedPort(for url: URL) -> Int {
        if let port = url.port {
            return port
        }
        return url.scheme?.lowercased() == "https" ? 443 : 80
    }

    private func hostCanShareSimDeckToken(_ host: String?) -> Bool {
        guard let host = host?.lowercased(), !host.isEmpty else {
            return false
        }
        if host == "localhost" || host.hasSuffix(".local") {
            return true
        }
        let parts = host.split(separator: ".").compactMap { UInt8($0) }
        guard parts.count == 4 else {
            return false
        }
        return parts[0] == 10
            || parts[0] == 127
            || (parts[0] == 169 && parts[1] == 254)
            || (parts[0] == 172 && (16...31).contains(parts[1]))
            || (parts[0] == 192 && parts[1] == 168)
            || (parts[0] == 100 && (parts[1] & 0b1100_0000) == 0b0100_0000)
    }

    private func persistSavedEndpoints() {
        if let data = try? JSONEncoder().encode(savedEndpoints) {
            UserDefaults.standard.set(data, forKey: Self.savedEndpointsKey)
        }
    }

    private func uniqued(_ endpoints: [SimDeckEndpoint]) -> [SimDeckEndpoint] {
        var result: [SimDeckEndpoint] = []
        for endpoint in endpoints {
            if let index = result.firstIndex(where: { endpointsRepresentSameServer($0, endpoint) }) {
                result[index] = mergedEndpoint(result[index], endpoint)
            } else {
                result.append(endpoint)
            }
        }
        return result
    }

    private func loadSelectedEndpoint() -> SimDeckEndpoint? {
        guard let data = UserDefaults.standard.data(forKey: Self.selectedEndpointKey) else {
            return nil
        }
        return try? JSONDecoder().decode(SimDeckEndpoint.self, from: data)
    }

    private func saveSelectedEndpoint(_ endpoint: SimDeckEndpoint) {
        if let data = try? JSONEncoder().encode(endpoint) {
            UserDefaults.standard.set(data, forKey: Self.selectedEndpointKey)
        }
    }

    private static func loadStreamConfig() -> StreamConfig {
        guard let data = UserDefaults.standard.data(forKey: streamConfigKey),
              let config = try? JSONDecoder().decode(StreamConfig.self, from: data) else {
            return StreamConfig()
        }
        return config
    }

    private func saveStreamConfig(_ config: StreamConfig) {
        if let data = try? JSONEncoder().encode(config) {
            UserDefaults.standard.set(data, forKey: Self.streamConfigKey)
        }
    }

    private static func loadHapticsEnabled() -> Bool {
        UserDefaults.standard.object(forKey: hapticsEnabledKey) as? Bool ?? true
    }

    func hapticSelection() {
        guard hapticsEnabled else { return }
        UISelectionFeedbackGenerator().selectionChanged()
    }

    func hapticImpact() {
        guard hapticsEnabled else { return }
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
    }

    func hapticSuccess() {
        guard hapticsEnabled else { return }
        UINotificationFeedbackGenerator().notificationOccurred(.success)
    }

    func hapticWarning() {
        guard hapticsEnabled else { return }
        UINotificationFeedbackGenerator().notificationOccurred(.warning)
    }
}

private extension CGFloat {
    func clamped(to range: ClosedRange<CGFloat>) -> CGFloat {
        Swift.min(Swift.max(self, range.lowerBound), range.upperBound)
    }
}

private extension StreamState {
    init(peerState: RTCPeerConnectionState) {
        switch peerState {
        case .connected:
            self = .connected
        case .connecting, .new:
            self = .connecting
        case .disconnected, .closed:
            self = .disconnected
        case .failed:
            self = .failed
        @unknown default:
            self = .disconnected
        }
    }
}
