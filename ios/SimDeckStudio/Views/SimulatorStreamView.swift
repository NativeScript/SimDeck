import SwiftUI

struct SimulatorStreamView: View {
    @Bindable var model: AppModel
    @Environment(\.colorScheme) private var colorScheme
    @State private var activeTouchKind: StreamTouchKind?
    @State private var presentedSheet: StreamSheet?

    var body: some View {
        ZStack {
            if model.selectedSimulator == nil {
                ContentUnavailableView("No Simulator", systemImage: "iphone.slash")
            } else {
                streamViewport
            }
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
                    .disabled(model.currentStreamClient == nil)
                } label: {
                    Label("Stream Settings", systemImage: "gearshape")
                }
            }
        }
        .sheet(item: $presentedSheet) { sheet in
            switch sheet {
            case .simulators:
                StreamSimulatorSelectionSheet(model: model)
            }
        }
        .safeAreaInset(edge: .bottom) {
            if model.selectedSimulator?.isBooted == true {
                StreamControlBar(model: model)
            }
        }
    }

    private var streamViewport: some View {
        GeometryReader { proxy in
            let layout = DeviceViewportLayout(
                chromeProfile: model.chromeProfile,
                chromeImageSize: model.chromeImage?.size,
                videoSize: model.videoSize,
                availableSize: proxy.size
            )
            let displayToken = model.streamDisplayToken

            ZStack(alignment: .topLeading) {
                streamBackground

                RoundedRectangle(cornerRadius: layout.screenCornerRadius + 2, style: .continuous)
                    .fill(.black)
                    .frame(width: layout.screenBackingFrame.width, height: layout.screenBackingFrame.height)
                    .position(x: layout.screenBackingFrame.midX, y: layout.screenBackingFrame.midY)

                if model.selectedSimulator?.isBooted == true, model.currentStreamClient != nil {
                    WebRTCVideoView(client: model.currentStreamClient) { size in
                        model.videoSize = size
                        model.markStreamFrameRendered(displayToken: displayToken)
                    }
                    .id(displayToken)
                    .frame(width: layout.videoFrame.width, height: layout.videoFrame.height)
                    .clipShape(RoundedRectangle(cornerRadius: layout.screenCornerRadius + 1, style: .continuous))
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

                if let simulator = model.selectedSimulator, !simulator.isBooted {
                    BootSimulatorOverlay(model: model, simulator: simulator)
                        .frame(width: layout.screenFrame.width, height: layout.screenFrame.height)
                        .position(x: layout.screenFrame.midX, y: layout.screenFrame.midY)
                }
            }
            .contentShape(Rectangle())
            .streamTouchGesture(model.selectedSimulator?.isBooted == true, gesture: touchGesture(in: layout.screenFrame))
        }
        .background(streamBackground)
    }

    private var streamBackground: Color {
        colorScheme == .dark ? Color(.systemBackground) : Color(.secondarySystemGroupedBackground)
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
        switch activeTouchKind {
        case .bottomEdge:
            model.sendEdgeTouch(location: location, in: screenFrame, phase: phase, edge: "bottom")
        case .single:
            model.sendTouch(location: location, in: screenFrame, phase: phase)
        case nil:
            break
        }
    }
}

private enum StreamTouchKind {
    case single
    case bottomEdge
}

private enum StreamSheet: Identifiable {
    case simulators

    var id: Self { self }
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
            .padding(.vertical, 8)
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .frame(minWidth: 190, maxWidth: 260)
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

    var body: some View {
        if #available(iOS 26.0, *) {
            LiquidGlassStreamControlBar(model: model)
        } else {
            LegacyStreamControlBar(model: model)
        }
    }
}

@available(iOS 26.0, *)
private struct LiquidGlassStreamControlBar: View {
    @Bindable var model: AppModel

    var body: some View {
        GlassEffectContainer(spacing: 14) {
            StreamControlButtons(model: model)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }
}

private struct LegacyStreamControlBar: View {
    @Bindable var model: AppModel

    var body: some View {
        StreamControlButtons(model: model)
            .buttonStyle(StreamToolbarButtonStyle())
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
    }
}

private struct StreamControlButtons: View {
    @Bindable var model: AppModel

    var body: some View {
        HStack(spacing: 8) {
            StreamControlButton("Home", systemImage: "house") { model.sendHome() }

            StreamControlButton("Switcher", systemImage: "square.on.square") { model.sendAppSwitcher() }

            Spacer(minLength: 4)

            StreamControlButton("Rotate Left", systemImage: "rotate.left") { model.rotateLeft() }

            StreamControlButton("Rotate Right", systemImage: "rotate.right") { model.rotateRight() }

            Spacer(minLength: 4)

            StreamControlButton("Keyframe", systemImage: "bolt") { model.requestKeyframe() }

            StreamControlButton("Lock", systemImage: "lock") { model.sendLock() }
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
            label
        }
        .buttonStyle(.plain)
        .buttonBorderShape(.circle)
    }

    @ViewBuilder
    private var label: some View {
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

    init(chromeProfile: ChromeProfile?, chromeImageSize: CGSize?, videoSize: CGSize, availableSize: CGSize) {
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
            let metricScale = Self.chromeMetricScale(profile: chromeProfile, imageSize: chromeImageSize)
            let shell = profileSize.aspectFit(in: viewport)
            let scale = shell.width / profileSize.width
            let screenRect = Self.chromeScreenRect(profile: chromeProfile, videoSize: videoSize, metricScale: metricScale)
            shellFrame = shell
            screenFrame = CGRect(
                x: shell.minX + screenRect.minX * scale,
                y: shell.minY + screenRect.minY * scale,
                width: screenRect.width * scale,
                height: screenRect.height * scale
            )
            screenBackingFrame = screenFrame.insetBy(dx: -2, dy: -2)
            videoFrame = screenFrame.insetBy(dx: -1, dy: -1)
            screenCornerRadius = Self.screenCornerRadius(
                profile: chromeProfile,
                profileScreenRect: screenRect,
                scale: scale,
                metricScale: metricScale
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
    }

    private static func chromeMetricScale(profile: ChromeProfile, imageSize: CGSize?) -> Double {
        guard profile.totalWidth > 0,
              profile.totalHeight > 0,
              let imageSize,
              imageSize.width > 0,
              imageSize.height > 0,
              profile.screenWidth > profile.totalWidth || profile.screenHeight > profile.totalHeight else {
            return 1
        }
        let widthScale = Double(imageSize.width) / profile.totalWidth
        let heightScale = Double(imageSize.height) / profile.totalHeight
        let scale = min(widthScale, heightScale)
        return scale.isFinite && scale > 1 ? scale : 1
    }

    private static func chromeScreenRect(profile: ChromeProfile, videoSize: CGSize, metricScale: Double) -> CGRect {
        let profileScreenWidth = profile.screenWidth / metricScale
        let profileScreenHeight = profile.screenHeight / metricScale
        let profileScreenX = profile.screenX / metricScale
        let profileScreenY = profile.screenY / metricScale
        let profileAspect = profileScreenWidth / profileScreenHeight
        let videoAspect = videoSize.width > 0 && videoSize.height > 0
            ? Double(videoSize.width / videoSize.height)
            : profileAspect
        guard profileAspect.isFinite, profileAspect > 0, videoAspect.isFinite, videoAspect > 0 else {
            return CGRect(
                x: CGFloat(profileScreenX),
                y: CGFloat(profileScreenY),
                width: CGFloat(profileScreenWidth),
                height: CGFloat(profileScreenHeight)
            )
        }

        let aspectDelta = abs(videoAspect - profileAspect) / profileAspect
        if aspectDelta <= 0.01 {
            return CGRect(
                x: CGFloat(profileScreenX),
                y: CGFloat(profileScreenY),
                width: CGFloat(profileScreenWidth),
                height: CGFloat(profileScreenHeight)
            )
        }

        var width = profileScreenWidth
        var height = width / videoAspect
        var x = profileScreenX
        var y = profileScreenY
        if height > profileScreenHeight {
            height = profileScreenHeight
            width = height * videoAspect
            x += (profileScreenWidth - width) / 2
        } else {
            y += (profileScreenHeight - height) / 2
        }
        return CGRect(x: CGFloat(x), y: CGFloat(y), width: CGFloat(width), height: CGFloat(height))
    }

    private static func screenCornerRadius(profile: ChromeProfile, profileScreenRect: CGRect, scale: CGFloat, metricScale: Double) -> CGFloat {
        let fullScreen = CGRect(
            x: CGFloat(profile.screenX / metricScale),
            y: CGFloat(profile.screenY / metricScale),
            width: CGFloat(profile.screenWidth / metricScale),
            height: CGFloat(profile.screenHeight / metricScale)
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
            CGFloat(profile.cornerRadius / metricScale) * scale
        )
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
    func streamTouchGesture<G: Gesture>(_ enabled: Bool, gesture: G) -> some View {
        if enabled {
            self.gesture(gesture)
        } else {
            self
        }
    }
}
