@preconcurrency import AVFoundation
import SwiftUI
import UIKit

struct ContentView: View {
    @Bindable var model: AppModel
    @State private var searchText = ""
    @State private var searchExpanded = false

    var body: some View {
        navigationContent(usesSearchAccessory: true)
        .task {
            model.start()
        }
    }

    private func navigationContent(usesSearchAccessory: Bool) -> some View {
        NavigationSplitView {
            SidebarView(
                model: model,
                searchText: $searchText,
                searchExpanded: $searchExpanded,
                usesSearchAccessory: usesSearchAccessory
            )
        } detail: {
            if model.endpoint != nil,
               model.authEndpoint == nil,
               model.selectedSimulator != nil {
                SimulatorStreamView(model: model)
            } else {
                Color.clear
            }
        }
    }
}

private struct SidebarView: View {
    @Bindable var model: AppModel
    @Binding var searchText: String
    @Binding var searchExpanded: Bool
    let usesSearchAccessory: Bool
    @State private var presentedSheet: SidebarSheet?
    @State private var isScanningPairingQR = false
    @State private var selectedSimulatorTypeFilter: SimulatorTypeFilter?

    private let simulatorFilterHeaderHeight: CGFloat = 48

    private var usesIPadSidebarLayout: Bool {
        UIDevice.current.userInterfaceIdiom == .pad
    }

    private var filteredSimulators: [SimulatorMetadata] {
        var simulators = model.simulators
        if let selectedSimulatorTypeFilter {
            simulators = simulators.filter { selectedSimulatorTypeFilter.matches($0) }
        }
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return simulators }
        return simulators.filter { simulator in
            simulator.name.localizedCaseInsensitiveContains(query)
                || simulator.subtitle.localizedCaseInsensitiveContains(query)
                || simulator.udid.localizedCaseInsensitiveContains(query)
        }
    }

    private var showsSimulatorTypeFilters: Bool {
        model.endpoint != nil && !model.simulators.isEmpty
    }

    var body: some View {
        sidebarContent
            .sheet(item: $presentedSheet) { sheet in
                switch sheet {
                case .servers:
                    ServerSelectionSheet(model: model)
                case .connect:
                    ConnectServerSheet(model: model)
                case .pair:
                    PairServerSheet(model: model)
                case .settings:
                    SettingsSheet(model: model)
                case .newSimulator:
                    NewSimulatorSheet(model: model)
                }
            }
            .sheet(isPresented: $isScanningPairingQR) {
                QRCodeScannerSheet(model: model) {
                    presentedSheet = nil
                }
            }
            .onChange(of: model.authEndpoint?.id) { _, endpointID in
                if endpointID != nil {
                    presentedSheet = .pair
                }
            }
    }

    @ViewBuilder
    private var sidebarContent: some View {
        if usesSearchAccessory {
            sidebarList
                .safeAreaInset(edge: .bottom, spacing: 0) {
                    SimulatorSearchDock(
                        model: model,
                        text: $searchText,
                        isExpanded: $searchExpanded,
                        showsSidebarActions: usesIPadSidebarLayout,
                        onSettings: {
                            model.hapticSelection()
                            presentedSheet = .settings
                        },
                        onConnect: {
                            model.hapticSelection()
                            presentedSheet = .connect
                        }
                    ) {
                        presentedSheet = .newSimulator
                    }
                }
        } else {
            sidebarList
        }
    }

    private var sidebarList: some View {
        ZStack(alignment: .top) {
            simulatorList
            if showsSimulatorTypeFilters {
                SimulatorTypeFilterBar(
                    selectedFilter: $selectedSimulatorTypeFilter,
                    hapticSelection: model.hapticSelection
                )
                .frame(height: simulatorFilterHeaderHeight, alignment: .top)
                .frame(maxWidth: .infinity, alignment: .topLeading)
                .zIndex(1)
            }
        }
        .task(id: model.endpoint?.id) {
            await model.refreshSimulatorsIfStale(maxAge: 0, silent: true)
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(6))
                await model.refreshSimulatorsIfStale(maxAge: 5, silent: true)
            }
        }
        .navigationTitle("")
        .navigationBarTitleDisplayMode(.inline)
        .overlay {
            if model.isBusy && model.simulators.isEmpty {
                ProgressView()
            } else if model.endpoint == nil {
                NoServerActionsView(
                    canSelectServer: !model.availableEndpoints.isEmpty,
                    selectServer: {
                        model.hapticSelection()
                        presentedSheet = .servers
                    },
                    pairServer: {
                        model.hapticSelection()
                        presentedSheet = .pair
                    },
                    scanQR: {
                        model.hapticSelection()
                        isScanningPairingQR = true
                    },
                    copyInstallCommands: {
                        model.hapticSuccess()
                    }
                )
            } else if model.authEndpoint != nil {
                VStack(spacing: 16) {
                    ContentUnavailableView("Pair Server", systemImage: "lock")
                    Button {
                        presentedSheet = .pair
                    } label: {
                        Label("Pair", systemImage: "checkmark.seal")
                    }
                    .buttonStyle(.borderedProminent)
                }
            } else if filteredSimulators.isEmpty {
                ContentUnavailableView(
                    emptySimulatorTitle,
                    systemImage: emptySimulatorSystemImage
                )
            }
        }
        .toolbar {
            if !usesIPadSidebarLayout {
                ToolbarItem(placement: .topBarLeading) {
                    Button {
                        model.hapticSelection()
                        presentedSheet = .settings
                    } label: {
                        Label("Settings", systemImage: "gearshape")
                    }
                }
            }
            ToolbarItem(placement: .principal) {
                ServerTitleButton(model: model, usesGlass: !usesIPadSidebarLayout) {
                    model.hapticSelection()
                    presentedSheet = .servers
                }
            }
            if !usesIPadSidebarLayout {
                ToolbarItem(placement: .primaryAction) {
                    Button {
                        model.hapticSelection()
                        presentedSheet = .connect
                    } label: {
                        Label("Connect", systemImage: "personalhotspot")
                    }
                }
            }
        }
        .modifier(SidebarNavigationBarBackgroundModifier(hidden: usesIPadSidebarLayout))
    }

    private var simulatorList: some View {
        List(selection: simulatorSelection) {
            simulatorRows
        }
        .refreshable {
            await model.refreshSimulators()
        }
        .contentMargins(
            .top,
            showsSimulatorTypeFilters ? simulatorFilterHeaderHeight : 0,
            for: .scrollContent
        )
        .background {
            RefreshControlOffsetProbe(offset: showsSimulatorTypeFilters ? simulatorFilterHeaderHeight : 0)
        }
    }

    @ViewBuilder
    private var simulatorRows: some View {
        ForEach(filteredSimulators) { simulator in
            SimulatorRow(
                simulator: simulator,
                isBusy: model.isSimulatorLifecycleBusy(simulator)
            )
                .tag(simulator.udid)
                .contextMenu {
                    SimulatorLifecycleMenuItems(model: model, simulator: simulator)
                }
        }
    }

    private var emptySimulatorTitle: String {
        if !searchText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || selectedSimulatorTypeFilter != nil {
            return "No Results"
        }
        return "No Simulators"
    }

    private var emptySimulatorSystemImage: String {
        selectedSimulatorTypeFilter?.systemImage
            ?? (searchText.isEmpty ? "iphone.slash" : "magnifyingglass")
    }

    private var simulatorSelection: Binding<String?> {
        Binding {
            model.selectedSimulatorID
        } set: { udid in
            model.selectSimulator(udid)
        }
    }

}

private struct RefreshControlOffsetProbe: UIViewRepresentable {
    let offset: CGFloat

    func makeUIView(context: Context) -> RefreshControlOffsetProbeView {
        let view = RefreshControlOffsetProbeView()
        view.isUserInteractionEnabled = false
        view.backgroundColor = .clear
        return view
    }

    func updateUIView(_ uiView: RefreshControlOffsetProbeView, context: Context) {
        uiView.offset = offset
    }
}

private final class RefreshControlOffsetProbeView: UIView {
    var offset: CGFloat = 0 {
        didSet {
            scheduleRefreshControlUpdate()
        }
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        scheduleRefreshControlUpdate()
    }

    override func didMoveToSuperview() {
        super.didMoveToSuperview()
        scheduleRefreshControlUpdate()
    }

    private func scheduleRefreshControlUpdate(attemptsRemaining: Int = 6) {
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.02) { [weak self] in
            guard let self else { return }
            if self.applyRefreshControlOffset() || attemptsRemaining <= 0 {
                return
            }
            self.scheduleRefreshControlUpdate(attemptsRemaining: attemptsRemaining - 1)
        }
    }

    @discardableResult
    private func applyRefreshControlOffset() -> Bool {
        guard let refreshControl = nearbyRefreshControl() else {
            return false
        }
        refreshControl.transform = CGAffineTransform(translationX: 0, y: offset)
        return true
    }

    private func nearbyRefreshControl() -> UIRefreshControl? {
        var ancestor: UIView? = self
        while let view = ancestor {
            if let scrollView = view as? UIScrollView,
               let refreshControl = scrollView.refreshControl {
                return refreshControl
            }
            ancestor = view.superview
        }

        ancestor = superview
        while let view = ancestor {
            if let refreshControl = view.firstDescendantRefreshControl() {
                return refreshControl
            }
            ancestor = view.superview
        }
        return nil
    }
}

private extension UIView {
    func firstDescendantRefreshControl() -> UIRefreshControl? {
        if let scrollView = self as? UIScrollView,
           let refreshControl = scrollView.refreshControl,
           scrollView.alwaysBounceVertical || scrollView.contentSize.height > scrollView.bounds.height {
            return refreshControl
        }
        for subview in subviews {
            if let refreshControl = subview.firstDescendantRefreshControl() {
                return refreshControl
            }
        }
        return nil
    }
}

private enum SidebarSheet: Identifiable {
    case servers
    case connect
    case pair
    case settings
    case newSimulator

    var id: Self { self }
}

private enum SimulatorTypeFilter: String, CaseIterable, Identifiable {
    case iPhone
    case iPad
    case appleWatch
    case appleTV
    case appleVision
    case android

    var id: Self { self }

    var title: String {
        switch self {
        case .iPhone: "iPhone"
        case .iPad: "iPad"
        case .appleWatch: "Watch"
        case .appleTV: "TV"
        case .appleVision: "Vision"
        case .android: "Android"
        }
    }

    var systemImage: String {
        switch self {
        case .iPhone: "iphone"
        case .iPad: "ipad"
        case .appleWatch: "applewatch"
        case .appleTV: "appletv"
        case .appleVision: "visionpro"
        case .android: "rectangle.portrait"
        }
    }

    func matches(_ simulator: SimulatorMetadata) -> Bool {
        simulator.typeFilter == self
    }
}

private struct SimulatorTypeFilterBar: View {
    @Binding var selectedFilter: SimulatorTypeFilter?
    let hapticSelection: () -> Void

    var body: some View {
        ScrollView(.horizontal) {
            if #available(iOS 26.0, *) {
                GlassEffectContainer(spacing: 8) {
                    pillRow
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 8)
            } else {
                pillRow
                    .padding(.horizontal, 16)
                    .padding(.vertical, 8)
            }
        }
        .scrollIndicators(.hidden)
        .padding(.bottom, 4)
    }

    private var pillRow: some View {
        HStack(spacing: 8) {
            ForEach(SimulatorTypeFilter.allCases) { filter in
                SimulatorTypeFilterPill(
                    filter: filter,
                    isSelected: selectedFilter == filter
                ) {
                    hapticSelection()
                    withAnimation(.snappy(duration: 0.18)) {
                        selectedFilter = selectedFilter == filter ? nil : filter
                    }
                }
            }
        }
    }
}

private struct SimulatorTypeFilterPill: View {
    let filter: SimulatorTypeFilter
    let isSelected: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Label(filter.title, systemImage: filter.systemImage)
                .font(.caption.weight(isSelected ? .bold : .semibold))
                .imageScale(.small)
                .lineLimit(1)
                .padding(.horizontal, 11)
                .padding(.vertical, 7)
                .foregroundStyle(isSelected ? .primary : .secondary)
                .contentShape(Capsule())
                .modifier(GlassCapsuleModifier(interactive: true))
                .overlay {
                    Capsule()
                        .strokeBorder(
                            Color.primary.opacity(isSelected ? 0.36 : 0.12),
                            lineWidth: isSelected ? 1.5 : 1
                        )
                }
                .scaleEffect(isSelected ? 1.03 : 1)
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
        .animation(.snappy(duration: 0.18), value: isSelected)
    }
}

private extension SimulatorMetadata {
    var typeFilter: SimulatorTypeFilter {
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
        if metadata.contains("ipad") {
            return .iPad
        }
        return .iPhone
    }
}

private struct NoServerActionsView: View {
    private static let installCommands = "npm i -g simdeck\nsimdeck pair"

    @State private var isCopiedToastVisible = false
    @State private var copiedToastGeneration = 0

    let canSelectServer: Bool
    let selectServer: () -> Void
    let pairServer: () -> Void
    let scanQR: () -> Void
    let copyInstallCommands: () -> Void

    var body: some View {
        ZStack(alignment: .top) {
            onboardingStack
            if isCopiedToastVisible {
                copiedToast
                    .offset(y: -52)
                    .transition(.opacity.combined(with: .scale(scale: 0.94)))
            }
        }
        .controlSize(.large)
        .frame(maxWidth: 300)
        .padding(.horizontal, 24)
        .animation(.snappy(duration: 0.18), value: isCopiedToastVisible)
        .task(id: copiedToastGeneration) {
            guard isCopiedToastVisible else { return }
            try? await Task.sleep(for: .milliseconds(1_250))
            guard !Task.isCancelled else { return }
            withAnimation(.snappy(duration: 0.18)) {
                isCopiedToastVisible = false
            }
        }
    }

    private var onboardingStack: some View {
        VStack(spacing: 24) {
            installCommandBlock
            VStack(spacing: 14) {
                if canSelectServer {
                    neutralActionButton("Select a Server", systemImage: "server.rack", action: selectServer)
                }
                neutralActionButton("Pair New Server", systemImage: "checkmark.seal", action: pairServer)
                neutralActionButton("Scan QR", systemImage: "qrcode.viewfinder", action: scanQR)
            }
        }
    }

    private var installCommandBlock: some View {
        Button {
            UIPasteboard.general.string = Self.installCommands
            copyInstallCommands()
            copiedToastGeneration += 1
            withAnimation(.snappy(duration: 0.18)) {
                isCopiedToastVisible = true
            }
        } label: {
            Text(verbatim: Self.installCommands)
                .font(.system(.callout, design: .monospaced).weight(.semibold))
                .foregroundStyle(.primary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 14)
                .padding(.vertical, 12)
                .contentShape(.rect(cornerRadius: 12))
        }
        .buttonStyle(.plain)
        .modifier(GlassRoundedRectangleModifier(cornerRadius: 12, interactive: true))
        .accessibilityLabel("Install command npm i dash g simdeck, then simdeck pair")
        .accessibilityHint("Copies the commands")
    }

    private var copiedToast: some View {
        Text("Copied")
            .font(.subheadline.weight(.semibold))
            .foregroundStyle(.primary)
            .padding(.horizontal, 16)
            .frame(height: 38)
            .contentShape(Capsule())
            .modifier(GlassCapsuleModifier(interactive: false))
    }

    private func neutralActionButton(
        _ title: String,
        systemImage: String,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            Label(title, systemImage: systemImage)
                .font(.body.weight(.semibold))
                .foregroundStyle(.primary)
                .frame(maxWidth: .infinity)
                .frame(height: 48)
                .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .modifier(GlassCapsuleModifier(interactive: true))
    }
}

private struct ServerTitleButton: View {
    @Bindable var model: AppModel
    let usesGlass: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Circle()
                    .fill(titleColor)
                    .frame(width: 7, height: 7)
                Spacer(minLength: 4)
                VStack(alignment: .center, spacing: 1) {
                    Text(model.selectedEndpointTitle)
                        .font(.headline)
                        .lineLimit(1)
                        .multilineTextAlignment(.center)
                    Text(model.selectedEndpointSubtitle)
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
        .frame(minWidth: 170, maxWidth: 240)
        .frame(height: 42)
        .modifier(ServerTitleChromeModifier(usesGlass: usesGlass))
    }

    private var titleColor: Color {
        if model.authEndpoint != nil {
            return .orange
        }
        return model.endpoint == nil ? .secondary : .green
    }
}

private struct ServerTitleChromeModifier: ViewModifier {
    let usesGlass: Bool

    @ViewBuilder
    func body(content: Content) -> some View {
        if usesGlass {
            content.modifier(GlassCapsuleModifier(interactive: true))
        } else {
            content
        }
    }
}

private struct SidebarNavigationBarBackgroundModifier: ViewModifier {
    let hidden: Bool

    @ViewBuilder
    func body(content: Content) -> some View {
        if hidden {
            content.toolbarBackground(.hidden, for: .navigationBar)
        } else {
            content
        }
    }
}

private struct ServerDetailPresentation: Identifiable {
    var id: String { endpoint.id }

    let endpoint: SimDeckEndpoint
    let saveEndpoint: Bool
}

private struct ServerSelectionSheet: View {
    @Bindable var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var detailPresentation: ServerDetailPresentation?
    @State private var renamingEndpoint: SimDeckEndpoint?
    @State private var renameText = ""

    private var hasServers: Bool {
        !model.savedEndpoints.isEmpty || !model.automaticEndpoints.isEmpty
    }

    var body: some View {
        NavigationStack {
            serverContent
            .alert("Rename Server", isPresented: renameAlertBinding) {
                TextField("Name", text: $renameText)
                Button("Cancel", role: .cancel) {
                    renamingEndpoint = nil
                }
                Button("Save") {
                    if let renamingEndpoint {
                        model.renameSavedEndpoint(renamingEndpoint, to: renameText)
                    }
                    renamingEndpoint = nil
                }
                .disabled(renameText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            .navigationTitle("SimDeck Servers")
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
                        model.discovery.refresh()
                    } label: {
                        Label("Refresh", systemImage: "arrow.clockwise")
                    }
                    .disabled(model.discovery.isScanning)
                }
            }
            .sheet(item: $detailPresentation) { presentation in
                ServerDetailSheet(
                    endpoint: presentation.endpoint,
                    isConnected: model.endpoint.map {
                        $0.baseURL == presentation.endpoint.baseURL
                    } ?? false
                ) { endpoint in
                    let connected = await model.connect(
                        endpoint,
                        autoStart: false,
                        saveEndpoint: presentation.saveEndpoint
                    )
                    if connected {
                        dismiss()
                    }
                    return connected
                }
            }
        }
        .presentationDetents([.medium, .large])
    }

    @ViewBuilder
    private var serverContent: some View {
        if hasServers {
            List {
                if !model.savedEndpoints.isEmpty {
                    Section("Saved") {
                        ForEach(model.savedEndpoints) { endpoint in
                            serverButton(endpoint, saveEndpoint: false)
                                .swipeActions(edge: .trailing) {
                                    Button(role: .destructive) {
                                        model.deleteSavedEndpoint(endpoint)
                                    } label: {
                                        Label("Delete", systemImage: "trash")
                                    }
                                    Button {
                                        beginRenaming(endpoint)
                                    } label: {
                                        Label("Rename", systemImage: "pencil")
                                    }
                                }
                                .contextMenu {
                                    Button {
                                        beginRenaming(endpoint)
                                    } label: {
                                        Label("Rename", systemImage: "pencil")
                                    }
                                    Button(role: .destructive) {
                                        model.deleteSavedEndpoint(endpoint)
                                    } label: {
                                        Label("Delete", systemImage: "trash")
                                    }
                                }
                        }
                    }
                }

                if !model.automaticEndpoints.isEmpty {
                    Section("Auto-Detected") {
                        ForEach(model.automaticEndpoints) { endpoint in
                            serverButton(endpoint, saveEndpoint: false)
                        }
                    }
                }
            }
        } else {
            ContentUnavailableView("No Servers", systemImage: "server.rack")
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private func serverButton(_ endpoint: SimDeckEndpoint, saveEndpoint: Bool) -> some View {
        Button {
            model.hapticSelection()
            detailPresentation = ServerDetailPresentation(endpoint: endpoint, saveEndpoint: saveEndpoint)
        } label: {
            HStack(spacing: 12) {
                EndpointRow(endpoint: endpoint)
                Spacer()
                if model.endpoint?.baseURL == endpoint.baseURL {
                    Image(systemName: "checkmark")
                        .font(.headline)
                        .foregroundStyle(.tint)
                }
                Image(systemName: "chevron.right")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.tertiary)
            }
        }
        .buttonStyle(.plain)
    }

    private var renameAlertBinding: Binding<Bool> {
        Binding {
            renamingEndpoint != nil
        } set: { isPresented in
            if !isPresented {
                renamingEndpoint = nil
            }
        }
    }

    private func beginRenaming(_ endpoint: SimDeckEndpoint) {
        model.hapticSelection()
        renamingEndpoint = endpoint
        renameText = endpoint.name
    }
}

private struct ServerDetailSheet: View {
    let endpoint: SimDeckEndpoint
    let isConnected: Bool
    let connect: (SimDeckEndpoint) async -> Bool

    @Environment(\.dismiss) private var dismiss
    @State private var isConnecting = false

    var body: some View {
        NavigationStack {
            Form {
                Section("Server") {
                    LabeledContent("Name", value: endpoint.displayName)
                    if let serverKindLabel = endpoint.serverKindLabel {
                        LabeledContent("Service", value: serverKindLabel)
                    }
                    LabeledContent("Source", value: endpoint.source.label)
                    LabeledContent("Status", value: statusLabel)
                    if let hostName = endpoint.hostName?.nilIfBlank {
                        LabeledContent("Host", value: hostName)
                    }
                }

                Section("App IPs") {
                    ForEach(Array(endpoint.allBaseURLs.enumerated()), id: \.element) { index, url in
                        ServerAddressRow(
                            url: url,
                            title: index == 0 ? "Active" : "Alternate \(index)"
                        )
                    }
                }

                if endpoint.serverID?.nilIfBlank != nil || endpoint.hostID?.nilIfBlank != nil {
                    Section("Identifiers") {
                        if let serverID = endpoint.serverID?.nilIfBlank {
                            LabeledContent("Server ID", value: serverID)
                        }
                        if let hostID = endpoint.hostID?.nilIfBlank {
                            LabeledContent("Host ID", value: hostID)
                        }
                    }
                }

                Section {
                    Button {
                        connectToEndpoint()
                    } label: {
                        if isConnecting {
                            ProgressView()
                                .controlSize(.small)
                        } else {
                            Label(isConnected ? "Reconnect" : "Connect", systemImage: "bolt.horizontal")
                        }
                    }
                    .disabled(isConnecting)
                }
            }
            .navigationTitle(endpoint.displayName)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") {
                        dismiss()
                    }
                }
            }
        }
        .presentationDetents([.medium, .large])
    }

    private var statusLabel: String {
        if endpoint.requiresPairing {
            "Pairing required"
        } else if endpoint.token?.nilIfBlank != nil {
            "Paired"
        } else {
            "Available"
        }
    }

    private func connectToEndpoint() {
        guard !isConnecting else { return }
        isConnecting = true
        Task {
            if await connect(endpoint) {
                dismiss()
            }
            isConnecting = false
        }
    }
}

private struct ServerAddressRow: View {
    let url: URL
    let title: String

    var body: some View {
        Label {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                Text(displayURL)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .textSelection(.enabled)
            }
        } icon: {
            Image(systemName: "network")
                .foregroundStyle(.secondary)
        }
    }

    private var displayURL: String {
        guard let host = url.host(percentEncoded: false)?.nilIfBlank else {
            return url.absoluteString
        }
        if let port = url.port {
            return "\(host):\(port)"
        }
        return host
    }
}

private struct PairServerSheet: View {
    @Bindable var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var isScanning = false

    var body: some View {
        NavigationStack {
            pairContent
                .navigationTitle("Pair Server")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Done") {
                            model.hapticSelection()
                            dismiss()
                        }
                    }
                }
        }
        .sheet(isPresented: $isScanning) {
            QRCodeScannerSheet(model: model) {
                dismiss()
            }
        }
        .presentationDetents([.medium])
    }

    @ViewBuilder
    private var pairContent: some View {
        if #available(iOS 26.0, *) {
            GlassEffectContainer(spacing: 0) {
                centeredPairButton
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            centeredPairButton
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private var centeredPairButton: some View {
        Button {
            model.hapticSelection()
            isScanning = true
        } label: {
            Text("Pair")
                .font(.headline)
                .foregroundStyle(.primary)
                .frame(width: 180, height: 52)
                .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .modifier(GlassCapsuleModifier(interactive: true))
    }
}

private struct SettingsSheet: View {
    @Bindable var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var isResetConnectionsConfirmationPresented = false

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Toggle("Haptics", isOn: $model.hapticsEnabled)
                        .onChange(of: model.hapticsEnabled) { _, enabled in
                            if enabled {
                                model.hapticSuccess()
                            }
                        }
                }
                Section {
                    Button(role: .destructive) {
                        model.hapticSelection()
                        isResetConnectionsConfirmationPresented = true
                    } label: {
                        Label("Reset Connections", systemImage: "trash")
                    }
                } footer: {
                    Text("Clears saved servers and pairing credentials from this iPhone.")
                }
            }
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") {
                        model.hapticSelection()
                        dismiss()
                    }
                }
            }
            .confirmationDialog(
                "Reset Connections?",
                isPresented: $isResetConnectionsConfirmationPresented,
                titleVisibility: .visible
            ) {
                Button("Reset Connections", role: .destructive) {
                    model.resetConnections()
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("All saved servers and pairing credentials will be removed.")
            }
        }
        .presentationDetents([.medium])
    }
}

private struct NewSimulatorSheet: View {
    @Bindable var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var options: SimulatorCreateOptionsResponse?
    @State private var platform: CreationPlatform = .ios
    @State private var name = ""
    @State private var nameDirty = false
    @State private var deviceTypeIdentifier = ""
    @State private var runtimeIdentifier = ""
    @State private var pairedWatch = false
    @State private var watchName = ""
    @State private var watchNameDirty = false
    @State private var watchDeviceTypeIdentifier = ""
    @State private var watchRuntimeIdentifier = ""
    @State private var androidName = ""
    @State private var androidNameDirty = false
    @State private var androidDeviceTypeIdentifier = ""
    @State private var androidSystemImageIdentifier = ""
    @State private var isLoading = false
    @State private var isCreating = false
    @State private var error = ""

    private var runtimeOptions: [SimulatorRuntimeOption] {
        compatibleRuntimes(deviceTypeIdentifier, options: options)
    }

    private var watchRuntimeOptions: [SimulatorRuntimeOption] {
        compatibleRuntimes(watchDeviceTypeIdentifier, options: options)
    }

    private var watchDeviceTypes: [SimulatorDeviceTypeOption] {
        (options?.deviceTypes ?? []).filter {
            isWatchDeviceType($0) && !compatibleRuntimes($0.identifier, options: options).isEmpty
        }
    }

    private var selectedDeviceType: SimulatorDeviceTypeOption? {
        options?.deviceTypes.first { $0.identifier == deviceTypeIdentifier }
    }

    private var selectedAndroidDeviceType: AndroidEmulatorDeviceTypeOption? {
        options?.android?.deviceTypes.first { $0.identifier == androidDeviceTypeIdentifier }
    }

    private var selectedAndroidSystemImage: AndroidEmulatorSystemImageOption? {
        options?.android?.systemImages.first { $0.identifier == androidSystemImageIdentifier }
    }

    private var pairedWatchAvailable: Bool {
        guard let selectedDeviceType else { return false }
        return isPhoneDeviceType(selectedDeviceType) && !watchDeviceTypes.isEmpty && !watchRuntimeOptions.isEmpty
    }

    private var canCreate: Bool {
        switch platform {
        case .ios:
            let baseReady = !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                && !deviceTypeIdentifier.isEmpty
                && !runtimeIdentifier.isEmpty
            let watchReady = !pairedWatch || (
                !watchName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    && !watchDeviceTypeIdentifier.isEmpty
                    && !watchRuntimeIdentifier.isEmpty
            )
            return baseReady && watchReady
        case .android:
            return !(options?.android?.unavailableReason?.isEmpty == false)
                && !androidName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                && !androidDeviceTypeIdentifier.isEmpty
                && !androidSystemImageIdentifier.isEmpty
        }
    }

    var body: some View {
        NavigationStack {
            Form {
                Picker("Platform", selection: $platform) {
                    ForEach(CreationPlatform.allCases) { platform in
                        Text(platform.label).tag(platform)
                    }
                }
                .pickerStyle(.segmented)
                .onChange(of: platform) { _, _ in
                    model.hapticSelection()
                    error = ""
                    if platform == .android {
                        pairedWatch = false
                    }
                }

                if isLoading {
                    Section {
                        HStack {
                            Spacer()
                            ProgressView()
                            Spacer()
                        }
                    }
                } else if platform == .android {
                    androidFields
                } else {
                    iosFields
                    if pairedWatchAvailable {
                        Section {
                            Toggle("Paired Apple Watch", isOn: $pairedWatch)
                                .onChange(of: pairedWatch) { _, _ in model.hapticSelection() }
                        }
                    }
                    if pairedWatch {
                        watchFields
                    }
                }

                if !error.isEmpty {
                    Section {
                        Text(error)
                            .font(.footnote)
                            .foregroundStyle(.red)
                    }
                } else if !model.status.isEmpty {
                    Section {
                        Text(model.status)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            .navigationTitle("New Simulator")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") {
                        model.hapticSelection()
                        dismiss()
                    }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(isCreating ? "Creating" : "Create") {
                        Task { await create() }
                    }
                    .disabled(isLoading || isCreating || !canCreate)
                }
            }
            .task {
                await loadOptions()
            }
        }
        .presentationDetents([.large])
    }

    private var iosFields: some View {
        Section("iOS Simulator") {
            TextField("Simulator Name", text: $name)
                .onChange(of: name) { _, _ in nameDirty = true }
            Picker("Device Type", selection: $deviceTypeIdentifier) {
                ForEach(options?.deviceTypes.filter { !isWatchDeviceType($0) } ?? []) { deviceType in
                    Text(deviceType.name).tag(deviceType.identifier)
                }
            }
            .onChange(of: deviceTypeIdentifier) { _, identifier in
                model.hapticSelection()
                let deviceType = options?.deviceTypes.first { $0.identifier == identifier }
                runtimeIdentifier = chooseCompatibleRuntime(identifier, options: options)?.identifier ?? ""
                if !nameDirty {
                    name = deviceType?.name ?? ""
                }
            }
            Picker("OS Version", selection: $runtimeIdentifier) {
                ForEach(runtimeOptions) { runtime in
                    Text(runtime.name).tag(runtime.identifier)
                }
            }
            .onChange(of: runtimeIdentifier) { _, _ in model.hapticSelection() }
        }
    }

    private var watchFields: some View {
        Section("Apple Watch") {
            TextField("Watch Name", text: $watchName)
                .onChange(of: watchName) { _, _ in watchNameDirty = true }
            Picker("Device Type", selection: $watchDeviceTypeIdentifier) {
                ForEach(watchDeviceTypes) { deviceType in
                    Text(deviceType.name).tag(deviceType.identifier)
                }
            }
            .onChange(of: watchDeviceTypeIdentifier) { _, identifier in
                model.hapticSelection()
                let deviceType = options?.deviceTypes.first { $0.identifier == identifier }
                watchRuntimeIdentifier = chooseCompatibleRuntime(identifier, options: options)?.identifier ?? ""
                if !watchNameDirty {
                    watchName = deviceType?.name ?? ""
                }
            }
            Picker("OS Version", selection: $watchRuntimeIdentifier) {
                ForEach(watchRuntimeOptions) { runtime in
                    Text(runtime.name).tag(runtime.identifier)
                }
            }
            .onChange(of: watchRuntimeIdentifier) { _, _ in model.hapticSelection() }
        }
    }

    private var androidFields: some View {
        Section("Android Emulator") {
            if let unavailableReason = options?.android?.unavailableReason {
                Text(unavailableReason)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            TextField("Emulator Name", text: $androidName)
                .onChange(of: androidName) { _, _ in androidNameDirty = true }
            Picker("Device Profile", selection: $androidDeviceTypeIdentifier) {
                ForEach(options?.android?.deviceTypes ?? []) { deviceType in
                    Text(deviceType.name).tag(deviceType.identifier)
                }
            }
            .onChange(of: androidDeviceTypeIdentifier) { _, identifier in
                model.hapticSelection()
                let deviceType = options?.android?.deviceTypes.first { $0.identifier == identifier }
                if !androidNameDirty, let deviceType {
                    androidName = defaultAndroidName(deviceType: deviceType, systemImage: selectedAndroidSystemImage)
                }
            }
            Picker("System Image", selection: $androidSystemImageIdentifier) {
                ForEach(options?.android?.systemImages ?? []) { systemImage in
                    Text(systemImage.name).tag(systemImage.identifier)
                }
            }
            .onChange(of: androidSystemImageIdentifier) { _, identifier in
                model.hapticSelection()
                let systemImage = options?.android?.systemImages.first { $0.identifier == identifier }
                if !androidNameDirty, let selectedAndroidDeviceType {
                    androidName = defaultAndroidName(deviceType: selectedAndroidDeviceType, systemImage: systemImage)
                }
            }
        }
    }

    private func loadOptions() async {
        guard options == nil, let endpoint = model.endpoint else { return }
        isLoading = true
        error = ""
        defer { isLoading = false }
        do {
            let loadedOptions = try await SimDeckAPI(endpoint: endpoint).simulatorCreateOptions()
            options = loadedOptions
            applyDefaults(from: loadedOptions)
        } catch {
            self.error = error.localizedDescription
            model.hapticWarning()
        }
    }

    private func applyDefaults(from options: SimulatorCreateOptionsResponse) {
        platform = model.selectedSimulator?.platform == "android-emulator" ? .android : .ios
        let initialDeviceType = chooseInitialDeviceType(
            options.deviceTypes,
            selectedDeviceTypeIdentifier: model.selectedSimulator?.deviceTypeIdentifier
        )
        deviceTypeIdentifier = initialDeviceType?.identifier ?? ""
        runtimeIdentifier = chooseCompatibleRuntime(
            initialDeviceType?.identifier ?? "",
            options: options,
            preferredIdentifier: model.selectedSimulator?.runtimeIdentifier
        )?.identifier ?? ""
        name = initialDeviceType?.name ?? ""
        nameDirty = false

        let initialWatchDeviceType = chooseInitialWatchDeviceType(options)
        watchDeviceTypeIdentifier = initialWatchDeviceType?.identifier ?? ""
        watchRuntimeIdentifier = chooseCompatibleRuntime(
            initialWatchDeviceType?.identifier ?? "",
            options: options
        )?.identifier ?? ""
        watchName = initialWatchDeviceType?.name ?? ""
        watchNameDirty = false
        pairedWatch = false

        let initialAndroidDeviceType = chooseInitialAndroidDeviceType(
            options,
            preferredName: model.selectedSimulator?.android?.avdName
        )
        let initialAndroidSystemImage = options.android?.systemImages.first
        androidDeviceTypeIdentifier = initialAndroidDeviceType?.identifier ?? ""
        androidSystemImageIdentifier = initialAndroidSystemImage?.identifier ?? ""
        if let initialAndroidDeviceType {
            androidName = defaultAndroidName(deviceType: initialAndroidDeviceType, systemImage: initialAndroidSystemImage)
        }
        androidNameDirty = false
    }

    private func create() async {
        guard canCreate else { return }
        model.hapticSelection()
        isCreating = true
        error = ""
        defer { isCreating = false }
        let request = CreateSimulatorRequest(
            platform: platform.rawValue,
            name: platform == .android
                ? androidName.trimmingCharacters(in: .whitespacesAndNewlines)
                : name.trimmingCharacters(in: .whitespacesAndNewlines),
            deviceTypeIdentifier: platform == .android ? androidDeviceTypeIdentifier : deviceTypeIdentifier,
            runtimeIdentifier: platform == .android ? androidSystemImageIdentifier : runtimeIdentifier,
            pairedWatch: platform == .ios && pairedWatch
                ? CreatePairedWatchRequest(
                    name: watchName.trimmingCharacters(in: .whitespacesAndNewlines),
                    deviceTypeIdentifier: watchDeviceTypeIdentifier,
                    runtimeIdentifier: watchRuntimeIdentifier
                )
                : nil
        )
        if await model.createSimulator(request) {
            dismiss()
        } else {
            error = model.status
        }
    }
}

private enum CreationPlatform: String, CaseIterable, Identifiable {
    case ios
    case android

    var id: Self { self }

    var label: String {
        switch self {
        case .ios: "iOS"
        case .android: "Android"
        }
    }
}

private func chooseInitialDeviceType(
    _ deviceTypes: [SimulatorDeviceTypeOption],
    selectedDeviceTypeIdentifier: String?
) -> SimulatorDeviceTypeOption? {
    deviceTypes.first { $0.identifier == selectedDeviceTypeIdentifier }
        ?? deviceTypes.first(where: isPhoneDeviceType)
        ?? deviceTypes.first { !isWatchDeviceType($0) }
        ?? deviceTypes.first
}

private func chooseInitialWatchDeviceType(_ options: SimulatorCreateOptionsResponse) -> SimulatorDeviceTypeOption? {
    options.deviceTypes.first {
        isWatchDeviceType($0) && !compatibleRuntimes($0.identifier, options: options).isEmpty
    }
}

private func chooseInitialAndroidDeviceType(
    _ options: SimulatorCreateOptionsResponse,
    preferredName: String?
) -> AndroidEmulatorDeviceTypeOption? {
    let deviceTypes = options.android?.deviceTypes ?? []
    return deviceTypes.first { $0.identifier == preferredName }
        ?? deviceTypes.first { $0.identifier == "pixel_8" }
        ?? deviceTypes.first { $0.identifier.hasPrefix("pixel_") }
        ?? deviceTypes.first
}

private func chooseCompatibleRuntime(
    _ deviceTypeIdentifier: String,
    options: SimulatorCreateOptionsResponse?,
    preferredIdentifier: String? = nil
) -> SimulatorRuntimeOption? {
    let runtimes = compatibleRuntimes(deviceTypeIdentifier, options: options)
    return runtimes.first { $0.identifier == preferredIdentifier } ?? runtimes.first
}

private func compatibleRuntimes(
    _ deviceTypeIdentifier: String,
    options: SimulatorCreateOptionsResponse?
) -> [SimulatorRuntimeOption] {
    guard !deviceTypeIdentifier.isEmpty, let options else { return [] }
    let deviceType = options.deviceTypes.first { $0.identifier == deviceTypeIdentifier }
    return options.runtimes.filter { runtime in
        if runtime.isAvailable == false {
            return false
        }
        return runtime.supportedDeviceTypeIdentifiers?.contains(deviceTypeIdentifier) == true
            || deviceType?.supportedRuntimeIdentifiers?.contains(runtime.identifier) == true
    }
}

private func isPhoneDeviceType(_ deviceType: SimulatorDeviceTypeOption) -> Bool {
    (deviceType.productFamily ?? "").lowercased() == "iphone"
}

private func isWatchDeviceType(_ deviceType: SimulatorDeviceTypeOption) -> Bool {
    (deviceType.productFamily ?? "").lowercased().contains("watch")
}

private func defaultAndroidName(
    deviceType: AndroidEmulatorDeviceTypeOption,
    systemImage: AndroidEmulatorSystemImageOption?
) -> String {
    let apiSuffix = systemImage?.apiLevel.map { "_API_\($0)" } ?? ""
    let raw = "\(deviceType.name)\(apiSuffix)"
    let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "_.-"))
    let sanitized = raw.map { character in
        character.unicodeScalars.allSatisfy { allowed.contains($0) } ? String(character) : "_"
    }.joined()
    return sanitized
        .replacingOccurrences(of: "_+", with: "_", options: .regularExpression)
        .trimmingCharacters(in: CharacterSet(charactersIn: "_"))
}

private struct ConnectServerSheet: View {
    @Bindable var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var isScanning = false

    var body: some View {
        NavigationStack {
            Form {
                Section("Server") {
                    TextField("Host or Studio URL", text: $model.manualAddress)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .keyboardType(.URL)
                    SecureField("Token", text: $model.manualToken)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    Button {
                        model.hapticSelection()
                        isScanning = true
                    } label: {
                        Label("Scan Pairing QR", systemImage: "qrcode.viewfinder")
                    }
                    Button {
                        model.hapticSelection()
                        Task {
                            if await model.connectManual() {
                                dismiss()
                            }
                        }
                    } label: {
                        Label("Connect", systemImage: "link")
                    }
                }

                if model.authEndpoint != nil {
                    Section("Pair") {
                        TextField("Pairing Code", text: $model.pairingCode)
                            .keyboardType(.numberPad)
                        Button {
                            model.hapticSelection()
                            isScanning = true
                        } label: {
                            Label("Scan QR Code", systemImage: "qrcode.viewfinder")
                        }
                        Button {
                            model.hapticSelection()
                            Task {
                                if await model.pair() {
                                    dismiss()
                                }
                            }
                        } label: {
                            Label("Pair", systemImage: "checkmark.seal")
                        }
                        Button {
                            model.hapticSelection()
                            Task {
                                if await model.useToken() {
                                    dismiss()
                                }
                            }
                        } label: {
                            Label("Use Token", systemImage: "key")
                        }
                    }
                }

                if !model.status.isEmpty {
                    Section {
                        Text(model.status)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            .navigationTitle("Connect")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") {
                        model.hapticSelection()
                        dismiss()
                    }
                }
            }
        }
        .sheet(isPresented: $isScanning) {
            QRCodeScannerSheet(model: model) {
                dismiss()
            }
        }
        .presentationDetents([.medium, .large])
    }
}

private struct SimulatorSearchDock: View {
    @Bindable var model: AppModel
    @Binding var text: String
    @Binding var isExpanded: Bool
    let showsSidebarActions: Bool
    let onSettings: () -> Void
    let onConnect: () -> Void
    let onCreateSimulator: () -> Void
    @FocusState private var isFocused: Bool

    private var isSearchBarVisible: Bool {
        isExpanded || !text.isEmpty
    }

    private var visibleActionButtonCount: Int {
        showsSidebarActions ? 3 : 1
    }

    var body: some View {
        GeometryReader { proxy in
            let reservedWidth = CGFloat(visibleActionButtonCount) * 56 + 32
            let width = isSearchBarVisible
                ? max(48, proxy.size.width - reservedWidth)
                : 48.0

            HStack(spacing: 8) {
                searchControl(width: width)

                if showsSidebarActions {
                    dockIconButton("Settings", systemImage: "gearshape", action: onSettings)
                    dockIconButton("Connect Server", systemImage: "personalhotspot", action: onConnect)
                }

                dockIconButton(
                    "New Simulator",
                    systemImage: "plus",
                    isDisabled: model.endpoint == nil || model.authEndpoint != nil,
                    action: {
                        model.hapticSelection()
                        onCreateSimulator()
                    }
                )
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .trailing)
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
            .zIndex(1)
        }
        .frame(height: 64)
        .animation(.snappy(duration: 0.24), value: isSearchBarVisible)
        .onChange(of: isExpanded) { _, expanded in
            guard expanded else { return }
            Task { @MainActor in
                try? await Task.sleep(nanoseconds: 80_000_000)
                isFocused = true
            }
        }
    }

    @ViewBuilder
    private func searchControl(width: CGFloat) -> some View {
        HStack(spacing: 8) {
            if isSearchBarVisible {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(.secondary)

                TextField("Search Simulators", text: $text)
                    .focused($isFocused)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .submitLabel(.search)
                    .transition(.opacity)

                Button {
                    model.hapticSelection()
                    if text.isEmpty {
                        isFocused = false
                        withAnimation(.snappy(duration: 0.24)) {
                            isExpanded = false
                        }
                    } else {
                        text = ""
                    }
                } label: {
                    Label(text.isEmpty ? "Close Search" : "Clear", systemImage: "xmark.circle.fill")
                }
                .labelStyle(.iconOnly)
                .foregroundStyle(.secondary)
                .buttonStyle(.plain)
                .transition(.opacity)
            } else {
                Button {
                    model.hapticSelection()
                    withAnimation(.snappy(duration: 0.24)) {
                        isExpanded = true
                    }
                } label: {
                    Label("Search", systemImage: "magnifyingglass")
                        .labelStyle(.iconOnly)
                        .foregroundStyle(.primary)
                        .frame(width: 48, height: 48)
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, isSearchBarVisible ? 14 : 0)
        .frame(width: width, height: 48)
        .contentShape(Capsule())
        .modifier(GlassCapsuleModifier(interactive: true))
    }

    private func dockIconButton(
        _ title: String,
        systemImage: String,
        isDisabled: Bool = false,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            Label(title, systemImage: systemImage)
                .labelStyle(.iconOnly)
                .foregroundStyle(.primary)
                .frame(width: 48, height: 48)
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .modifier(GlassCircleModifier(interactive: true))
        .disabled(isDisabled)
        .accessibilityLabel(title)
    }
}

private struct GlassCapsuleModifier: ViewModifier {
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

private struct GlassCircleModifier: ViewModifier {
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

private struct GlassRoundedRectangleModifier: ViewModifier {
    let cornerRadius: CGFloat
    let interactive: Bool

    func body(content: Content) -> some View {
        if #available(iOS 26.0, *) {
            if interactive {
                content.glassEffect(.regular.interactive(), in: .rect(cornerRadius: cornerRadius))
            } else {
                content.glassEffect(.regular, in: .rect(cornerRadius: cornerRadius))
            }
        } else {
            content.background(
                .ultraThinMaterial,
                in: RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
            )
        }
    }
}

private struct QRCodeScannerSheet: View {
    @Bindable var model: AppModel
    @Environment(\.dismiss) private var dismiss
    let onCompleted: () -> Void
    @State private var didScan = false

    var body: some View {
        NavigationStack {
            QRCodeScannerView { value in
                guard !didScan else { return }
                didScan = true
                dismiss()
                Task { @MainActor in
                    if await model.handleScannedPairingPayload(value) {
                        onCompleted()
                    }
                }
            }
            .ignoresSafeArea(edges: .bottom)
            .navigationTitle("Scan Pairing QR")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") {
                        model.hapticSelection()
                        dismiss()
                    }
                }
            }
        }
    }

}

private struct QRCodeScannerView: UIViewControllerRepresentable {
    let onScan: (String) -> Void

    func makeUIViewController(context: Context) -> QRCodeScannerViewController {
        QRCodeScannerViewController(onScan: onScan)
    }

    func updateUIViewController(_ uiViewController: QRCodeScannerViewController, context: Context) {}
}

private final class QRCodeScannerViewController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
    private let onScan: (String) -> Void
    private let session = AVCaptureSession()
    private var previewLayer: AVCaptureVideoPreviewLayer?
    private var didScan = false
    private let messageLabel = UILabel()

    init(onScan: @escaping (String) -> Void) {
        self.onScan = onScan
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        return nil
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        configureMessageLabel()
        requestCameraAccess()
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        previewLayer?.frame = view.bounds
        messageLabel.frame = CGRect(
            x: 24,
            y: view.safeAreaInsets.top + 24,
            width: view.bounds.width - 48,
            height: 64
        )
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        stopSession()
    }

    func metadataOutput(
        _ output: AVCaptureMetadataOutput,
        didOutput metadataObjects: [AVMetadataObject],
        from connection: AVCaptureConnection
    ) {
        guard !didScan,
              let object = metadataObjects.compactMap({ $0 as? AVMetadataMachineReadableCodeObject }).first(where: { $0.type == .qr }),
              let value = object.stringValue?.nilIfBlank else {
            return
        }
        didScan = true
        stopSession()
        onScan(value)
    }

    private func configureMessageLabel() {
        messageLabel.text = "Scan the QR from simdeck pair"
        messageLabel.textAlignment = .center
        messageLabel.textColor = .white
        messageLabel.font = .preferredFont(forTextStyle: .headline)
        messageLabel.backgroundColor = UIColor.black.withAlphaComponent(0.45)
        messageLabel.layer.cornerRadius = 14
        messageLabel.clipsToBounds = true
        view.addSubview(messageLabel)
    }

    private func requestCameraAccess() {
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            configureSession()
        case .notDetermined:
            AVCaptureDevice.requestAccess(for: .video) { [weak self] granted in
                DispatchQueue.main.async {
                    granted ? self?.configureSession() : self?.showCameraDenied()
                }
            }
        default:
            showCameraDenied()
        }
    }

    private func configureSession() {
        guard previewLayer == nil else { return }
        guard let device = AVCaptureDevice.default(for: .video),
              let input = try? AVCaptureDeviceInput(device: device),
              session.canAddInput(input) else {
            showScannerUnavailable()
            return
        }
        session.addInput(input)

        let output = AVCaptureMetadataOutput()
        guard session.canAddOutput(output) else {
            showScannerUnavailable()
            return
        }
        session.addOutput(output)
        output.setMetadataObjectsDelegate(self, queue: .main)
        output.metadataObjectTypes = [.qr]

        let previewLayer = AVCaptureVideoPreviewLayer(session: session)
        previewLayer.videoGravity = .resizeAspectFill
        previewLayer.frame = view.bounds
        view.layer.insertSublayer(previewLayer, at: 0)
        self.previewLayer = previewLayer

        DispatchQueue.global(qos: .userInitiated).async { [session] in
            session.startRunning()
        }
    }

    private func stopSession() {
        guard session.isRunning else { return }
        DispatchQueue.global(qos: .userInitiated).async { [session] in
            session.stopRunning()
        }
    }

    private func showCameraDenied() {
        messageLabel.text = "Camera access is needed to scan pairing QR codes."
    }

    private func showScannerUnavailable() {
        messageLabel.text = "QR scanning is unavailable on this device."
    }
}

private struct EndpointRow: View {
    let endpoint: SimDeckEndpoint

    var body: some View {
        Label {
            VStack(alignment: .leading, spacing: 2) {
                Text(endpoint.displayName)
                    .lineLimit(1)
                Text(endpoint.listSubtitle)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        } icon: {
            Image(systemName: endpoint.source.systemImage)
                .foregroundStyle(endpoint.requiresPairing ? .orange : .blue)
        }
    }
}

struct SimulatorRow: View {
    let simulator: SimulatorMetadata
    var isBusy = false

    var body: some View {
        Label {
            VStack(alignment: .leading, spacing: 2) {
                Text(simulator.name)
                    .lineLimit(1)
                if !simulator.subtitle.isEmpty {
                    Text(simulator.subtitle)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
        } icon: {
            if isBusy {
                ProgressView()
                    .controlSize(.small)
            } else {
                Image(systemName: simulator.systemImage)
                    .foregroundStyle(simulator.isBooted ? .green : .secondary)
            }
        }
    }
}

struct SimulatorLifecycleMenuItems: View {
    @Bindable var model: AppModel
    let simulator: SimulatorMetadata

    var body: some View {
        if model.isSimulatorLifecycleBusy(simulator) {
            Button {} label: {
                Label(simulator.isBooted ? "Stopping" : "Starting", systemImage: "hourglass")
            }
            .disabled(true)
        } else if simulator.isBooted {
            Button(role: .destructive) {
                Task { await model.stopSimulator(simulator) }
            } label: {
                Label("Stop", systemImage: "stop.circle")
            }
            .disabled(model.endpoint == nil || model.simulatorLifecycleID == simulator.udid)
        } else {
            Button {
                Task { await model.startSimulator(simulator) }
            } label: {
                Label("Start", systemImage: "play.circle")
            }
            .disabled(model.endpoint == nil || model.simulatorLifecycleID == simulator.udid)
        }
    }
}
