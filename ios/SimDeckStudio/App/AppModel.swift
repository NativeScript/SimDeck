import Foundation
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

@MainActor
@Observable
final class AppModel {
    let discovery = SimDeckDiscovery()
    private static let recentEndpointsKey = "recentEndpoints"
    private static let selectedEndpointKey = "selectedEndpoint"
    private static let streamConfigKey = "streamConfig"
    private static let hapticsEnabledKey = "hapticsEnabled"

    var endpoint: SimDeckEndpoint?
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
    var streamDiagnostics = StreamDiagnostics()
    var bootingSimulatorID: String?
    var streamDisplayToken = 0
    var hasCurrentStreamFrame = false
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

    var isSelectedSimulatorBooting: Bool {
        bootingSimulatorID == selectedSimulatorID
    }

    var availableEndpoints: [SimDeckEndpoint] {
        var endpoints = discovery.endpoints
        if let endpoint, !endpoints.contains(where: { $0.baseURL == endpoint.baseURL }) {
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
        loadRecents()
        if let lastSelectedEndpoint = loadSelectedEndpoint() {
            discovery.upsert(lastSelectedEndpoint)
            isAutoConnecting = true
            Task {
                let connected = await connect(lastSelectedEndpoint, autoStart: false)
                isAutoConnecting = false
                hasAutoConnected = connected
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
        return await connect(endpoint, autoStart: false)
    }

    func handle(url: URL) {
        guard let route = StudioLinkResolver.route(for: url) else {
            status = "Unsupported link."
            return
        }
        switch route {
        case let .endpoint(endpoint, autoStart):
            Task { await connect(endpoint, autoStart: autoStart) }
        }
    }

    @discardableResult
    func connect(_ endpoint: SimDeckEndpoint, autoStart: Bool) async -> Bool {
        isBusy = true
        status = "Connecting to \(endpoint.name)"
        defer { isBusy = false }

        do {
            let api = SimDeckAPI(endpoint: endpoint)
            _ = try await api.health()
            let simulators = try await api.simulators()
            stopStream()
            self.endpoint = endpoint
            self.authEndpoint = nil
            self.simulators = simulators
            selectedSimulatorID = autoStart
                ? endpoint.preferredSimulatorID
                    ?? simulators.first(where: \.isBooted)?.udid
                    ?? simulators.first?.udid
                : endpoint.preferredSimulatorID
            saveRecent(endpoint)
            saveSelectedEndpoint(endpoint)
            status = simulators.isEmpty ? "Connected. No simulators found." : "Connected."
            hapticSuccess()
            if autoStart, selectedSimulatorID != nil {
                await prepareSelectedSimulator()
            }
            return true
        } catch SimDeckAPIError.authRequired {
            var pendingEndpoint = endpoint
            pendingEndpoint.requiresPairing = true
            self.endpoint = pendingEndpoint
            self.authEndpoint = pendingEndpoint
            self.simulators = []
            self.selectedSimulatorID = nil
            manualAddress = pendingEndpoint.baseURL.absoluteString
            manualToken = pendingEndpoint.token ?? ""
            discovery.upsert(pendingEndpoint)
            saveRecent(pendingEndpoint)
            saveSelectedEndpoint(pendingEndpoint)
            status = "Pairing required."
            hapticWarning()
            return false
        } catch {
            status = error.localizedDescription
            hapticWarning()
            return false
        }
    }

    @discardableResult
    func pair() async -> Bool {
        guard let authEndpoint else { return false }
        isBusy = true
        defer { isBusy = false }
        do {
            let token = try await SimDeckAPI(endpoint: authEndpoint).pair(code: pairingCode)
            var pairedEndpoint = authEndpoint
            if let token {
                pairedEndpoint.token = token
                manualToken = token
            }
            pairingCode = ""
            let connected = await connect(pairedEndpoint, autoStart: false)
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
        let connected = await connect(authEndpoint, autoStart: false)
        if connected {
            hapticSuccess()
        }
        return connected
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

    func startStream() async {
        guard let endpoint, let selectedSimulatorID else { return }
        guard selectedSimulator?.isBooted == true else {
            await loadSelectedSimulatorChrome()
            return
        }
        streamRequestGeneration += 1
        let generation = streamRequestGeneration
        stopCurrentStream(resetState: false)
        resetStreamPresentation()
        streamState = .connecting
        status = "Starting WebRTC."
        do {
            let api = SimDeckAPI(endpoint: endpoint)
            let health = try await api.health()
            async let profile = try? api.chromeProfile(udid: selectedSimulatorID)
            async let image = try? api.chromeImage(udid: selectedSimulatorID)
            let client = WebRTCClient()
            client.onConnectionState = { [weak self] state in
                Task { @MainActor in
                    self?.streamState = StreamState(peerState: state)
                }
            }
            client.onVideoSize = { [weak self] size in
                Task { @MainActor in
                    if self?.videoSize != size {
                        self?.videoSize = size
                    }
                }
            }
            client.onDiagnostics = { [weak self] diagnostics in
                Task { @MainActor in
                    self?.streamDiagnostics = diagnostics
                }
            }
            let loadedChromeProfile = await profile
            let loadedChromeImage = await image
            guard isCurrentStreamRequest(generation, simulatorID: selectedSimulatorID) else {
                client.disconnect()
                return
            }
            chromeProfile = loadedChromeProfile
            chromeImage = loadedChromeImage
            let answer = try await client.connect(
                api: api,
                simulatorID: selectedSimulatorID,
                health: health,
                streamConfig: streamConfig
            )
            guard isCurrentStreamRequest(generation, simulatorID: selectedSimulatorID) else {
                client.disconnect()
                return
            }
            streamClient = client
            if let video = answer.video, video.width > 0, video.height > 0 {
                videoSize = CGSize(width: video.width, height: video.height)
            }
            status = "WebRTC connected."
            hapticSuccess()
        } catch {
            guard streamRequestGeneration == generation else { return }
            streamState = .failed
            status = error.localizedDescription
            hapticWarning()
            stopCurrentStream(resetState: false)
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
        async let profile = try? api.chromeProfile(udid: selectedSimulatorID)
        async let image = try? api.chromeImage(udid: selectedSimulatorID)
        let loadedChromeProfile = await profile
        let loadedChromeImage = await image
        guard isCurrentStreamRequest(generation, simulatorID: selectedSimulatorID) else { return }
        chromeProfile = loadedChromeProfile
        chromeImage = loadedChromeImage
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

    func sendTouch(x: Double, y: Double, phase: String) {
        streamClient?.sendTouch(x: x, y: y, phase: phase)
    }

    func sendHome() {
        hapticImpact()
        streamClient?.sendHome()
    }

    func sendAppSwitcher() {
        hapticImpact()
        streamClient?.sendAppSwitcher()
    }

    func sendLock() {
        hapticImpact()
        streamClient?.sendLock()
    }

    func rotateLeft() {
        hapticSelection()
        streamClient?.sendRotateLeft()
    }

    func rotateRight() {
        hapticSelection()
        streamClient?.sendRotateRight()
    }

    func requestKeyframe() {
        hapticImpact()
        streamClient?.requestKeyframe()
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
        isAutoConnecting = true
        let connected = await connect(endpoint, autoStart: false)
        isAutoConnecting = false
        if connected {
            hasAutoConnected = true
        }
    }

    private func isCurrentStreamRequest(_ generation: Int, simulatorID: String) -> Bool {
        streamRequestGeneration == generation && selectedSimulatorID == simulatorID
    }

    private func resetStreamPresentation() {
        streamDisplayToken &+= 1
        chromeProfile = nil
        chromeImage = nil
        videoSize = .zero
        hasCurrentStreamFrame = false
        streamDiagnostics = StreamDiagnostics()
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

    private func loadRecents() {
        guard let data = UserDefaults.standard.data(forKey: Self.recentEndpointsKey),
              let endpoints = try? JSONDecoder().decode([SimDeckEndpoint].self, from: data) else {
            return
        }
        for endpoint in endpoints {
            var recent = endpoint
            recent.source = .recent
            discovery.upsert(recent)
        }
    }

    private func saveRecent(_ endpoint: SimDeckEndpoint) {
        var endpoint = endpoint
        endpoint.source = .recent
        var recents = (try? JSONDecoder().decode(
            [SimDeckEndpoint].self,
            from: UserDefaults.standard.data(forKey: Self.recentEndpointsKey) ?? Data()
        )) ?? []
        recents.removeAll { $0.baseURL == endpoint.baseURL }
        recents.insert(endpoint, at: 0)
        recents = Array(recents.prefix(8))
        if let data = try? JSONEncoder().encode(recents) {
            UserDefaults.standard.set(data, forKey: Self.recentEndpointsKey)
        }
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
