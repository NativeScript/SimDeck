import SwiftUI
import UIKit

struct SimulatorStreamView: View {
    @Bindable var model: AppModel
    @Environment(\.colorScheme) private var colorScheme
    @State private var activeTouchKind: StreamTouchKind?
    @State private var activeTouchIndicatorID: UUID?
    @State private var touchIndicators: [StreamTouchIndicator] = []
    @State private var touchOverlayRemovalTask: Task<Void, Never>?
    @State private var presentedSheet: StreamSheet?
    @State private var keyboardCaptureActive = false
    @State private var keyboardHeight: CGFloat = 0

    var body: some View {
        ZStack {
            if model.selectedSimulator == nil {
                ContentUnavailableView("No Simulator", systemImage: "iphone.slash")
            } else {
                streamViewport
            }

            KeyboardCaptureView(
                isActive: $keyboardCaptureActive,
                onText: { model.sendKeyboardText($0) },
                onDelete: { model.sendKeyboardBackspace() }
            )
            .frame(width: 1, height: 1)
            .opacity(0.01)
            .accessibilityHidden(true)
        }
        .navigationTitle("")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .principal) {
                StreamTitleButton(model: model) {
                    model.hapticSelection()
                    presentedSheet = .simulators
                }
            }
            ToolbarItem(placement: .topBarTrailing) {
                Menu {
                    Section("Stream") {
                        Text(model.streamConfig.summary)
                        Menu("Encoder") {
                            ForEach(StreamEncoder.allCases, id: \.self) { encoder in
                                Button {
                                    model.setStreamEncoder(encoder)
                                } label: {
                                    if model.streamConfig.encoder == encoder {
                                        Label(encoder.label, systemImage: "checkmark")
                                    } else {
                                        Text(encoder.label)
                                    }
                                }
                            }
                        }
                        Menu("Frame Rate") {
                            ForEach([15, 30, 60, 120], id: \.self) { fps in
                                Button {
                                    model.setStreamFPS(fps)
                                } label: {
                                    if model.streamConfig.fps == fps {
                                        Label("\(fps) fps", systemImage: "checkmark")
                                    } else {
                                        Text("\(fps) fps")
                                    }
                                }
                            }
                        }
                        Menu("Resolution") {
                            ForEach(StreamQualityPreset.allCases, id: \.self) { quality in
                                Button {
                                    model.setStreamQuality(quality)
                                } label: {
                                    if model.streamConfig.quality == quality {
                                        Label(quality.label, systemImage: "checkmark")
                                    } else {
                                        Text(quality.label)
                                    }
                                }
                            }
                        }
                    }
                    Section("Interaction") {
                        Toggle(isOn: Binding(
                            get: { model.touchOverlayVisible },
                            set: { model.setTouchOverlayVisible($0) }
                        )) {
                            Label("Show Touch Overlay", systemImage: "hand.tap")
                        }
                        Button {
                            model.hapticSelection()
                            presentedSheet = .debugInfo
                        } label: {
                            Label("Debug Info", systemImage: "info.circle")
                        }
                    }
                    Divider()
                    Button {
                        model.hapticSelection()
                        Task { await model.refreshSimulators() }
                    } label: {
                        Label("Refresh", systemImage: "arrow.clockwise")
                    }
                    Button {
                        if model.selectedSimulator?.isBooted == true {
                            model.hapticSelection()
                            Task { await model.startStream() }
                        } else {
                            Task { await model.bootSelectedSimulator() }
                        }
                    } label: {
                        Label(model.selectedSimulator?.isBooted == true ? "Start Stream" : "Boot", systemImage: "play.circle")
                    }
                    .disabled(model.selectedSimulatorID == nil || model.endpoint == nil)
                    Button {
                        model.stopStream()
                    } label: {
                        Label("Stop", systemImage: "stop.circle")
                    }
                    .disabled(!model.canStopStream)
                } label: {
                    Label("Stream Settings", systemImage: "gearshape")
                }
            }
        }
        .sheet(item: $presentedSheet) { sheet in
            switch sheet {
            case .simulators:
                StreamSimulatorSelectionSheet(model: model)
            case .debugInfo:
                StreamDebugInfoSheet(model: model)
            }
        }
        .safeAreaInset(edge: .bottom) {
            if model.selectedSimulator?.isBooted == true {
                StreamControlBar(model: model, keyboardCaptureActive: $keyboardCaptureActive)
            }
        }
        .onChange(of: model.selectedSimulatorID) { _, _ in
            keyboardCaptureActive = false
            clearTouchOverlay()
        }
        .onChange(of: model.selectedSimulator?.isBooted == true) { _, isBooted in
            if !isBooted {
                keyboardCaptureActive = false
                clearTouchOverlay()
            }
        }
        .onChange(of: model.touchOverlayVisible) { _, isVisible in
            if !isVisible {
                clearTouchOverlay()
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillChangeFrameNotification)) { notification in
            updateKeyboardHeight(notification)
        }
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillHideNotification)) { notification in
            updateKeyboardHeight(notification)
        }
    }

    private var streamViewport: some View {
        GeometryReader { proxy in
            let layout = DeviceViewportLayout(
                chromeProfile: model.chromeProfile,
                videoSize: model.videoSize,
                availableSize: proxy.size
            )
            let displayToken = model.streamDisplayToken
            let screenMaskImage = model.chromeProfile?.hasScreenMask == true ? model.chromeScreenMask : nil

            ZStack(alignment: .topLeading) {
                streamBackground

                Rectangle()
                    .fill(.black)
                    .frame(width: layout.screenBackingFrame.width, height: layout.screenBackingFrame.height)
                    .clippedToSimulatorScreen(cornerRadius: layout.screenCornerRadius + 2, maskImage: screenMaskImage)
                    .position(x: layout.screenBackingFrame.midX, y: layout.screenBackingFrame.midY)

                if showsCachedStreamFrame, let lastStreamFrame = model.lastStreamFrame {
                    CachedStreamFrameView(
                        image: lastStreamFrame,
                        cornerRadius: layout.screenCornerRadius + 1,
                        maskImage: screenMaskImage
                    )
                        .frame(width: layout.videoFrame.width, height: layout.videoFrame.height)
                        .position(x: layout.videoFrame.midX, y: layout.videoFrame.midY)
                        .transition(.opacity)
                }

                if model.selectedSimulator?.isBooted == true, model.currentStreamClient != nil {
                    WebRTCVideoView(
                        client: model.currentStreamClient,
                        onVideoSize: { size in
                            model.videoSize = size
                        },
                        onFrameRendered: {
                            model.markStreamFrameRendered(displayToken: displayToken)
                        },
                        onFrameSnapshot: { image in
                            model.updateLastStreamFrame(image, displayToken: displayToken)
                        }
                    )
                    .id(displayToken)
                    .frame(width: layout.videoFrame.width, height: layout.videoFrame.height)
                    .clippedToSimulatorScreen(cornerRadius: layout.screenCornerRadius + 1, maskImage: screenMaskImage)
                    .position(x: layout.videoFrame.midX, y: layout.videoFrame.midY)
                    .opacity(model.hasCurrentStreamFrame ? 1 : 0)
                }

                if let chromeImage = model.chromeImage, layout.usesChrome {
                    Image(uiImage: chromeImage)
                        .resizable()
                        .interpolation(.high)
                        .frame(width: layout.shellFrame.width, height: layout.shellFrame.height)
                        .position(x: layout.shellFrame.midX, y: layout.shellFrame.midY)
                        .shadow(color: .black.opacity(0.22), radius: 18, y: 10)
                        .allowsHitTesting(false)
                }

                if model.selectedSimulator?.isBooted == true,
                   let chromeProfile = model.chromeProfile,
                   layout.usesChrome {
                    HardwareButtonLayer(model: model, chromeProfile: chromeProfile, layout: layout)
                }

                if model.selectedSimulator?.isBooted == true,
                   model.touchOverlayVisible,
                   !touchIndicators.isEmpty {
                    TouchInteractionOverlay(indicators: touchIndicators)
                        .frame(width: layout.screenFrame.width, height: layout.screenFrame.height)
                        .clippedToSimulatorScreen(cornerRadius: layout.screenCornerRadius, maskImage: screenMaskImage)
                        .position(x: layout.screenFrame.midX, y: layout.screenFrame.midY)
                        .allowsHitTesting(false)
                        .transition(.opacity)
                }

                if let simulator = model.selectedSimulator, !simulator.isBooted {
                    BootSimulatorOverlay(model: model, simulator: simulator)
                        .frame(width: layout.screenFrame.width, height: layout.screenFrame.height)
                        .clippedToSimulatorScreen(cornerRadius: layout.screenCornerRadius, maskImage: screenMaskImage)
                        .position(x: layout.screenFrame.midX, y: layout.screenFrame.midY)
                }

                if showsFirstFrameSpinner {
                    StreamFirstFrameLoadingOverlay()
                        .frame(width: layout.screenFrame.width, height: layout.screenFrame.height)
                        .clippedToSimulatorScreen(cornerRadius: layout.screenCornerRadius, maskImage: screenMaskImage)
                        .position(x: layout.screenFrame.midX, y: layout.screenFrame.midY)
                        .transition(.opacity)
                }

                if showsRetryOverlay {
                    StreamRetryOverlay(model: model)
                        .frame(width: layout.screenFrame.width, height: layout.screenFrame.height)
                        .clippedToSimulatorScreen(cornerRadius: layout.screenCornerRadius, maskImage: screenMaskImage)
                        .position(x: layout.screenFrame.midX, y: layout.screenFrame.midY)
                        .transition(.opacity)
                }
            }
            .contentShape(Rectangle())
            .streamTouchGesture(model.selectedSimulator?.isBooted == true, gesture: touchGesture(in: layout.screenFrame))
            .animation(.snappy(duration: 0.3), value: keyboardCaptureActive)
            .animation(.smooth(duration: 0.28), value: keyboardHeight)
        }
        .background(streamBackground)
    }

    private var streamBackground: Color {
        colorScheme == .dark ? Color(.systemBackground) : Color(.secondarySystemGroupedBackground)
    }

    private var showsFirstFrameSpinner: Bool {
        guard model.selectedSimulator?.isBooted == true else { return false }
        return model.streamState == .connecting
            || (model.currentStreamClient != nil && !model.hasCurrentStreamFrame)
    }

    private var showsCachedStreamFrame: Bool {
        guard model.selectedSimulator?.isBooted == true else { return false }
        return model.lastStreamFrame != nil && !model.hasCurrentStreamFrame
    }

    private var showsRetryOverlay: Bool {
        guard model.selectedSimulator?.isBooted == true else { return false }
        return model.streamState == .failed || model.streamState == .disconnected
    }

    private func touchGesture(in screenFrame: CGRect) -> some Gesture {
        DragGesture(minimumDistance: 0)
            .onChanged { value in
                if activeTouchKind == nil {
                    guard screenFrame.contains(value.startLocation),
                          let point = model.normalizedTouchPoint(location: value.startLocation, in: screenFrame) else {
                        return
                    }
                    activeTouchKind = point.y >= 0.93 ? .bottomEdge : .single
                    sendActiveTouch(location: value.location, in: screenFrame, phase: "began")
                    return
                }
                sendActiveTouch(location: value.location, in: screenFrame, phase: "moved")
            }
            .onEnded { value in
                sendActiveTouch(location: value.location, in: screenFrame, phase: "ended")
                activeTouchKind = nil
            }
    }

    private func sendActiveTouch(location: CGPoint, in screenFrame: CGRect, phase: String) {
        updateTouchOverlay(location: location, in: screenFrame, phase: phase)
        switch activeTouchKind {
        case .bottomEdge:
            model.sendEdgeTouch(location: location, in: screenFrame, phase: phase, edge: "bottom")
        case .single:
            model.sendTouch(location: location, in: screenFrame, phase: phase)
        case nil:
            break
        }
    }

    private func updateTouchOverlay(location: CGPoint, in screenFrame: CGRect, phase: String) {
        guard model.touchOverlayVisible else {
            clearTouchOverlay()
            return
        }

        let clampedLocation = clampedTouchPoint(location, in: screenFrame)
        switch phase {
        case "began":
            touchOverlayRemovalTask?.cancel()
            let id = UUID()
            activeTouchIndicatorID = id
            withAnimation(.snappy(duration: 0.12)) {
                touchIndicators = [
                    StreamTouchIndicator(id: id, start: clampedLocation, current: clampedLocation, isEnding: false)
                ]
            }
        case "moved":
            guard let activeTouchIndicatorID,
                  let index = touchIndicators.firstIndex(where: { $0.id == activeTouchIndicatorID }) else {
                return
            }
            touchIndicators[index].current = clampedLocation
        case "ended":
            guard let activeTouchIndicatorID,
                  let index = touchIndicators.firstIndex(where: { $0.id == activeTouchIndicatorID }) else {
                return
            }
            let endingID = activeTouchIndicatorID
            touchIndicators[index].current = clampedLocation
            touchIndicators[index].isEnding = true
            self.activeTouchIndicatorID = nil
            touchOverlayRemovalTask?.cancel()
            touchOverlayRemovalTask = Task { @MainActor in
                try? await Task.sleep(for: .milliseconds(240))
                withAnimation(.easeOut(duration: 0.16)) {
                    touchIndicators.removeAll { $0.id == endingID }
                }
            }
        default:
            break
        }
    }

    private func clampedTouchPoint(_ location: CGPoint, in screenFrame: CGRect) -> CGPoint {
        CGPoint(
            x: min(max(location.x - screenFrame.minX, 0), screenFrame.width),
            y: min(max(location.y - screenFrame.minY, 0), screenFrame.height)
        )
    }

    private func clearTouchOverlay() {
        touchOverlayRemovalTask?.cancel()
        touchOverlayRemovalTask = nil
        activeTouchIndicatorID = nil
        touchIndicators = []
    }

    private func updateKeyboardHeight(_ notification: Notification) {
        let endFrame = (notification.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? NSValue)?.cgRectValue ?? .zero
        let duration = (notification.userInfo?[UIResponder.keyboardAnimationDurationUserInfoKey] as? NSNumber)?.doubleValue ?? 0.28
        let height = notification.name == UIResponder.keyboardWillHideNotification
            ? 0
            : max(0, UIScreen.main.bounds.height - endFrame.minY)
        withAnimation(.easeOut(duration: duration)) {
            keyboardHeight = height
        }
        if height <= 1 {
            keyboardCaptureActive = false
        }
    }
}

private enum StreamTouchKind {
    case single
    case bottomEdge
}

private enum StreamSheet: Identifiable {
    case simulators
    case debugInfo

    var id: Self { self }
}

private struct StreamTouchIndicator: Identifiable, Equatable {
    let id: UUID
    var start: CGPoint
    var current: CGPoint
    var isEnding: Bool
}

private struct TouchInteractionOverlay: View {
    let indicators: [StreamTouchIndicator]

    var body: some View {
        ZStack(alignment: .topLeading) {
            ForEach(indicators) { indicator in
                Path { path in
                    path.move(to: indicator.start)
                    path.addLine(to: indicator.current)
                }
                .stroke(.white.opacity(indicator.isEnding ? 0.25 : 0.62), style: StrokeStyle(lineWidth: 4, lineCap: .round))
                .shadow(color: .black.opacity(0.28), radius: 4)

                Circle()
                    .fill(.white.opacity(indicator.isEnding ? 0.18 : 0.36))
                    .stroke(.white.opacity(indicator.isEnding ? 0.36 : 0.86), lineWidth: 2)
                    .frame(width: indicator.isEnding ? 34 : 42, height: indicator.isEnding ? 34 : 42)
                    .position(x: indicator.current.x, y: indicator.current.y)
                    .shadow(color: .black.opacity(0.3), radius: 7)
                    .scaleEffect(indicator.isEnding ? 0.82 : 1)
            }
        }
        .compositingGroup()
        .accessibilityHidden(true)
    }
}

private struct BootSimulatorOverlay: View {
    @Bindable var model: AppModel
    let simulator: SimulatorMetadata

    var body: some View {
        ZStack {
            Color.black.opacity(0.08)
            Button {
                Task { await model.bootSelectedSimulator() }
            } label: {
                ZStack {
                    if model.isSelectedSimulatorBooting {
                        ProgressView()
                            .controlSize(.regular)
                            .tint(.white)
                    } else {
                        Image(systemName: "play.fill")
                            .font(.title2.weight(.semibold))
                            .foregroundStyle(.white)
                            .offset(x: 2)
                    }
                }
                .frame(width: 72, height: 72)
                .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .disabled(model.isSelectedSimulatorBooting)
            .modifier(StreamGlassCircleModifier(interactive: !model.isSelectedSimulatorBooting))
            .accessibilityLabel(model.isSelectedSimulatorBooting ? "Booting \(simulator.name)" : "Boot \(simulator.name)")
        }
    }
}

private struct StreamFirstFrameLoadingOverlay: View {
    var body: some View {
        ZStack {
            Color.clear
            ProgressView()
                .controlSize(.small)
                .tint(.white)
        }
        .allowsHitTesting(false)
        .accessibilityLabel("Loading stream")
    }
}

private struct CachedStreamFrameView: View {
    let image: UIImage
    let cornerRadius: CGFloat
    let maskImage: UIImage?

    var body: some View {
        Image(uiImage: image)
            .resizable()
            .scaledToFill()
            .saturation(0.82)
            .brightness(-0.08)
            .overlay(Color.black.opacity(0.28))
            .clippedToSimulatorScreen(cornerRadius: cornerRadius, maskImage: maskImage)
            .shadow(color: .black.opacity(0.34), radius: 16, y: 8)
            .clipped()
            .allowsHitTesting(false)
            .accessibilityHidden(true)
    }
}

private struct StreamRetryOverlay: View {
    @Bindable var model: AppModel

    var body: some View {
        ZStack {
            Color.black.opacity(0.06)
            VStack(spacing: 10) {
                Button {
                    model.retryStream()
                } label: {
                    Image(systemName: "arrow.clockwise")
                        .font(.title3.weight(.semibold))
                        .foregroundStyle(.white)
                        .frame(width: 52, height: 52)
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
                .modifier(StreamGlassCircleModifier(interactive: true))
                .accessibilityLabel("Retry Stream")

                Text("Retry")
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.white.opacity(0.88))
            }
        }
    }
}

private struct StreamTitleButton: View {
    @Bindable var model: AppModel
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Circle()
                    .fill(statusColor)
                    .frame(width: 7, height: 7)
                Spacer(minLength: 4)
                VStack(alignment: .center, spacing: 1) {
                    Text(model.selectedSimulator?.name ?? "Select Simulator")
                        .font(.headline)
                        .lineLimit(1)
                        .multilineTextAlignment(.center)
                    Text(model.streamNavigationSubtitle)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .multilineTextAlignment(.center)
                }
                Spacer(minLength: 4)
                Image(systemName: "chevron.down")
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(.secondary)
            }
            .padding(.leading, 14)
            .padding(.trailing, 12)
            .padding(.vertical, 5)
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .frame(minWidth: 190, maxWidth: 260)
        .frame(height: 42)
        .modifier(StreamGlassCapsuleModifier(interactive: true))
        .accessibilityElement(children: .combine)
    }

    private var statusColor: Color {
        if let selectedSimulator = model.selectedSimulator, !selectedSimulator.isBooted {
            return model.isSelectedSimulatorBooting ? .orange : .secondary
        }
        switch model.streamState {
        case .connected:
            return .green
        case .connecting:
            return .orange
        case .failed:
            return .red
        case .disconnected, .idle:
            return .secondary
        }
    }
}

private struct StreamSimulatorSelectionSheet: View {
    @Bindable var model: AppModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                ForEach(model.simulators) { simulator in
                    Button {
                        model.hapticSelection()
                        model.selectSimulator(simulator.udid)
                        dismiss()
                    } label: {
                        HStack(spacing: 12) {
                            SimulatorRow(simulator: simulator)
                            Spacer()
                            if model.selectedSimulatorID == simulator.udid {
                                Image(systemName: "checkmark")
                                    .font(.headline)
                                    .foregroundStyle(.tint)
                            }
                        }
                    }
                    .buttonStyle(.plain)
                }
            }
            .navigationTitle("Simulators")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") {
                        model.hapticSelection()
                        dismiss()
                    }
                }
                ToolbarItem(placement: .primaryAction) {
                    Button {
                        model.hapticSelection()
                        Task { await model.refreshSimulators() }
                    } label: {
                        Label("Refresh", systemImage: "arrow.clockwise")
                    }
                }
            }
        }
        .presentationDetents([.medium, .large])
    }
}

private struct StreamDebugInfoSheet: View {
    @Bindable var model: AppModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                Section("Stream") {
                    DebugInfoRow("State", value: model.streamState.rawValue)
                    DebugInfoRow("FPS", value: formattedDecimal(model.streamDiagnostics.renderedFps))
                    DebugInfoRow("Decoded FPS", value: formattedDecimal(model.streamDiagnostics.decodedFps))
                    DebugInfoRow("Packet FPS", value: formattedDecimal(model.streamDiagnostics.packetFps))
                    DebugInfoRow("Resolution", value: resolution)
                    DebugInfoRow("Path", value: "webrtc")
                    DebugInfoRow("Config", value: model.streamConfig.summary)
                    DebugInfoRow("Codec", value: model.streamDiagnostics.codec.nilIfBlank ?? "-")
                }

                Section("Frames") {
                    DebugInfoRow("Packets", value: "\(model.streamDiagnostics.receivedPackets)")
                    DebugInfoRow("Packet Loss", value: "\(model.streamDiagnostics.packetsLost)")
                    DebugInfoRow("Decoded", value: "\(model.streamDiagnostics.decodedFrames)")
                    DebugInfoRow("Rendered", value: "\(model.streamDiagnostics.renderedFrames)")
                    DebugInfoRow("Decode Drops", value: "\(model.streamDiagnostics.decoderDroppedFrames)")
                    DebugInfoRow("Present Drops", value: "\(model.streamDiagnostics.presentationDroppedFrames)")
                    DebugInfoRow("Frame Gap", value: formattedMilliseconds(model.streamDiagnostics.latestFrameGapMs))
                    DebugInfoRow("Packet Gap", value: formattedMilliseconds(model.streamDiagnostics.latestPacketGapMs))
                }

                Section("Connection") {
                    DebugInfoRow("Peer", value: model.streamDiagnostics.peerConnectionState.nilIfBlank ?? "-")
                    DebugInfoRow("ICE", value: model.streamDiagnostics.iceConnectionState.nilIfBlank ?? "-")
                    DebugInfoRow("Gathering", value: model.streamDiagnostics.iceGatheringState.nilIfBlank ?? "-")
                    DebugInfoRow("Signaling", value: model.streamDiagnostics.signalingState.nilIfBlank ?? "-")
                    DebugInfoRow("Reconnects", value: "\(model.streamReconnects)")
                    DebugInfoRow("Reconnect Reason", value: model.streamReconnectReason.nilIfBlank ?? "-")
                    DebugInfoRow("Candidate Pair", value: model.streamDiagnostics.selectedCandidatePair.nilIfBlank ?? "-")
                }

                Section("Target") {
                    DebugInfoRow("Server", value: model.endpoint?.baseURL.absoluteString ?? "-")
                    DebugInfoRow("Simulator", value: model.selectedSimulator?.name ?? "-")
                    DebugInfoRow("UDID", value: model.selectedSimulatorID ?? "-")
                    DebugInfoRow("Updated", value: model.streamDiagnostics.timestamp.formatted(date: .omitted, time: .standard))
                }
            }
            .navigationTitle("Debug Info")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") {
                        model.hapticSelection()
                        dismiss()
                    }
                }
            }
        }
        .presentationDetents([.medium, .large])
    }

    private var resolution: String {
        let diagnostics = model.streamDiagnostics
        if diagnostics.width > 0, diagnostics.height > 0 {
            return "\(diagnostics.width)x\(diagnostics.height)"
        }
        if model.videoSize.width > 0, model.videoSize.height > 0 {
            return "\(Int(model.videoSize.width))x\(Int(model.videoSize.height))"
        }
        return "-"
    }

    private func formattedDecimal(_ value: Double) -> String {
        guard value.isFinite else { return "0.0" }
        return value.formatted(.number.precision(.fractionLength(1)))
    }

    private func formattedMilliseconds(_ value: Double) -> String {
        guard value.isFinite, value > 0 else { return "-" }
        return "\(value.formatted(.number.precision(.fractionLength(1)))) ms"
    }
}

private struct DebugInfoRow: View {
    let title: LocalizedStringKey
    let value: String

    init(_ title: LocalizedStringKey, value: String) {
        self.title = title
        self.value = value
    }

    var body: some View {
        LabeledContent(title) {
            Text(value)
                .fontDesign(.monospaced)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.trailing)
                .textSelection(.enabled)
        }
    }
}

private struct StreamBadge: View {
    let state: StreamState
    let size: CGSize

    var body: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(color)
                .frame(width: 8, height: 8)
            Text(label)
                .font(.caption)
                .monospacedDigit()
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(.thinMaterial, in: Capsule())
    }

    private var label: String {
        if size.width > 0, size.height > 0 {
            "\(state.rawValue) \(Int(size.width))x\(Int(size.height))"
        } else {
            state.rawValue
        }
    }

    private var color: Color {
        switch state {
        case .connected: .green
        case .connecting: .orange
        case .failed: .red
        case .disconnected: .secondary
        case .idle: .secondary
        }
    }
}

private struct StreamControlBar: View {
    @Bindable var model: AppModel
    @Binding var keyboardCaptureActive: Bool

    var body: some View {
        if #available(iOS 26.0, *) {
            LiquidGlassStreamControlBar(model: model, keyboardCaptureActive: $keyboardCaptureActive)
        } else {
            LegacyStreamControlBar(model: model, keyboardCaptureActive: $keyboardCaptureActive)
        }
    }
}

@available(iOS 26.0, *)
private struct LiquidGlassStreamControlBar: View {
    @Bindable var model: AppModel
    @Binding var keyboardCaptureActive: Bool

    var body: some View {
        GlassEffectContainer(spacing: 14) {
            StreamControlButtons(model: model, keyboardCaptureActive: $keyboardCaptureActive)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }
}

private struct LegacyStreamControlBar: View {
    @Bindable var model: AppModel
    @Binding var keyboardCaptureActive: Bool

    var body: some View {
        StreamControlButtons(model: model, keyboardCaptureActive: $keyboardCaptureActive)
            .buttonStyle(StreamToolbarButtonStyle())
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
    }
}

private struct StreamControlButtons: View {
    @Bindable var model: AppModel
    @Binding var keyboardCaptureActive: Bool

    var body: some View {
        HStack(spacing: 8) {
            StreamHardwareControlButton("Home", systemImage: "house", buttonName: "home", model: model)

            StreamControlButton("Switcher", systemImage: "square.on.square") { model.sendAppSwitcher() }

            Spacer(minLength: 4)

            StreamControlButton("Appearance", systemImage: "circle.lefthalf.filled") { model.toggleAppearance() }

            StreamControlButton("Rotate Right", systemImage: "rotate.right") { model.rotateRight() }

            Spacer(minLength: 4)

            StreamHardwareControlButton("Lock", systemImage: "lock", buttonName: "power", model: model)

            StreamKeyboardControlButton(model: model, isActive: $keyboardCaptureActive)
        }
    }
}

private struct StreamControlButton: View {
    let title: LocalizedStringKey
    let systemImage: String
    let action: () -> Void

    init(_ title: LocalizedStringKey, systemImage: String, action: @escaping () -> Void) {
        self.title = title
        self.systemImage = systemImage
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            StreamControlIconLabel(title: title, systemImage: systemImage)
        }
        .buttonStyle(.plain)
        .buttonBorderShape(.circle)
    }
}

private struct StreamHardwareControlButton: View {
    let title: LocalizedStringKey
    let systemImage: String
    let buttonName: String
    @Bindable var model: AppModel
    @State private var isPressed = false

    init(_ title: LocalizedStringKey, systemImage: String, buttonName: String, model: AppModel) {
        self.title = title
        self.systemImage = systemImage
        self.buttonName = buttonName
        self.model = model
    }

    var body: some View {
        StreamControlIconLabel(title: title, systemImage: systemImage)
            .opacity(isPressed ? 0.45 : 1)
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { _ in pressDown() }
                    .onEnded { _ in pressUp() }
            )
            .onDisappear {
                pressUp()
            }
            .accessibilityLabel(title)
            .accessibilityAddTraits(.isButton)
            .accessibilityAction {
                model.tapHardwareButton(named: buttonName)
            }
    }

    private func pressDown() {
        guard !isPressed else { return }
        isPressed = true
        model.sendHardwareButton(named: buttonName, phase: .down)
    }

    private func pressUp() {
        guard isPressed else { return }
        isPressed = false
        model.sendHardwareButton(named: buttonName, phase: .up)
    }
}

private struct StreamKeyboardControlButton: View {
    @Bindable var model: AppModel
    @Binding var isActive: Bool

    var body: some View {
        Button {
            model.hapticSelection()
            withAnimation(.snappy(duration: 0.25)) {
                isActive.toggle()
            }
            if !isActive {
                model.dismissSimulatorKeyboard()
            }
        } label: {
            StreamControlIconLabel(title: "Keyboard", systemImage: "keyboard")
                .opacity(isActive ? 1 : 0.86)
                .scaleEffect(isActive ? 1.04 : 1)
        }
        .buttonStyle(.plain)
        .buttonBorderShape(.circle)
        .accessibilityLabel("Keyboard")
        .accessibilityValue(isActive ? "Active" : "Inactive")
    }
}

private struct StreamControlIconLabel: View {
    let title: LocalizedStringKey
    let systemImage: String

    @ViewBuilder
    var body: some View {
        let content = Label(title, systemImage: systemImage)
            .labelStyle(.iconOnly)
            .font(.body)
            .foregroundStyle(.primary)
            .frame(width: 44, height: 44)
            .contentShape(Circle())
        if #available(iOS 26.0, *) {
            content
                .glassEffect(.regular.interactive(), in: .circle)
        } else {
            content
                .background(.ultraThinMaterial, in: Circle())
        }
    }
}

private struct HardwareButtonLayer: View {
    @Bindable var model: AppModel
    let chromeProfile: ChromeProfile
    let layout: DeviceViewportLayout

    var body: some View {
        ForEach(chromeProfile.buttons ?? [], id: \.self) { button in
            if let buttonName = button.hardwareWireName, button.width > 0, button.height > 0 {
                HardwareButtonHitArea(
                    model: model,
                    button: button,
                    buttonName: buttonName,
                    frame: layout.chromeButtonFrame(button)
                )
            }
        }
    }
}

private struct HardwareButtonHitArea: View {
    @Bindable var model: AppModel
    let button: ChromeButtonProfile
    let buttonName: String
    let frame: CGRect
    @State private var isPressed = false

    var body: some View {
        Color.clear
            .frame(width: hitFrame.width, height: hitFrame.height)
            .contentShape(Rectangle())
            .position(x: hitFrame.midX, y: hitFrame.midY)
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { _ in pressDown() }
                    .onEnded { _ in pressUp() }
            )
            .onDisappear {
                pressUp()
            }
            .accessibilityLabel(Text(button.label ?? button.name))
            .accessibilityAddTraits(.isButton)
            .accessibilityAction {
                model.tapHardwareButton(named: buttonName, usagePage: button.usagePage, usage: button.usage)
            }
    }

    private var hitFrame: CGRect {
        let minimumTarget: CGFloat = 34
        let width = max(frame.width, minimumTarget)
        let height = max(frame.height, minimumTarget)
        return CGRect(
            x: frame.midX - width / 2,
            y: frame.midY - height / 2,
            width: width,
            height: height
        )
    }

    private func pressDown() {
        guard !isPressed else { return }
        isPressed = true
        model.sendHardwareButton(
            named: buttonName,
            phase: .down,
            usagePage: button.usagePage,
            usage: button.usage
        )
    }

    private func pressUp() {
        guard isPressed else { return }
        isPressed = false
        model.sendHardwareButton(
            named: buttonName,
            phase: .up,
            usagePage: button.usagePage,
            usage: button.usage
        )
    }
}

private struct KeyboardCaptureView: UIViewRepresentable {
    @Binding var isActive: Bool
    let onText: (String) -> Void
    let onDelete: () -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(isActive: $isActive)
    }

    func makeUIView(context: Context) -> KeyboardCaptureTextView {
        let view = KeyboardCaptureTextView()
        view.delegate = context.coordinator
        view.backgroundColor = .clear
        view.tintColor = .clear
        view.textColor = .clear
        view.isScrollEnabled = false
        view.autocorrectionType = .no
        view.autocapitalizationType = .none
        view.spellCheckingType = .no
        view.smartDashesType = .no
        view.smartInsertDeleteType = .no
        view.smartQuotesType = .no
        view.keyboardType = .default
        view.returnKeyType = .default
        view.textContentType = nil
        view.textContainerInset = .zero
        view.textContainer.lineFragmentPadding = 0
        view.inputAssistantItem.leadingBarButtonGroups = []
        view.inputAssistantItem.trailingBarButtonGroups = []
        return view
    }

    func updateUIView(_ view: KeyboardCaptureTextView, context: Context) {
        view.onText = onText
        view.onDelete = onDelete
        if isActive, !view.isFirstResponder {
            DispatchQueue.main.async {
                view.becomeFirstResponder()
            }
        } else if !isActive, view.isFirstResponder {
            DispatchQueue.main.async {
                view.resignFirstResponder()
            }
        }
    }

    final class Coordinator: NSObject, UITextViewDelegate {
        var isActive: Binding<Bool>

        init(isActive: Binding<Bool>) {
            self.isActive = isActive
        }

        func textViewDidEndEditing(_ textView: UITextView) {
            isActive.wrappedValue = false
        }
    }
}

private final class KeyboardCaptureTextView: UITextView {
    var onText: ((String) -> Void)?
    var onDelete: (() -> Void)?

    override var canBecomeFirstResponder: Bool {
        true
    }

    override var hasText: Bool {
        true
    }

    override func insertText(_ text: String) {
        onText?(text)
    }

    override func deleteBackward() {
        onDelete?()
    }

    override func paste(_ sender: Any?) {
        guard let text = UIPasteboard.general.string, !text.isEmpty else { return }
        onText?(text)
    }
}

private struct StreamGlassCapsuleModifier: ViewModifier {
    let interactive: Bool

    func body(content: Content) -> some View {
        if #available(iOS 26.0, *) {
            if interactive {
                content.glassEffect(.regular.interactive(), in: .capsule)
            } else {
                content.glassEffect(.regular, in: .capsule)
            }
        } else {
            content.background(.ultraThinMaterial, in: Capsule())
        }
    }
}

private struct StreamGlassCircleModifier: ViewModifier {
    let interactive: Bool

    func body(content: Content) -> some View {
        if #available(iOS 26.0, *) {
            if interactive {
                content.glassEffect(.regular.interactive(), in: .circle)
            } else {
                content.glassEffect(.regular, in: .circle)
            }
        } else {
            content.background(.ultraThinMaterial, in: Circle())
        }
    }
}

private struct StreamToolbarButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(.primary)
            .frame(width: 44, height: 44)
            .contentShape(Rectangle())
            .opacity(configuration.isPressed ? 0.45 : 1)
    }
}

private struct DeviceViewportLayout {
    let shellFrame: CGRect
    let screenFrame: CGRect
    let screenBackingFrame: CGRect
    let videoFrame: CGRect
    let screenCornerRadius: CGFloat
    let usesChrome: Bool
    private let chromeCoordinateScale: CGFloat

    init(chromeProfile: ChromeProfile?, videoSize: CGSize, availableSize: CGSize) {
        let viewport = CGRect(origin: .zero, size: availableSize)
            .insetBy(dx: min(20, availableSize.width * 0.045), dy: 16)

        if let chromeProfile,
           chromeProfile.totalWidth > 0,
           chromeProfile.totalHeight > 0,
           chromeProfile.screenWidth > 0,
           chromeProfile.screenHeight > 0,
           viewport.width > 0,
           viewport.height > 0 {
            let profileSize = CGSize(width: CGFloat(chromeProfile.totalWidth), height: CGFloat(chromeProfile.totalHeight))
            let shell = profileSize.aspectFit(in: viewport)
            let scale = shell.width / profileSize.width
            let screenRect = Self.chromeScreenRect(profile: chromeProfile)
            shellFrame = shell
            chromeCoordinateScale = scale
            screenFrame = CGRect(
                x: shell.minX + screenRect.minX * scale,
                y: shell.minY + screenRect.minY * scale,
                width: screenRect.width * scale,
                height: screenRect.height * scale
            )
            screenBackingFrame = screenFrame.insetBy(dx: -2, dy: -2)
            videoFrame = screenFrame
            screenCornerRadius = Self.screenCornerRadius(
                profile: chromeProfile,
                profileScreenRect: screenRect,
                scale: scale
            )
            usesChrome = true
            return
        }

        let fallbackSize = videoSize.width > 0 && videoSize.height > 0
            ? videoSize
            : CGSize(width: 440, height: 956)
        let screen = fallbackSize.aspectFit(in: viewport)
        shellFrame = screen
        screenFrame = screen
        screenBackingFrame = screen
        videoFrame = screen
        screenCornerRadius = min(44, screen.width * 0.14)
        usesChrome = false
        chromeCoordinateScale = 1
    }

    func chromeButtonFrame(_ button: ChromeButtonProfile) -> CGRect {
        guard usesChrome else { return .zero }
        return CGRect(
            x: shellFrame.minX + CGFloat(button.x) * chromeCoordinateScale,
            y: shellFrame.minY + CGFloat(button.y) * chromeCoordinateScale,
            width: CGFloat(button.width) * chromeCoordinateScale,
            height: CGFloat(button.height) * chromeCoordinateScale
        )
    }

    private static func chromeScreenRect(profile: ChromeProfile) -> CGRect {
        CGRect(
            x: CGFloat(profile.screenX),
            y: CGFloat(profile.screenY),
            width: CGFloat(profile.screenWidth),
            height: CGFloat(profile.screenHeight)
        )
    }

    private static func screenCornerRadius(profile: ChromeProfile, profileScreenRect: CGRect, scale: CGFloat) -> CGFloat {
        let fullScreen = CGRect(
            x: CGFloat(profile.screenX),
            y: CGFloat(profile.screenY),
            width: CGFloat(profile.screenWidth),
            height: CGFloat(profile.screenHeight)
        )
        guard abs(profileScreenRect.minX - fullScreen.minX) <= 0.5,
              abs(profileScreenRect.minY - fullScreen.minY) <= 0.5,
              abs(profileScreenRect.maxX - fullScreen.maxX) <= 0.5,
              abs(profileScreenRect.maxY - fullScreen.maxY) <= 0.5 else {
            return 0
        }
        return min(
            profileScreenRect.width * scale / 2,
            profileScreenRect.height * scale / 2,
            CGFloat(profile.cornerRadius) * scale
        )
    }
}

private extension ChromeButtonProfile {
    var hardwareWireName: String? {
        switch name.lowercased() {
        case "action":
            "action"
        case "digital-crown", "crown":
            "digital-crown"
        case "home":
            "home"
        case "left-side-button":
            "left-side-button"
        case "lock", "power":
            "power"
        case "mute":
            "mute"
        case "side-button":
            "side-button"
        case "volume-down":
            "volume-down"
        case "volume-up":
            "volume-up"
        default:
            nil
        }
    }
}

private extension CGSize {
    func aspectFit(in rect: CGRect) -> CGRect {
        guard width > 0, height > 0, rect.width > 0, rect.height > 0 else {
            return .zero
        }
        let scale = min(rect.width / width, rect.height / height)
        let fittedSize = CGSize(width: width * scale, height: height * scale)
        return CGRect(
            x: rect.midX - fittedSize.width / 2,
            y: rect.midY - fittedSize.height / 2,
            width: fittedSize.width,
            height: fittedSize.height
        )
    }
}

private extension View {
    @ViewBuilder
    func clippedToSimulatorScreen(cornerRadius: CGFloat, maskImage: UIImage?) -> some View {
        if let maskImage {
            self.mask(
                Image(uiImage: maskImage)
                    .resizable()
                    .scaledToFill()
            )
        } else {
            self.clipShape(RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
        }
    }

    @ViewBuilder
    func streamTouchGesture<G: Gesture>(_ enabled: Bool, gesture: G) -> some View {
        if enabled {
            self.gesture(gesture)
        } else {
            self
        }
    }
}
