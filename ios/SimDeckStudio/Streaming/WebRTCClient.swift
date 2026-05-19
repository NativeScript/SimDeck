import Foundation
@preconcurrency import WebRTC

final class WebRTCClient: NSObject {
    let clientID = "simdeck-ios-\(UUID().uuidString)"

    var onConnectionState: (@Sendable (RTCPeerConnectionState) -> Void)?
    var onVideoSize: (@Sendable (CGSize) -> Void)?
    var onMessage: (@Sendable (String) -> Void)?
    var onDiagnostics: (@Sendable (StreamDiagnostics) -> Void)?
    var onReconnectNeeded: (@Sendable (String) -> Void)?

    private static let initializeSSL: Void = {
        RTCInitializeSSL()
    }()

    private let factory: RTCPeerConnectionFactory
    private var peerConnection: RTCPeerConnection?
    private var controlChannel: RTCDataChannel?
    private var telemetryChannel: RTCDataChannel?
    private var remoteTrack: RTCVideoTrack?
    private var renderers: [any RTCVideoRenderer] = []
    private var pendingControlMessages: [Data] = []
    private var keepAliveTask: Task<Void, Never>?
    private var statsTask: Task<Void, Never>?
    private var renderWatchdogTask: Task<Void, Never>?
    private var activeSimulatorID: String?
    private var peerConnectionState = "new"
    private var iceConnectionState = "new"
    private var iceGatheringState = "new"
    private var signalingState = "stable"
    private var lastDecodedFrames: UInt64 = 0
    private var lastDecodedFrameAt = Date()
    private var lastPacketsReceived: UInt64 = 0
    private var lastPacketReceivedAt = Date()
    private var lastStatsSampleAt: Date?
    private var lastStatsDecodedFrames: UInt64 = 0
    private var lastStatsRenderedFrames: UInt64 = 0
    private var lastStatsPacketsReceived: UInt64 = 0
    private var lastStallRecoveryAt = Date.distantPast
    private var lastReconnectRequestedAt = Date.distantPast
    private var lastUserActivityAt = Date.distantPast
    private var renderedFrameCount: UInt64 = 0
    private var lastRenderedFrameAt = Date()
    private var isAppForeground = true
    private var isDisconnecting = false

    override init() {
        _ = Self.initializeSSL
        factory = RTCPeerConnectionFactory(
            encoderFactory: RTCDefaultVideoEncoderFactory(),
            decoderFactory: RTCDefaultVideoDecoderFactory()
        )
        super.init()
    }

    func connect(
        api: SimDeckAPI,
        simulatorID: String,
        health: HealthResponse,
        streamConfig: StreamConfig
    ) async throws -> WebRTCAnswerPayload {
        disconnect()
        isDisconnecting = false

        let configuration = RTCConfiguration()
        configuration.sdpSemantics = .unifiedPlan
        configuration.bundlePolicy = .maxBundle
        configuration.rtcpMuxPolicy = .require
        configuration.continualGatheringPolicy = .gatherContinually
        configuration.tcpCandidatePolicy = .disabled
        configuration.enableDscp = true
        configuration.rtcpVideoReportIntervalMs = 250
        configuration.iceServers = iceServers(from: health)
        if health.webRtc?.iceTransportPolicy?.lowercased() == "relay" {
            configuration.iceTransportPolicy = .relay
        } else {
            configuration.iceTransportPolicy = .all
        }

        let constraints = RTCMediaConstraints(
            mandatoryConstraints: nil,
            optionalConstraints: ["DtlsSrtpKeyAgreement": "true"]
        )
        guard let peerConnection = factory.peerConnection(with: configuration, constraints: constraints, delegate: self) else {
            throw SimDeckAPIError.invalidResponse
        }
        self.peerConnection = peerConnection

        let transceiverInit = RTCRtpTransceiverInit()
        transceiverInit.direction = .recvOnly
        _ = peerConnection.addTransceiver(of: .video, init: transceiverInit)

        let controlConfig = RTCDataChannelConfiguration()
        controlConfig.isOrdered = true
        controlChannel = peerConnection.dataChannel(forLabel: "simdeck-control", configuration: controlConfig)
        controlChannel?.delegate = self

        let telemetryConfig = RTCDataChannelConfiguration()
        telemetryConfig.isOrdered = false
        telemetryConfig.maxRetransmits = 0
        telemetryChannel = peerConnection.dataChannel(forLabel: "simdeck-telemetry", configuration: telemetryConfig)
        telemetryChannel?.delegate = self

        let offer = try await offer(for: peerConnection)
        try await setLocalDescription(offer, on: peerConnection)
        await waitForIceGathering(on: peerConnection, timeout: api.baseURL.isLoopbackOrLocal ? 0.35 : 3.0)

        guard let localDescription = peerConnection.localDescription else {
            throw SimDeckAPIError.invalidResponse
        }
        let payload = WebRTCOfferPayload(
            clientId: clientID,
            sdp: localDescription.sdp,
            streamConfig: StreamQualityPayload(config: streamConfig),
            type: "offer"
        )
        let answer = try await api.postWebRTCOffer(payload, udid: simulatorID)
        try await setRemoteDescription(
            RTCSessionDescription(type: .answer, sdp: answer.sdp),
            on: peerConnection
        )
        activeSimulatorID = simulatorID
        lastDecodedFrames = 0
        lastPacketsReceived = 0
        lastDecodedFrameAt = Date()
        lastPacketReceivedAt = Date()
        lastStatsSampleAt = nil
        lastStatsDecodedFrames = 0
        lastStatsRenderedFrames = 0
        lastStatsPacketsReceived = 0
        renderedFrameCount = 0
        lastRenderedFrameAt = Date()
        lastStallRecoveryAt = .distantPast
        lastReconnectRequestedAt = .distantPast
        lastUserActivityAt = .distantPast
        isAppForeground = true
        sendPageVisibilityStats(visible: true, simulatorID: simulatorID)
        sendStreamControl(foreground: true, forceKeyframe: true, snapshot: true)
        startKeepAlive()
        startStatsReporting(simulatorID: simulatorID)
        startRenderWatchdog()
        return answer
    }

    func attachRenderer(_ renderer: any RTCVideoRenderer) {
        let rendererObject = renderer as AnyObject
        if renderers.contains(where: { ($0 as AnyObject) === rendererObject }) {
            return
        }
        renderers.append(renderer)
        remoteTrack?.add(renderer)
    }

    func detachRenderer(_ renderer: any RTCVideoRenderer) {
        remoteTrack?.remove(renderer)
        let rendererObject = renderer as AnyObject
        renderers.removeAll { ($0 as AnyObject) === rendererObject }
    }

    func disconnect() {
        isDisconnecting = true
        keepAliveTask?.cancel()
        keepAliveTask = nil
        statsTask?.cancel()
        statsTask = nil
        renderWatchdogTask?.cancel()
        renderWatchdogTask = nil
        if let activeSimulatorID {
            sendPageVisibilityStats(visible: false, simulatorID: activeSimulatorID)
        }
        sendStreamControl(foreground: false, forceKeyframe: false, snapshot: false, allowQueue: false)
        activeSimulatorID = nil
        for renderer in renderers {
            remoteTrack?.remove(renderer)
        }
        renderers.removeAll()
        remoteTrack = nil
        controlChannel?.close()
        telemetryChannel?.close()
        controlChannel = nil
        telemetryChannel = nil
        pendingControlMessages.removeAll()
        peerConnection?.close()
        peerConnection = nil
    }

    @discardableResult
    func sendTouch(x: Double, y: Double, phase: String) -> Bool {
        markUserActivity()
        return sendJSON(["type": "touch", "x": x, "y": y, "phase": phase], allowQueue: false)
    }

    @discardableResult
    func sendEdgeTouch(x: Double, y: Double, phase: String, edge: String) -> Bool {
        markUserActivity()
        return sendJSON(["type": "edgeTouch", "x": x, "y": y, "phase": phase, "edge": edge], allowQueue: false)
    }

    @discardableResult
    func sendMultiTouch(x1: Double, y1: Double, x2: Double, y2: Double, phase: String) -> Bool {
        markUserActivity()
        return sendJSON([
            "type": "multiTouch",
            "x1": x1,
            "y1": y1,
            "x2": x2,
            "y2": y2,
            "phase": phase
        ], allowQueue: false)
    }

    @discardableResult
    func sendKey(keyCode: Int, modifiers: Int) -> Bool {
        markUserActivity()
        return sendJSON([
            "type": "key",
            "keyCode": keyCode,
            "modifiers": modifiers
        ], allowQueue: false)
    }

    @discardableResult
    func dismissSimulatorKeyboard() -> Bool {
        markUserActivity()
        return sendJSON(["type": "dismissKeyboard"], allowQueue: false)
    }

    func sendHome() {
        markUserActivity()
        sendJSON(["type": "home"])
    }

    func sendAppSwitcher() {
        markUserActivity()
        sendJSON(["type": "appSwitcher"])
    }

    func sendRotateLeft() {
        markUserActivity()
        sendJSON(["type": "rotateLeft"])
    }

    func sendRotateRight() {
        markUserActivity()
        sendJSON(["type": "rotateRight"])
    }

    @discardableResult
    func sendCrown(delta: Double) -> Bool {
        guard delta.isFinite else { return false }
        markUserActivity()
        return sendJSON(["type": "crown", "delta": delta], allowQueue: false)
    }

    @discardableResult
    func sendToggleAppearance() -> Bool {
        markUserActivity()
        return sendJSON(["type": "toggleAppearance"], allowQueue: false)
    }

    func sendLock() {
        markUserActivity()
        pressHardwareButton(button: "power", durationMs: 80)
    }

    @discardableResult
    func sendHardwareButton(button: String, phase: String, usagePage: Int?, usage: Int?) -> Bool {
        markUserActivity()
        var payload: [String: Any] = [
            "type": "button",
            "button": button,
            "phase": phase
        ]
        if let usagePage {
            payload["usagePage"] = usagePage
        }
        if let usage {
            payload["usage"] = usage
        }
        return sendJSON(payload, allowQueue: false)
    }

    @discardableResult
    func pressHardwareButton(button: String, durationMs: Int = 80, usagePage: Int? = nil, usage: Int? = nil) -> Bool {
        markUserActivity()
        var payload: [String: Any] = [
            "type": "button",
            "button": button,
            "durationMs": durationMs
        ]
        if let usagePage {
            payload["usagePage"] = usagePage
        }
        if let usage {
            payload["usage"] = usage
        }
        return sendJSON(payload, allowQueue: false)
    }

    func requestKeyframe() {
        markUserActivity()
        sendStreamControl(foreground: true, forceKeyframe: true, snapshot: true)
    }

    func applyStreamQuality(_ config: StreamConfig) {
        sendJSON([
            "type": "streamQuality",
            "config": StreamQualityPayload(config: config).jsonObject
        ])
        sendStreamControl(foreground: true, forceKeyframe: true, snapshot: true)
    }

    func recordRenderedFrame(_ frame: RTCVideoFrame?) {
        guard frame != nil else { return }
        renderedFrameCount += 1
        lastRenderedFrameAt = Date()
    }

    func appDidBecomeActive() {
        isAppForeground = true
        startKeepAlive()
        startRenderWatchdog()
        if let activeSimulatorID {
            sendPageVisibilityStats(visible: true, simulatorID: activeSimulatorID)
        }
        sendStreamControl(foreground: true, forceKeyframe: true, snapshot: true, allowQueue: false)
        let now = Date()
        let staleFrameGap = now.timeIntervalSince(max(lastRenderedFrameAt, lastDecodedFrameAt))
        if peerConnectionState != "connected" || staleFrameGap > 4 {
            requestReconnect(reason: "foreground-resume")
        }
    }

    func appDidEnterBackground() {
        isAppForeground = false
        keepAliveTask?.cancel()
        keepAliveTask = nil
        if let activeSimulatorID {
            sendPageVisibilityStats(visible: false, simulatorID: activeSimulatorID)
        }
        sendStreamControl(foreground: false, forceKeyframe: false, snapshot: false, allowQueue: false)
    }

    private func sendStreamControl(
        foreground: Bool,
        forceKeyframe: Bool,
        snapshot: Bool,
        allowQueue: Bool = true
    ) {
        sendJSON([
            "type": "streamControl",
            "clientId": clientID,
            "foreground": foreground,
            "forceKeyframe": forceKeyframe,
            "snapshot": snapshot
        ], allowQueue: allowQueue)
    }

    private func markUserActivity() {
        lastUserActivityAt = Date()
    }

    @discardableResult
    private func sendJSON(_ object: [String: Any], allowQueue: Bool = true) -> Bool {
        guard let data = try? JSONSerialization.data(withJSONObject: object) else { return false }
        let isMove = Self.isMoveControlMessage(object)
        if isMove, let controlChannel, controlChannel.readyState == .open, controlChannel.bufferedAmount > 128_000 {
            return false
        }
        if sendControlData(data) {
            return true
        }
        guard allowQueue, !isMove else { return false }
        pendingControlMessages.append(data)
        if pendingControlMessages.count > 80 {
            pendingControlMessages.removeFirst(pendingControlMessages.count - 80)
        }
        return false
    }

    private func requestReconnect(reason: String) {
        guard isAppForeground, !isDisconnecting else { return }
        let now = Date()
        guard now.timeIntervalSince(lastReconnectRequestedAt) > 5 else { return }
        lastReconnectRequestedAt = now
        onReconnectNeeded?(reason)
    }

    private func sendTelemetryJSON(_ object: [String: Any]) {
        guard telemetryChannel?.readyState == .open,
              let data = try? JSONSerialization.data(withJSONObject: object) else {
            return
        }
        let buffer = RTCDataBuffer(data: data, isBinary: false)
        _ = telemetryChannel?.sendData(buffer)
    }

    @discardableResult
    private func sendControlData(_ data: Data) -> Bool {
        guard controlChannel?.readyState == .open else {
            return false
        }
        let buffer = RTCDataBuffer(data: data, isBinary: false)
        return controlChannel?.sendData(buffer) ?? false
    }

    private func flushPendingControlMessages() {
        guard controlChannel?.readyState == .open else { return }
        let queued = pendingControlMessages
        pendingControlMessages.removeAll()
        for data in queued {
            _ = sendControlData(data)
        }
    }

    private func startKeepAlive() {
        keepAliveTask?.cancel()
        keepAliveTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(2))
                guard !Task.isCancelled else { return }
                self?.sendStreamControl(
                    foreground: true,
                    forceKeyframe: false,
                    snapshot: false
                )
            }
        }
    }

    private func startStatsReporting(simulatorID: String) {
        statsTask?.cancel()
        statsTask = Task { [weak self] in
            while !Task.isCancelled {
                self?.collectStats(simulatorID: simulatorID)
                try? await Task.sleep(for: .seconds(2))
            }
        }
    }

    private func startRenderWatchdog() {
        renderWatchdogTask?.cancel()
        renderWatchdogTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(750))
                guard let self, !Task.isCancelled, self.peerConnection != nil, self.isAppForeground else { return }
                let now = Date()
                let packetGap = now.timeIntervalSince(self.lastPacketReceivedAt)
                let renderedGap = now.timeIntervalSince(self.lastRenderedFrameAt)
                let decodedGap = now.timeIntervalSince(self.lastDecodedFrameAt)
                let noFirstFrame = self.renderedFrameCount == 0 && renderedGap > 1.5
                let hardStall = max(packetGap, max(renderedGap, decodedGap)) > 4
                if hardStall {
                    self.requestReconnect(reason: "stream-stalled")
                    continue
                }
                let recentUserActivity = now.timeIntervalSince(self.lastUserActivityAt) < 8
                let activeStall = recentUserActivity && (packetGap > 1.5 || max(renderedGap, decodedGap) > 2)
                guard (noFirstFrame || activeStall),
                      now.timeIntervalSince(self.lastStallRecoveryAt) > 3 else {
                    continue
                }
                self.lastStallRecoveryAt = now
                self.sendStreamControl(foreground: true, forceKeyframe: true, snapshot: true)
            }
        }
    }

    private func collectStats(simulatorID: String) {
        guard let peerConnection else { return }
        peerConnection.statistics { [weak self] report in
            guard let self else { return }
            let now = Date()
            var inboundVideo: RTCStatistics?
            var selectedPair: RTCStatistics?
            var codecsByID: [String: RTCStatistics] = [:]

            for (_, statistic) in report.statistics {
                if statistic.type == "codec" {
                    codecsByID[statistic.id] = statistic
                }
                if statistic.type == "inbound-rtp", statistic.mediaKind == "video" {
                    inboundVideo = statistic
                }
                if statistic.type == "candidate-pair",
                   statistic.stringValue("state") == "succeeded",
                   statistic.boolValue("nominated") == true {
                    selectedPair = statistic
                }
            }

            var stats: [String: Any] = [
                "clientId": self.clientID,
                "kind": "webrtc",
                "timestampMs": now.timeIntervalSince1970 * 1000,
                "udid": simulatorID,
                "status": self.peerConnectionState,
                "detail": "receiver-stats",
                "peerConnectionState": self.peerConnectionState,
                "iceConnectionState": self.iceConnectionState,
                "iceGatheringState": self.iceGatheringState,
                "signalingState": self.signalingState,
                "clientBundle": Bundle.main.bundleIdentifier ?? "dev.dj.simdeck.studio",
                "userAgent": "SimDeck Studio iOS"
            ]

            if let inboundVideo {
                let codecID = inboundVideo.stringValue("codecId")
                let codec = codecID.flatMap { codecsByID[$0]?.stringValue("mimeType") }
                let receivedPackets = inboundVideo.uintValue("packetsReceived") ?? 0
                let packetsLost = inboundVideo.uintValue("packetsLost") ?? 0
                let decodedFrames = inboundVideo.uintValue("framesDecoded") ?? inboundVideo.uintValue("framesReceived") ?? 0
                let droppedFrames = inboundVideo.uintValue("framesDropped") ?? 0
                if receivedPackets > self.lastPacketsReceived {
                    self.lastPacketsReceived = receivedPackets
                    self.lastPacketReceivedAt = now
                }
                if decodedFrames > self.lastDecodedFrames {
                    self.lastDecodedFrames = decodedFrames
                    self.lastDecodedFrameAt = now
                }
                let latestPacketGapMs = now.timeIntervalSince(self.lastPacketReceivedAt) * 1000
                let latestDecodedFrameGapMs = now.timeIntervalSince(self.lastDecodedFrameAt) * 1000
                let latestRenderedFrameGapMs = now.timeIntervalSince(self.lastRenderedFrameAt) * 1000
                let latestFrameGapMs = max(latestDecodedFrameGapMs, latestRenderedFrameGapMs)
                stats["codec"] = codec ?? "video"
                stats["receivedPackets"] = receivedPackets
                stats["packetsLost"] = packetsLost
                stats["decodedFrames"] = decodedFrames
                stats["decoderDroppedFrames"] = droppedFrames
                stats["droppedFrames"] = droppedFrames
                stats["latestPacketGapMs"] = latestPacketGapMs
                stats["latestFrameGapMs"] = latestFrameGapMs
                let recentUserActivity = now.timeIntervalSince(self.lastUserActivityAt) < 8
                let activeStall = recentUserActivity && (latestPacketGapMs > 1_500 || latestFrameGapMs > 2_000)
                let noFirstFrame = self.renderedFrameCount == 0 && latestFrameGapMs > 1_500
                if (noFirstFrame || activeStall),
                   now.timeIntervalSince(self.lastStallRecoveryAt) > 3 {
                    self.lastStallRecoveryAt = now
                    self.sendStreamControl(foreground: true, forceKeyframe: true, snapshot: true)
                }
                if let width = inboundVideo.uintValue("frameWidth") {
                    stats["width"] = width
                }
                if let height = inboundVideo.uintValue("frameHeight") {
                    stats["height"] = height
                }
                stats["renderedFrames"] = self.renderedFrameCount
                if let decodedFps = inboundVideo.doubleValue("framesPerSecond") {
                    stats["decodedFps"] = decodedFps
                }
                if let previousSampleAt = self.lastStatsSampleAt {
                    let elapsed = now.timeIntervalSince(previousSampleAt)
                    if elapsed > 0 {
                        stats["packetFps"] = Double(receivedPackets.saturatingDelta(from: self.lastStatsPacketsReceived)) / elapsed
                        stats["decodedFps"] = Double(decodedFrames.saturatingDelta(from: self.lastStatsDecodedFrames)) / elapsed
                        stats["appFps"] = Double(self.renderedFrameCount.saturatingDelta(from: self.lastStatsRenderedFrames)) / elapsed
                    }
                }
                self.lastStatsSampleAt = now
                self.lastStatsPacketsReceived = receivedPackets
                self.lastStatsDecodedFrames = decodedFrames
                self.lastStatsRenderedFrames = self.renderedFrameCount
            }
            if let selectedPair {
                let local = selectedPair.stringValue("localCandidateId") ?? "local"
                let remote = selectedPair.stringValue("remoteCandidateId") ?? "remote"
                stats["selectedCandidatePair"] = "\(local) -> \(remote)"
            }

            self.onDiagnostics?(StreamDiagnostics(stats: stats))
            self.sendTelemetryJSON(["type": "clientStats", "stats": stats])
        }
    }

    private static func isMoveControlMessage(_ object: [String: Any]) -> Bool {
        guard let type = object["type"] as? String,
              let phase = object["phase"] as? String else {
            return false
        }
        return phase == "moved" && (type == "touch" || type == "edgeTouch" || type == "multiTouch")
    }

    private func sendPageVisibilityStats(visible: Bool, simulatorID: String) {
        sendTelemetryJSON([
            "type": "clientStats",
            "stats": [
                "clientId": clientID,
                "kind": "page",
                "timestampMs": Date().timeIntervalSince1970 * 1000,
                "udid": simulatorID,
                "visibilityState": visible ? "visible" : "hidden",
                "focused": visible
            ]
        ])
    }

    private func iceServers(from health: HealthResponse) -> [RTCIceServer] {
        let servers = health.webRtc?.iceServers ?? [IceServer(urls: ["stun:stun.l.google.com:19302"])]
        return servers.map { server in
            RTCIceServer(
                urlStrings: server.urls,
                username: server.username,
                credential: server.credential
            )
        }
    }

    private func offer(for peerConnection: RTCPeerConnection) async throws -> RTCSessionDescription {
        let constraints = RTCMediaConstraints(
            mandatoryConstraints: [
                "OfferToReceiveVideo": "true",
                "OfferToReceiveAudio": "false"
            ],
            optionalConstraints: nil
        )
        return try await withCheckedThrowingContinuation { continuation in
            peerConnection.offer(for: constraints) { description, error in
                if let error {
                    continuation.resume(throwing: error)
                } else if let description {
                    continuation.resume(returning: description)
                } else {
                    continuation.resume(throwing: SimDeckAPIError.invalidResponse)
                }
            }
        }
    }

    private func setLocalDescription(_ description: RTCSessionDescription, on peerConnection: RTCPeerConnection) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            peerConnection.setLocalDescription(description) { error in
                if let error {
                    continuation.resume(throwing: error)
                } else {
                    continuation.resume()
                }
            }
        }
    }

    private func setRemoteDescription(_ description: RTCSessionDescription, on peerConnection: RTCPeerConnection) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            peerConnection.setRemoteDescription(description) { error in
                if let error {
                    continuation.resume(throwing: error)
                } else {
                    continuation.resume()
                }
            }
        }
    }

    private func waitForIceGathering(on peerConnection: RTCPeerConnection, timeout: TimeInterval) async {
        let deadline = Date().addingTimeInterval(timeout)
        while peerConnection.iceGatheringState != .complete && Date() < deadline {
            try? await Task.sleep(for: .milliseconds(50))
        }
    }

    private func attachRemoteTrack(_ track: RTCVideoTrack) {
        for renderer in renderers {
            remoteTrack?.remove(renderer)
            track.add(renderer)
        }
        remoteTrack = track
    }
}

extension WebRTCClient: RTCPeerConnectionDelegate {
    func peerConnection(_ peerConnection: RTCPeerConnection, didChange stateChanged: RTCSignalingState) {
        signalingState = stateChanged.statsLabel
    }

    func peerConnection(_ peerConnection: RTCPeerConnection, didAdd stream: RTCMediaStream) {
        if let track = stream.videoTracks.first {
            attachRemoteTrack(track)
        }
    }

    func peerConnection(_ peerConnection: RTCPeerConnection, didRemove stream: RTCMediaStream) {}

    func peerConnectionShouldNegotiate(_ peerConnection: RTCPeerConnection) {}

    func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCIceConnectionState) {
        iceConnectionState = newState.statsLabel
    }

    func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCIceGatheringState) {
        iceGatheringState = newState.statsLabel
    }

    func peerConnection(_ peerConnection: RTCPeerConnection, didGenerate candidate: RTCIceCandidate) {}

    func peerConnection(_ peerConnection: RTCPeerConnection, didRemove candidates: [RTCIceCandidate]) {}

    func peerConnection(_ peerConnection: RTCPeerConnection, didOpen dataChannel: RTCDataChannel) {
        dataChannel.delegate = self
    }

    func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCPeerConnectionState) {
        peerConnectionState = newState.statsLabel
        onConnectionState?(newState)
        switch newState {
        case .failed, .closed:
            requestReconnect(reason: "peer-\(newState.statsLabel)")
        case .disconnected:
            Task { [weak self] in
                try? await Task.sleep(for: .seconds(2))
                guard let self, self.peerConnectionState == "disconnected" else { return }
                self.requestReconnect(reason: "peer-disconnected")
            }
        default:
            break
        }
    }

    func peerConnection(_ peerConnection: RTCPeerConnection, didAdd rtpReceiver: RTCRtpReceiver, streams mediaStreams: [RTCMediaStream]) {
        if let track = rtpReceiver.track as? RTCVideoTrack {
            attachRemoteTrack(track)
        }
    }
}

private extension RTCStatistics {
    var mediaKind: String? {
        stringValue("kind") ?? stringValue("mediaType")
    }

    func stringValue(_ key: String) -> String? {
        values[key] as? String
    }

    func boolValue(_ key: String) -> Bool? {
        if let value = values[key] as? NSNumber {
            return value.boolValue
        }
        return values[key] as? Bool
    }

    func uintValue(_ key: String) -> UInt64? {
        if let value = values[key] as? NSNumber {
            return value.uint64Value
        }
        return nil
    }

    func doubleValue(_ key: String) -> Double? {
        if let value = values[key] as? NSNumber {
            return value.doubleValue
        }
        return nil
    }
}

private extension UInt64 {
    func saturatingDelta(from previous: UInt64) -> UInt64 {
        self >= previous ? self - previous : 0
    }
}

private extension RTCPeerConnectionState {
    var statsLabel: String {
        switch self {
        case .new: "new"
        case .connecting: "connecting"
        case .connected: "connected"
        case .disconnected: "disconnected"
        case .failed: "failed"
        case .closed: "closed"
        @unknown default: "unknown"
        }
    }
}

private extension RTCIceConnectionState {
    var statsLabel: String {
        switch self {
        case .new: "new"
        case .checking: "checking"
        case .connected: "connected"
        case .completed: "completed"
        case .failed: "failed"
        case .disconnected: "disconnected"
        case .closed: "closed"
        case .count: "count"
        @unknown default: "unknown"
        }
    }
}

private extension RTCIceGatheringState {
    var statsLabel: String {
        switch self {
        case .new: "new"
        case .gathering: "gathering"
        case .complete: "complete"
        @unknown default: "unknown"
        }
    }
}

private extension RTCSignalingState {
    var statsLabel: String {
        switch self {
        case .stable: "stable"
        case .haveLocalOffer: "have-local-offer"
        case .haveLocalPrAnswer: "have-local-pranswer"
        case .haveRemoteOffer: "have-remote-offer"
        case .haveRemotePrAnswer: "have-remote-pranswer"
        case .closed: "closed"
        @unknown default: "unknown"
        }
    }
}

extension WebRTCClient: RTCDataChannelDelegate {
    func dataChannelDidChangeState(_ dataChannel: RTCDataChannel) {
        if dataChannel.readyState == .open, dataChannel.label == "simdeck-control" {
            flushPendingControlMessages()
            sendStreamControl(foreground: true, forceKeyframe: true, snapshot: true)
            startKeepAlive()
        }
    }

    func dataChannel(_ dataChannel: RTCDataChannel, didReceiveMessageWith buffer: RTCDataBuffer) {
        guard !buffer.isBinary, let text = String(data: buffer.data, encoding: .utf8) else { return }
        onMessage?(text)
    }
}

extension URL {
    var isLoopbackOrLocal: Bool {
        guard let host = host(percentEncoded: false)?.lowercased() else { return false }
        return host == "localhost" || host == "127.0.0.1" || host == "::1" || host.hasSuffix(".local")
    }
}
