import SwiftUI
import UIKit

struct SimulatorStreamView: View {
    @Bindable var model: AppModel
    @Environment(\.colorScheme) private var colorScheme
    @State private var touchIndicators: [StreamTouchIndicator] = []
    @State private var touchOverlayRemovalTask: Task<Void, Never>?
    @State private var presentedSheet: StreamSheet?
    @State private var keyboardCaptureActive = false
    @State private var keyboardHeight: CGFloat = 0

    var body: some View {
        GeometryReader { proxy in
            streamContent(usesSideControls: usesSideControls(in: proxy.size))
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
                streamSettingsMenu
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
        .onAppear {
            applyOrientationPolicy()
        }
        .onChange(of: model.selectedSimulatorID) { _, _ in
            keyboardCaptureActive = false
            clearTouchInteractionState()
            applyOrientationPolicy()
        }
        .onChange(of: model.selectedSimulator?.isBooted == true) { _, isBooted in
            if !isBooted {
                keyboardCaptureActive = false
                clearTouchInteractionState()
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

    private func streamContent(usesSideControls: Bool) -> some View {
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
        .safeAreaInset(edge: .bottom) {
            if model.selectedSimulator != nil, !usesSideControls {
                StreamControlBar(
                    model: model,
                    keyboardCaptureActive: $keyboardCaptureActive,
                    placement: .bottom
                )
            }
        }
        .safeAreaInset(edge: .leading, spacing: 0) {
            if model.selectedSimulator != nil, usesSideControls {
                StreamControlBar(
                    model: model,
                    keyboardCaptureActive: $keyboardCaptureActive,
                    placement: .leadingSide
                )
            }
        }
        .safeAreaInset(edge: .trailing, spacing: 0) {
            if model.selectedSimulator != nil, usesSideControls {
                StreamControlBar(
                    model: model,
                    keyboardCaptureActive: $keyboardCaptureActive,
                    placement: .trailingSide
                )
            }
        }
    }

    private var streamSettingsMenu: some View {
        Menu {
            Section {
                Button(role: model.selectedSimulator?.isBooted == true ? .destructive : nil) {
                    if let simulator = model.selectedSimulator {
                        Task { await model.toggleSimulatorLifecycle(simulator) }
                    }
                } label: {
                    if model.isSelectedSimulatorLifecycleBusy {
                        HStack {
                            ProgressView()
                                .controlSize(.small)
                            Text(model.selectedSimulator?.isBooted == true ? "Stopping Simulator" : "Starting Simulator")
                        }
                    } else {
                        Label(
                            model.selectedSimulator?.isBooted == true ? "Stop Simulator" : "Start Simulator",
                            systemImage: model.selectedSimulator?.isBooted == true ? "stop.circle" : "play.circle"
                        )
                    }
                }
                .disabled(model.selectedSimulator == nil || model.endpoint == nil || model.isSelectedSimulatorLifecycleBusy)

                Button {
                    model.hapticSelection()
                    Task { await model.refreshSimulators() }
                } label: {
                    Label("Refresh", systemImage: "arrow.clockwise")
                }
            }
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
        } label: {
            Label("Stream Settings", systemImage: "gearshape")
        }
    }

    private func usesSideControls(in size: CGSize) -> Bool {
        selectedControlFamily.usesLandscapeSideControls && size.width > size.height
    }

    private var selectedControlFamily: StreamControlFamily {
        model.selectedSimulator?.streamControlFamily ?? .iOS
    }

    private func applyOrientationPolicy() {
        AppOrientationPolicy.apply(selectedControlFamily.allowedHostOrientations)
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
                    .clippedToSimulatorScreen(cornerRadius: layout.screenBackingCornerRadius, maskImage: nil)
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

                if model.selectedSimulator?.isBooted == true {
                    StreamTouchInputLayer { event in
                        handleTouchEvent(event)
                    }
                    .frame(width: layout.screenFrame.width, height: layout.screenFrame.height)
                    .position(x: layout.screenFrame.midX, y: layout.screenFrame.midY)

                    NavigationPopGestureBlocker()
                        .frame(width: layout.screenFrame.width, height: layout.screenFrame.height)
                        .position(x: layout.screenFrame.midX, y: layout.screenFrame.midY)
                        .allowsHitTesting(false)
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

    private func handleTouchEvent(_ event: StreamTouchEvent) {
        if event.updatesOverlay {
            updateTouchOverlay(event)
        }
        guard event.dispatchesInput else { return }
        switch event.kind {
        case .single:
            guard let point = event.points.first else { return }
            model.sendTouch(x: Double(point.normalized.x), y: Double(point.normalized.y), phase: event.phase)
        case .bottomEdge:
            guard let point = event.points.first else { return }
            model.sendEdgeTouch(
                x: Double(point.normalized.x),
                y: Double(point.normalized.y),
                phase: event.phase,
                edge: "bottom"
            )
        case .multi:
            guard event.points.count >= 2 else { return }
            model.sendMultiTouch(
                x1: Double(event.points[0].normalized.x),
                y1: Double(event.points[0].normalized.y),
                x2: Double(event.points[1].normalized.x),
                y2: Double(event.points[1].normalized.y),
                phase: event.phase
            )
        }
    }

    private func updateTouchOverlay(_ event: StreamTouchEvent) {
        guard model.touchOverlayVisible else {
            clearTouchOverlay()
            return
        }

        switch event.phase {
        case "began":
            touchOverlayRemovalTask?.cancel()
            withAnimation(.snappy(duration: 0.12)) {
                updateActiveTouchIndicators(with: event.points)
            }
        case "moved":
            guard !event.points.isEmpty else { return }
            withAnimation(.linear(duration: 0.035)) {
                updateActiveTouchIndicators(with: event.points)
            }
        case "ended", "cancelled":
            guard !event.points.isEmpty else {
                clearTouchOverlay()
                return
            }
            for point in event.points {
                if let index = touchIndicators.firstIndex(where: { $0.id == point.id }) {
                    touchIndicators[index].current = point.local
                    touchIndicators[index].isEnding = true
                } else {
                    touchIndicators.append(
                        StreamTouchIndicator(
                            id: point.id,
                            start: point.local,
                            current: point.local,
                            isEnding: true
                        )
                    )
                }
            }
            let endingIDs = Set(event.points.map(\.id))
            touchOverlayRemovalTask?.cancel()
            touchOverlayRemovalTask = Task { @MainActor in
                try? await Task.sleep(for: .milliseconds(240))
                withAnimation(.easeOut(duration: 0.16)) {
                    touchIndicators.removeAll { endingIDs.contains($0.id) }
                }
            }
        default:
            break
        }
    }

    private func updateActiveTouchIndicators(with points: [StreamTouchPoint]) {
        let activeIDs = Set(points.map(\.id))
        touchIndicators.removeAll { !activeIDs.contains($0.id) }

        for point in points {
            if let index = touchIndicators.firstIndex(where: { $0.id == point.id }) {
                touchIndicators[index].current = point.local
                touchIndicators[index].isEnding = false
            } else {
                touchIndicators.append(
                    StreamTouchIndicator(
                        id: point.id,
                        start: point.local,
                        current: point.local,
                        isEnding: false
                    )
                )
            }
        }
        touchIndicators.sort { $0.id < $1.id }
    }

    private func clearTouchOverlay() {
        touchOverlayRemovalTask?.cancel()
        touchOverlayRemovalTask = nil
        touchIndicators = []
    }

    private func clearTouchInteractionState() {
        clearTouchOverlay()
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

private enum StreamTouchEventKind {
    case single
    case bottomEdge
    case multi
}

private enum StreamSheet: Identifiable {
    case simulators
    case debugInfo

    var id: Self { self }
}

private struct StreamTouchPoint {
    let id: Int
    let local: CGPoint
    let normalized: CGPoint
}

private struct StreamTouchEvent {
    let kind: StreamTouchEventKind
    let phase: String
    let points: [StreamTouchPoint]
    let dispatchesInput: Bool
    let updatesOverlay: Bool

    init(
        kind: StreamTouchEventKind,
        phase: String,
        points: [StreamTouchPoint],
        dispatchesInput: Bool = true,
        updatesOverlay: Bool = true
    ) {
        self.kind = kind
        self.phase = phase
        self.points = points
        self.dispatchesInput = dispatchesInput
        self.updatesOverlay = updatesOverlay
    }
}

#if DEBUG
struct StreamTouchInputDebugHarness: View {
    @State private var indicators: [StreamTouchIndicator] = []
    @State private var eventLog: [String] = []

    var body: some View {
        ZStack(alignment: .topLeading) {
            Color.black.ignoresSafeArea()

            StreamTouchInputLayer { event in
                handle(event)
            }
            .ignoresSafeArea()

            TouchInteractionOverlay(indicators: indicators)
                .allowsHitTesting(false)
                .ignoresSafeArea()

            VStack(alignment: .leading, spacing: 6) {
                Text("Touch Input Test")
                    .font(.headline)
                ForEach(eventLog, id: \.self) { line in
                    Text(line)
                        .font(.caption.monospaced())
                }
            }
            .foregroundStyle(.white)
            .padding(14)
        }
        .accessibilityIdentifier("touch-input-test-harness")
    }

    private func handle(_ event: StreamTouchEvent) {
        if event.updatesOverlay {
            switch event.phase {
            case "began", "moved":
                indicators = event.points.map {
                    StreamTouchIndicator(
                        id: $0.id,
                        start: $0.local,
                        current: $0.local,
                        isEnding: false
                    )
                }
            case "ended", "cancelled":
                indicators = event.points.map {
                    StreamTouchIndicator(
                        id: $0.id,
                        start: $0.local,
                        current: $0.local,
                        isEnding: true
                    )
                }
            default:
                break
            }
        }

        guard event.dispatchesInput else { return }
        let line = event.debugLine
        print("SIMDECK_TOUCH_INPUT_TEST \(line)")
        eventLog.append(line)
        eventLog = Array(eventLog.suffix(8))
    }
}

private extension StreamTouchEvent {
    var debugLine: String {
        let pointsDescription = points
            .map { point in
                "\(point.id):\(String(format: "%.3f", point.normalized.x)),\(String(format: "%.3f", point.normalized.y))"
            }
            .joined(separator: " ")
        return "\(kind.debugName) \(phase) \(points.count) \(pointsDescription)"
    }
}

private extension StreamTouchEventKind {
    var debugName: String {
        switch self {
        case .single:
            return "single"
        case .bottomEdge:
            return "bottomEdge"
        case .multi:
            return "multi"
        }
    }
}
#endif

private struct StreamTouchInputLayer: UIViewRepresentable {
    let onEvent: (StreamTouchEvent) -> Void

    func makeUIView(context: Context) -> StreamTouchInputView {
        let view = StreamTouchInputView()
        view.onEvent = onEvent
        return view
    }

    func updateUIView(_ view: StreamTouchInputView, context: Context) {
        view.onEvent = onEvent
    }
}

private final class StreamTouchInputView: UIView {
    var onEvent: ((StreamTouchEvent) -> Void)?

    private enum ActiveGesture {
        case single(kind: StreamTouchEventKind, touch: UITouch)
        case multi(touches: [UITouch])
    }

    private var activeTouches: [UITouch] = []
    private var touchIDs: [ObjectIdentifier: Int] = [:]
    private var nextTouchID = 1
    private var activeGesture: ActiveGesture?
    private var lastMultiPoints: [StreamTouchPoint] = []
    private var pendingSingleTouch: UITouch?
    private var pendingSingleWorkItem: DispatchWorkItem?
    private var pendingSingleKind: StreamTouchEventKind?
    private var pendingSingleSamples: [(phase: String, point: StreamTouchPoint)] = []
    private var suppressSingleUntilAllTouchesEnd = false
    private let singleTouchDeferral: TimeInterval = 0.16

    override init(frame: CGRect) {
        super.init(frame: frame)
        isOpaque = false
        backgroundColor = .clear
        isUserInteractionEnabled = true
        isMultipleTouchEnabled = true
        isExclusiveTouch = true

        #if DEBUG
        if CommandLine.arguments.contains("--simdeck-touch-input-test")
            || CommandLine.arguments.contains("--simdeck-e2e-controller") {
            accessibilityIdentifier = "touch-input-surface"
            isAccessibilityElement = true
        }
        #endif
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        nil
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        addTouches(touches)
        reconcileAfterTouchStart()
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        if appendPendingSingleMoveIfNeeded(for: touches) {
            return
        }
        sendCurrentGesture(phase: "moved")
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        finishTouches(touches, phase: "ended")
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        finishTouches(touches, phase: "cancelled")
    }

    private func addTouches(_ touches: Set<UITouch>) {
        for touch in touches where containsStartingPoint(touch) && !activeTouches.contains(where: { $0 === touch }) {
            activeTouches.append(touch)
            let objectID = ObjectIdentifier(touch)
            if touchIDs[objectID] == nil {
                touchIDs[objectID] = nextTouchID
                nextTouchID += 1
            }
        }
    }

    private func reconcileAfterTouchStart() {
        guard !suppressSingleUntilAllTouchesEnd else { return }

        switch activeGesture {
        case nil:
            if activeTouches.count >= 2 {
                cancelPendingSingleTouch()
                beginMultiTouch()
            } else if let touch = activeTouches.first {
                startPendingSingleTouch(touch)
            }
        case .single:
            if activeTouches.count >= 2 {
                promoteSingleTouchToMultiTouch()
            }
        case .multi:
            return
        }
    }

    private func finishTouches(_ touches: Set<UITouch>, phase: String) {
        if pendingSingleTouch != nil, touches.contains(where: isPendingSingleTouch) {
            activatePendingSingleTouch()
        } else if pendingSingleTouch != nil, activeTouches.count >= 2 {
            cancelPendingSingleTouch()
            beginMultiTouch()
        }

        switch activeGesture {
        case .single(let kind, let touch):
            if touches.contains(where: { $0 === touch }) {
                sendSingleTouch(kind: kind, touch: touch, phase: phase)
                activeGesture = nil
            }
        case .multi(let gestureTouches):
            if touches.contains(where: { endingTouch in
                gestureTouches.contains { $0 === endingTouch }
            }) {
                sendMultiTouch(touches: gestureTouches, phase: phase)
                activeGesture = nil
                lastMultiPoints = []
                suppressSingleUntilAllTouchesEnd = true
            }
        case nil:
            break
        }

        removeTouches(touches)

        if activeTouches.isEmpty {
            cancelPendingSingleTouch()
            suppressSingleUntilAllTouchesEnd = false
            activeGesture = nil
            lastMultiPoints = []
            touchIDs = [:]
        } else if activeGesture == nil, !suppressSingleUntilAllTouchesEnd {
            reconcileAfterTouchStart()
        }
    }

    private func startPendingSingleTouch(_ touch: UITouch) {
        if let pendingSingleTouch, pendingSingleTouch === touch {
            return
        }
        cancelPendingSingleTouch()
        guard let point = touchPoint(for: touch) else { return }
        pendingSingleTouch = touch
        pendingSingleKind = point.normalized.y >= 0.93 ? .bottomEdge : .single
        pendingSingleSamples = [(phase: "began", point: point)]
        onEvent?(
            StreamTouchEvent(
                kind: pendingSingleKind ?? .single,
                phase: "began",
                points: [point],
                dispatchesInput: false
            )
        )

        let workItem = DispatchWorkItem { [weak self] in
            guard let self,
                  let pendingSingleTouch = self.pendingSingleTouch,
                  pendingSingleTouch === touch,
                  self.activeGesture == nil,
                  !self.suppressSingleUntilAllTouchesEnd,
                  self.activeTouches.contains(where: { $0 === touch }) else {
                return
            }

            self.activatePendingSingleTouch()
        }
        pendingSingleWorkItem = workItem
        DispatchQueue.main.asyncAfter(deadline: .now() + singleTouchDeferral, execute: workItem)
    }

    private func appendPendingSingleMoveIfNeeded(for touches: Set<UITouch>) -> Bool {
        guard let pendingSingleTouch,
              activeGesture == nil else { return false }

        if activeTouches.count >= 2 {
            cancelPendingSingleTouch()
            beginMultiTouch()
            return true
        }

        guard touches.contains(where: { $0 === pendingSingleTouch }) else {
            return false
        }
        if let point = touchPoint(for: pendingSingleTouch) {
            pendingSingleSamples.append((phase: "moved", point: point))
            onEvent?(
                StreamTouchEvent(
                    kind: pendingSingleKind ?? .single,
                    phase: "moved",
                    points: [point],
                    dispatchesInput: false
                )
            )
        }
        return true
    }

    @discardableResult
    private func activatePendingSingleTouch() -> Bool {
        guard let pendingSingleTouch,
              let pendingSingleKind,
              activeGesture == nil,
              !suppressSingleUntilAllTouchesEnd,
              activeTouches.count == 1,
              activeTouches.contains(where: { $0 === pendingSingleTouch }) else {
            return false
        }

        let samples = pendingSingleSamples
        pendingSingleWorkItem?.cancel()
        pendingSingleWorkItem = nil
        self.pendingSingleTouch = nil
        self.pendingSingleKind = nil
        pendingSingleSamples = []
        activeGesture = .single(kind: pendingSingleKind, touch: pendingSingleTouch)

        if samples.isEmpty {
            sendSingleTouch(kind: pendingSingleKind, touch: pendingSingleTouch, phase: "began")
        } else {
            for sample in samples {
                onEvent?(
                    StreamTouchEvent(
                        kind: pendingSingleKind,
                        phase: sample.phase,
                        points: [sample.point],
                        updatesOverlay: false
                    )
                )
            }
        }
        return true
    }

    private func cancelPendingSingleTouch() {
        pendingSingleWorkItem?.cancel()
        pendingSingleWorkItem = nil
        pendingSingleTouch = nil
        pendingSingleKind = nil
        pendingSingleSamples = []
    }

    private func isPendingSingleTouch(_ touch: UITouch) -> Bool {
        pendingSingleTouch === touch
    }

    private func promoteSingleTouchToMultiTouch() {
        if case .single(let kind, let touch) = activeGesture {
            sendSingleTouch(kind: kind, touch: touch, phase: "cancelled")
        }
        beginMultiTouch()
    }

    private func beginMultiTouch() {
        let touches = Array(activeTouches.prefix(2))
        guard touches.count == 2 else { return }
        activeGesture = .multi(touches: touches)
        sendMultiTouch(touches: touches, phase: "began")
    }

    private func sendCurrentGesture(phase: String) {
        switch activeGesture {
        case .single(let kind, let touch):
            sendSingleTouch(kind: kind, touch: touch, phase: phase)
        case .multi(let touches):
            sendMultiTouch(touches: touches, phase: phase)
        case nil:
            if !suppressSingleUntilAllTouchesEnd {
                reconcileAfterTouchStart()
            }
        }
    }

    private func sendSingleTouch(kind: StreamTouchEventKind, touch: UITouch, phase: String) {
        guard let point = touchPoint(for: touch) else { return }
        onEvent?(StreamTouchEvent(kind: kind, phase: phase, points: [point]))
    }

    private func sendMultiTouch(touches: [UITouch], phase: String) {
        let points = touches.prefix(2).compactMap { touchPoint(for: $0) }
        guard points.count == 2 else {
            if (phase == "ended" || phase == "cancelled"), lastMultiPoints.count == 2 {
                onEvent?(StreamTouchEvent(kind: .multi, phase: phase, points: lastMultiPoints))
            }
            return
        }
        lastMultiPoints = points
        onEvent?(StreamTouchEvent(kind: .multi, phase: phase, points: points))
    }

    private func containsStartingPoint(_ touch: UITouch) -> Bool {
        guard bounds.width > 0, bounds.height > 0 else {
            return false
        }
        let location = touch.location(in: self)
        return location.x >= 0
            && location.x <= bounds.width
            && location.y >= 0
            && location.y <= bounds.height
    }

    private func removeTouches(_ touches: Set<UITouch>) {
        activeTouches.removeAll { activeTouch in
            touches.contains { $0 === activeTouch }
        }
        for touch in touches {
            touchIDs.removeValue(forKey: ObjectIdentifier(touch))
        }
    }

    private func touchPoint(for touch: UITouch) -> StreamTouchPoint? {
        guard bounds.width > 0,
              bounds.height > 0,
              let id = touchIDs[ObjectIdentifier(touch)] else {
            return nil
        }
        let location = touch.location(in: self)
        let local = CGPoint(
            x: min(max(location.x, 0), bounds.width),
            y: min(max(location.y, 0), bounds.height)
        )
        return StreamTouchPoint(
            id: id,
            local: local,
            normalized: CGPoint(
                x: min(max(local.x / bounds.width, 0), 1),
                y: min(max(local.y / bounds.height, 0), 1)
            )
        )
    }
}

private struct StreamTouchIndicator: Identifiable, Equatable {
    let id: Int
    var start: CGPoint
    var current: CGPoint
    var isEnding: Bool
}

private struct TouchInteractionOverlay: View {
    let indicators: [StreamTouchIndicator]

    var body: some View {
        ZStack(alignment: .topLeading) {
            ForEach(indicators) { indicator in
                Circle()
                    .fill(.white.opacity(indicator.isEnding ? 0.18 : 0.36))
                    .stroke(.white.opacity(indicator.isEnding ? 0.36 : 0.86), lineWidth: 2)
                    .frame(width: indicator.isEnding ? 34 : 42, height: indicator.isEnding ? 34 : 42)
                    .position(x: indicator.current.x, y: indicator.current.y)
                    .shadow(color: .black.opacity(0.3), radius: 7)
                    .scaleEffect(indicator.isEnding ? 0.82 : 1)
                    .animation(.interactiveSpring(response: 0.09, dampingFraction: 0.86), value: indicator.current)
                    .animation(.easeOut(duration: 0.14), value: indicator.isEnding)
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
            .accessibilityLabel(model.isSelectedSimulatorBooting ? "Starting \(simulator.name)" : "Start \(simulator.name)")
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
                if model.isSelectedSimulatorLifecycleBusy {
                    ProgressView()
                        .controlSize(.small)
                        .frame(width: 12, height: 12)
                } else {
                    Circle()
                        .fill(statusColor)
                        .frame(width: 7, height: 7)
                }
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
                            SimulatorRow(
                                simulator: simulator,
                                isBusy: model.isSimulatorLifecycleBusy(simulator)
                            )
                            Spacer()
                            if model.selectedSimulatorID == simulator.udid {
                                Image(systemName: "checkmark")
                                    .font(.headline)
                                    .foregroundStyle(.tint)
                            }
                        }
                    }
                    .buttonStyle(.plain)
                    .contextMenu {
                        SimulatorLifecycleMenuItems(model: model, simulator: simulator)
                    }
                }
            }
            .refreshable {
                await model.refreshSimulators()
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
    let placement: StreamControlPlacement

    var body: some View {
        Group {
            if #available(iOS 26.0, *) {
                LiquidGlassStreamControlBar(
                    model: model,
                    keyboardCaptureActive: $keyboardCaptureActive,
                    placement: placement
                )
            } else {
                LegacyStreamControlBar(
                    model: model,
                    keyboardCaptureActive: $keyboardCaptureActive,
                    placement: placement
                )
            }
        }
        .disabled(!isEnabled)
        .opacity(isEnabled ? 1 : 0.38)
        .animation(.snappy(duration: 0.18), value: isEnabled)
    }

    private var isEnabled: Bool {
        model.endpoint != nil && model.selectedSimulator?.isBooted == true
    }
}

private enum StreamControlPlacement {
    case bottom
    case leadingSide
    case trailingSide

    var isSide: Bool {
        self != .bottom
    }
}

@available(iOS 26.0, *)
private struct LiquidGlassStreamControlBar: View {
    @Bindable var model: AppModel
    @Binding var keyboardCaptureActive: Bool
    let placement: StreamControlPlacement

    var body: some View {
        GlassEffectContainer(spacing: 14) {
            StreamControlButtons(
                model: model,
                keyboardCaptureActive: $keyboardCaptureActive,
                placement: placement
            )
        }
        .padding(.horizontal, placement.isSide ? 10 : 16)
        .padding(.vertical, placement.isSide ? 16 : 10)
    }
}

private struct LegacyStreamControlBar: View {
    @Bindable var model: AppModel
    @Binding var keyboardCaptureActive: Bool
    let placement: StreamControlPlacement

    var body: some View {
        StreamControlButtons(
            model: model,
            keyboardCaptureActive: $keyboardCaptureActive,
            placement: placement
        )
            .buttonStyle(StreamToolbarButtonStyle())
            .padding(.horizontal, placement.isSide ? 10 : 16)
            .padding(.vertical, placement.isSide ? 16 : 10)
    }
}

private struct StreamControlButtons: View {
    @Bindable var model: AppModel
    @Binding var keyboardCaptureActive: Bool
    let placement: StreamControlPlacement

    var body: some View {
        if placement.isSide {
            VStack(spacing: 8) {
                controls
            }
        } else {
            HStack(spacing: 8) {
                controls
            }
        }
    }

    @ViewBuilder
    private var controls: some View {
        switch model.selectedSimulator?.streamControlFamily ?? .iOS {
        case .iOS:
            iosControls
        case .appleWatch:
            watchControls
        case .appleTV:
            tvControls
        case .appleVision:
            visionControls
        case .android:
            androidControls
        }
    }

    @ViewBuilder
    private var iosControls: some View {
        StreamHardwareControlButton("Home", systemImage: "house", buttonName: "home", model: model)

        StreamControlButton("Switcher", systemImage: "square.on.square") { model.sendAppSwitcher() }

        Spacer(minLength: 4)

        StreamControlButton("Appearance", systemImage: "circle.lefthalf.filled") { model.toggleAppearance() }

        StreamControlButton("Rotate Right", systemImage: "rotate.right") { model.rotateRight() }

        Spacer(minLength: 4)

        StreamHardwareControlButton("Lock", systemImage: "lock", buttonName: "power", model: model)

        StreamKeyboardControlButton(model: model, isActive: $keyboardCaptureActive)
    }

    @ViewBuilder
    private var watchControls: some View {
        switch placement {
        case .bottom:
            StreamCrownControlButton("Crown Up", systemImage: "arrow.up.circle", delta: -48, model: model)

            StreamCrownControlButton("Crown Down", systemImage: "arrow.down.circle", delta: 48, model: model)

            StreamHardwareControlButton("Crown Press", systemImage: "circle.circle", buttonName: "digital-crown", model: model)

            Spacer(minLength: 4)

            StreamHardwareControlButton("Side Button", systemImage: "button.programmable", buttonName: "side-button", model: model)
        case .leadingSide:
            StreamCrownControlButton("Crown Up", systemImage: "arrow.up.circle", delta: -48, model: model)

            StreamHardwareControlButton("Crown Press", systemImage: "circle.circle", buttonName: "digital-crown", model: model)

            StreamCrownControlButton("Crown Down", systemImage: "arrow.down.circle", delta: 48, model: model)
        case .trailingSide:
            StreamHardwareControlButton("Side Button", systemImage: "button.programmable", buttonName: "side-button", model: model)
        }
    }

    @ViewBuilder
    private var tvControls: some View {
        switch placement {
        case .bottom:
            StreamKeyControlButton("Back", systemImage: "chevron.backward", keyCode: 41, model: model)

            StreamKeyControlButton("Up", systemImage: "chevron.up", keyCode: 82, model: model)

            StreamKeyControlButton("Left", systemImage: "chevron.left", keyCode: 80, model: model)

            StreamKeyControlButton("Select", systemImage: "smallcircle.filled.circle", keyCode: 40, model: model)

            StreamKeyControlButton("Right", systemImage: "chevron.right", keyCode: 79, model: model)

            StreamKeyControlButton("Down", systemImage: "chevron.down", keyCode: 81, model: model)
        case .leadingSide:
            StreamKeyControlButton("Back", systemImage: "chevron.backward", keyCode: 41, model: model)
        case .trailingSide:
            StreamTVRemotePad(model: model)
        }
    }

    @ViewBuilder
    private var visionControls: some View {
        StreamHardwareControlButton("Home", systemImage: "house", buttonName: "home", model: model)

        StreamControlButton("Switcher", systemImage: "square.on.square") { model.sendAppSwitcher() }

        Spacer(minLength: 4)

        StreamControlButton("Appearance", systemImage: "circle.lefthalf.filled") { model.toggleAppearance() }

        StreamKeyboardControlButton(model: model, isActive: $keyboardCaptureActive)
    }

    @ViewBuilder
    private var androidControls: some View {
        StreamHardwareControlButton("Back", systemImage: "chevron.backward", buttonName: "back", model: model)

        StreamHardwareControlButton("Home", systemImage: "house", buttonName: "home", model: model)

        StreamControlButton("Switcher", systemImage: "square.on.square") { model.sendAppSwitcher() }

        Spacer(minLength: 4)

        StreamControlButton("Rotate Right", systemImage: "rotate.right") { model.rotateRight() }

        StreamKeyboardControlButton(model: model, isActive: $keyboardCaptureActive)

        StreamHardwareControlButton("Power", systemImage: "lock", buttonName: "lock", model: model)
    }
}

private enum StreamControlFamily {
    case iOS
    case appleWatch
    case appleTV
    case appleVision
    case android

    var usesLandscapeSideControls: Bool {
        switch self {
        case .appleWatch, .appleTV:
            return true
        case .iOS, .appleVision, .android:
            return false
        }
    }

    var allowedHostOrientations: UIInterfaceOrientationMask {
        usesLandscapeSideControls ? .allButUpsideDown : .portrait
    }
}

private extension SimulatorMetadata {
    var streamControlFamily: StreamControlFamily {
        let metadata = [
            platform,
            runtimeIdentifier,
            runtimeName,
            deviceTypeIdentifier,
            deviceTypeName,
            name,
            android?.avdName
        ]
        .compactMap { $0?.lowercased() }
        .joined(separator: " ")

        if android != nil || metadata.contains("android") || metadata.contains("pixel") {
            return .android
        }
        if metadata.contains("apple-tv") || metadata.contains("apple tv") || metadata.contains("tvos") {
            return .appleTV
        }
        if metadata.contains("apple-watch") || metadata.contains("apple watch") || metadata.contains("watchos") {
            return .appleWatch
        }
        if metadata.contains("vision") || metadata.contains("xros") {
            return .appleVision
        }
        return .iOS
    }
}

private struct StreamKeyControlButton: View {
    let title: LocalizedStringKey
    let systemImage: String
    let keyCode: Int
    @Bindable var model: AppModel

    init(_ title: LocalizedStringKey, systemImage: String, keyCode: Int, model: AppModel) {
        self.title = title
        self.systemImage = systemImage
        self.keyCode = keyCode
        self.model = model
    }

    var body: some View {
        Button {
            model.hapticSelection()
            _ = model.sendKey(keyCode: keyCode)
        } label: {
            StreamControlIconLabel(title: title, systemImage: systemImage)
        }
        .buttonStyle(.plain)
        .buttonBorderShape(.circle)
        .accessibilityLabel(title)
    }
}

private struct StreamTVRemotePad: View {
    @Bindable var model: AppModel

    var body: some View {
        VStack(spacing: 8) {
            StreamKeyControlButton("Up", systemImage: "chevron.up", keyCode: 82, model: model)

            HStack(spacing: 8) {
                StreamKeyControlButton("Left", systemImage: "chevron.left", keyCode: 80, model: model)

                StreamKeyControlButton("Select", systemImage: "smallcircle.filled.circle", keyCode: 40, model: model)

                StreamKeyControlButton("Right", systemImage: "chevron.right", keyCode: 79, model: model)
            }

            StreamKeyControlButton("Down", systemImage: "chevron.down", keyCode: 81, model: model)
        }
    }
}

private struct StreamCrownControlButton: View {
    let title: LocalizedStringKey
    let systemImage: String
    let delta: Double
    @Bindable var model: AppModel
    @Environment(\.isEnabled) private var isEnabled
    @State private var repeatTask: Task<Void, Never>?
    @State private var isPressed = false

    init(_ title: LocalizedStringKey, systemImage: String, delta: Double, model: AppModel) {
        self.title = title
        self.systemImage = systemImage
        self.delta = delta
        self.model = model
    }

    var body: some View {
        StreamControlIconLabel(title: title, systemImage: systemImage)
            .opacity(isPressed ? 0.55 : 1)
            .scaleEffect(isPressed ? 0.96 : 1)
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { _ in beginRepeating() }
                    .onEnded { _ in endRepeating() }
            )
            .onDisappear {
                endRepeating()
            }
            .animation(.snappy(duration: 0.12), value: isPressed)
        .buttonStyle(.plain)
        .buttonBorderShape(.circle)
        .accessibilityLabel(title)
        .accessibilityAddTraits(.isButton)
        .accessibilityAction {
            if isEnabled {
                sendTick()
            }
        }
    }

    private func beginRepeating() {
        guard isEnabled, repeatTask == nil else { return }
        isPressed = true
        sendTick()
        repeatTask = Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(260))
            while !Task.isCancelled {
                sendTick()
                try? await Task.sleep(for: .milliseconds(78))
            }
        }
    }

    private func endRepeating() {
        repeatTask?.cancel()
        repeatTask = nil
        isPressed = false
    }

    private func sendTick() {
        guard isEnabled else { return }
        model.rotateCrown(delta: delta)
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
    @Environment(\.isEnabled) private var isEnabled
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
                if isEnabled {
                    model.tapHardwareButton(named: buttonName)
                }
            }
    }

    private func pressDown() {
        guard isEnabled, !isPressed else { return }
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
    @State private var previousCrownDragLocation: CGPoint?
    @State private var accumulatedCrownDrag: CGFloat = 0
    @State private var totalCrownDragDistance: CGFloat = 0

    var body: some View {
        Color.clear
            .frame(width: hitFrame.width, height: hitFrame.height)
            .contentShape(Rectangle())
            .position(x: hitFrame.midX, y: hitFrame.midY)
            .gesture(hitGesture)
            .onDisappear {
                if isDigitalCrown {
                    resetCrownDrag()
                } else {
                    pressUp()
                }
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

    private var isDigitalCrown: Bool {
        buttonName == "digital-crown"
    }

    private var hitGesture: some Gesture {
        DragGesture(minimumDistance: 0)
            .onChanged { value in
                if isDigitalCrown {
                    updateCrownDrag(value)
                } else {
                    pressDown()
                }
            }
            .onEnded { _ in
                if isDigitalCrown {
                    finishCrownDrag()
                } else {
                    pressUp()
                }
            }
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

    private func updateCrownDrag(_ value: DragGesture.Value) {
        let location = value.location
        guard let previousCrownDragLocation else {
            self.previousCrownDragLocation = location
            accumulatedCrownDrag = 0
            totalCrownDragDistance = 0
            return
        }

        let verticalDelta = location.y - previousCrownDragLocation.y
        self.previousCrownDragLocation = location
        accumulatedCrownDrag += verticalDelta
        totalCrownDragDistance += abs(verticalDelta)

        let pointsPerTick: CGFloat = 7
        while abs(accumulatedCrownDrag) >= pointsPerTick {
            let direction = accumulatedCrownDrag > 0 ? 1.0 : -1.0
            model.rotateCrown(delta: direction * 32)
            accumulatedCrownDrag -= CGFloat(direction) * pointsPerTick
        }
    }

    private func finishCrownDrag() {
        if totalCrownDragDistance < 4 {
            model.tapHardwareButton(named: buttonName, usagePage: button.usagePage, usage: button.usage)
        }
        resetCrownDrag()
    }

    private func resetCrownDrag() {
        previousCrownDragLocation = nil
        accumulatedCrownDrag = 0
        totalCrownDragDistance = 0
    }
}

private struct NavigationPopGestureBlocker: UIViewRepresentable {
    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeUIView(context: Context) -> NavigationPopGestureBlockerView {
        let view = NavigationPopGestureBlockerView()
        view.coordinator = context.coordinator
        return view
    }

    func updateUIView(_ view: NavigationPopGestureBlockerView, context: Context) {
        view.coordinator = context.coordinator
        view.refreshPopGestureBlock()
    }

    static func dismantleUIView(_ view: NavigationPopGestureBlockerView, coordinator: Coordinator) {
        coordinator.detach()
    }

    final class Coordinator {
        private var proxy: InteractivePopGestureDelegateProxy?

        func attach(to navigationController: UINavigationController, blockedFrame: CGRect) {
            guard let gestureRecognizer = navigationController.interactivePopGestureRecognizer else {
                detach()
                return
            }

            if let proxy, proxy.gestureRecognizer === gestureRecognizer {
                proxy.navigationController = navigationController
                proxy.blockedFrame = blockedFrame
                return
            }

            detach()
            let originalDelegate = (gestureRecognizer.delegate as? InteractivePopGestureDelegateProxy)?.originalDelegate
                ?? gestureRecognizer.delegate
            let proxy = InteractivePopGestureDelegateProxy(
                navigationController: navigationController,
                gestureRecognizer: gestureRecognizer,
                originalDelegate: originalDelegate,
                blockedFrame: blockedFrame
            )
            self.proxy = proxy
            gestureRecognizer.delegate = proxy
        }

        func detach() {
            proxy?.restoreDelegate()
            proxy = nil
        }
    }
}

private final class NavigationPopGestureBlockerView: UIView {
    weak var coordinator: NavigationPopGestureBlocker.Coordinator?

    override init(frame: CGRect) {
        super.init(frame: frame)
        isOpaque = false
        backgroundColor = .clear
        isUserInteractionEnabled = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        nil
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        refreshPopGestureBlock()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        refreshPopGestureBlock()
    }

    func refreshPopGestureBlock() {
        guard let navigationController,
              let navigationView = navigationController.view,
              window != nil else {
            return
        }
        let blockedFrame = convert(bounds, to: navigationView)
        coordinator?.attach(to: navigationController, blockedFrame: blockedFrame)
    }

    private var navigationController: UINavigationController? {
        sequence(first: next, next: { $0?.next })
            .compactMap { responder -> UINavigationController? in
                if let navigationController = responder as? UINavigationController {
                    return navigationController
                }
                if let viewController = responder as? UIViewController {
                    return viewController.navigationController
                }
                return nil
            }
            .first
    }
}

private final class InteractivePopGestureDelegateProxy: NSObject, UIGestureRecognizerDelegate {
    weak var navigationController: UINavigationController?
    weak var gestureRecognizer: UIGestureRecognizer?
    weak var originalDelegate: UIGestureRecognizerDelegate?
    var blockedFrame: CGRect

    init(
        navigationController: UINavigationController,
        gestureRecognizer: UIGestureRecognizer,
        originalDelegate: UIGestureRecognizerDelegate?,
        blockedFrame: CGRect
    ) {
        self.navigationController = navigationController
        self.gestureRecognizer = gestureRecognizer
        self.originalDelegate = originalDelegate
        self.blockedFrame = blockedFrame
        super.init()
    }

    func restoreDelegate() {
        if gestureRecognizer?.delegate === self {
            gestureRecognizer?.delegate = originalDelegate
        }
    }

    func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
        if let navigationView = navigationController?.view,
           blockedFrame.contains(gestureRecognizer.location(in: navigationView)) {
            return false
        }
        if let decision = originalDelegate?.gestureRecognizerShouldBegin?(gestureRecognizer) {
            return decision
        }
        return (navigationController?.viewControllers.count ?? 0) > 1
    }

    func gestureRecognizer(
        _ gestureRecognizer: UIGestureRecognizer,
        shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer
    ) -> Bool {
        originalDelegate?.gestureRecognizer?(
            gestureRecognizer,
            shouldRecognizeSimultaneouslyWith: otherGestureRecognizer
        ) ?? false
    }

    func gestureRecognizer(
        _ gestureRecognizer: UIGestureRecognizer,
        shouldRequireFailureOf otherGestureRecognizer: UIGestureRecognizer
    ) -> Bool {
        originalDelegate?.gestureRecognizer?(
            gestureRecognizer,
            shouldRequireFailureOf: otherGestureRecognizer
        ) ?? false
    }

    func gestureRecognizer(
        _ gestureRecognizer: UIGestureRecognizer,
        shouldBeRequiredToFailBy otherGestureRecognizer: UIGestureRecognizer
    ) -> Bool {
        originalDelegate?.gestureRecognizer?(
            gestureRecognizer,
            shouldBeRequiredToFailBy: otherGestureRecognizer
        ) ?? false
    }

    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
        originalDelegate?.gestureRecognizer?(gestureRecognizer, shouldReceive: touch) ?? true
    }

    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive press: UIPress) -> Bool {
        originalDelegate?.gestureRecognizer?(gestureRecognizer, shouldReceive: press) ?? true
    }

    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive event: UIEvent) -> Bool {
        originalDelegate?.gestureRecognizer?(gestureRecognizer, shouldReceive: event) ?? true
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
    let screenBackingCornerRadius: CGFloat
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
            let backingRect = Self.chromeBackingRect(profile: chromeProfile)
            let contentRect = Self.chromeContentRect(profile: chromeProfile) ?? backingRect
            shellFrame = shell
            chromeCoordinateScale = scale
            screenFrame = CGRect(
                x: shell.minX + contentRect.minX * scale,
                y: shell.minY + contentRect.minY * scale,
                width: contentRect.width * scale,
                height: contentRect.height * scale
            )
            screenBackingFrame = CGRect(
                x: shell.minX + backingRect.minX * scale,
                y: shell.minY + backingRect.minY * scale,
                width: backingRect.width * scale,
                height: backingRect.height * scale
            )
            videoFrame = screenFrame
            screenCornerRadius = Self.screenCornerRadius(
                profile: chromeProfile,
                profileScreenRect: contentRect,
                scale: scale
            )
            screenBackingCornerRadius = Self.screenCornerRadius(
                profile: chromeProfile,
                profileScreenRect: backingRect,
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
        screenBackingCornerRadius = screenCornerRadius
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

    private static func chromeBackingRect(profile: ChromeProfile) -> CGRect {
        CGRect(
            x: CGFloat(profile.screenX),
            y: CGFloat(profile.screenY),
            width: CGFloat(profile.screenWidth),
            height: CGFloat(profile.screenHeight)
        )
    }

    private static func chromeContentRect(profile: ChromeProfile) -> CGRect? {
        guard let contentX = profile.contentX,
              let contentY = profile.contentY,
              let contentWidth = profile.contentWidth,
              let contentHeight = profile.contentHeight,
              contentWidth > 0,
              contentHeight > 0 else {
            return nil
        }
        return CGRect(
            x: CGFloat(contentX),
            y: CGFloat(contentY),
            width: CGFloat(contentWidth),
            height: CGFloat(contentHeight)
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
            if let contentX = profile.contentX,
               let contentY = profile.contentY,
               let contentWidth = profile.contentWidth,
               let contentHeight = profile.contentHeight,
               abs(profileScreenRect.minX - CGFloat(contentX)) <= 0.5,
               abs(profileScreenRect.minY - CGFloat(contentY)) <= 0.5,
               abs(profileScreenRect.width - CGFloat(contentWidth)) <= 0.5,
               abs(profileScreenRect.height - CGFloat(contentHeight)) <= 0.5 {
                return min(
                    profileScreenRect.width * scale / 2,
                    profileScreenRect.height * scale / 2,
                    CGFloat(profile.cornerRadius) * scale
                )
            }
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

}
