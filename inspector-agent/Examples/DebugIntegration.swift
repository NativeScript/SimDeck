#if DEBUG
import SwiftUI
import XcodeCanvasInspectorAgent

struct InspectorAgentBootstrap {
    static func start() {
        try? XcodeCanvasInspectorAgent.shared.start()
    }
}

struct TaggedSwiftUIExample: View {
    var body: some View {
        VStack {
            Text("Checkout")
                .xcwInspectorTag("checkout-title", id: "checkout.title")

            Button("Pay") {}
                .xcwInspectorTag("pay-button", id: "checkout.pay")
        }
        .xcwInspectorTag("checkout-screen", id: "checkout.screen")
    }
}
#endif
