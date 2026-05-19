import SwiftUI
import UIKit

final class AppOrientationDelegate: NSObject, UIApplicationDelegate {
    static var supportedOrientations: UIInterfaceOrientationMask = .portrait

    func application(
        _ application: UIApplication,
        supportedInterfaceOrientationsFor window: UIWindow?
    ) -> UIInterfaceOrientationMask {
        Self.supportedOrientations
    }
}

enum AppOrientationPolicy {
    @MainActor
    static func apply(_ orientations: UIInterfaceOrientationMask) {
        guard AppOrientationDelegate.supportedOrientations != orientations else { return }
        AppOrientationDelegate.supportedOrientations = orientations

        guard let windowScene = UIApplication.shared.connectedScenes
            .compactMap({ $0 as? UIWindowScene })
            .first(where: { $0.activationState != .unattached }) else {
            return
        }

        if #available(iOS 16.0, *) {
            windowScene.requestGeometryUpdate(.iOS(interfaceOrientations: orientations)) { _ in }
        }
        windowScene.windows
            .compactMap(\.rootViewController)
            .forEach { $0.setNeedsUpdateOfSupportedInterfaceOrientations() }
    }
}

@main
struct SimDeckStudioApp: App {
    @UIApplicationDelegateAdaptor(AppOrientationDelegate.self) private var orientationDelegate
    @State private var model = AppModel()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            ContentView(model: model)
                .onOpenURL { url in
                    model.handle(url: url)
                }
                .onChange(of: scenePhase) { _, phase in
                    model.handleScenePhase(phase)
                }
        }
    }
}
