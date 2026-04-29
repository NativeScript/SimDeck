#if DEBUG
import SwiftUI
import SimDeckInspectorAgent

struct InspectorAgentBootstrap {
    static func start() {
        try? SimDeckInspectorAgent.shared.start()
    }
}

struct TaggedSwiftUIExample: View {
    var body: some View {
        VStack {
            Text("Checkout")
                .simDeckInspectorTag("checkout-title", id: "checkout.title")

            Button("Pay") {}
                .simDeckInspectorTag("pay-button", id: "checkout.pay")
        }
        .simDeckInspectorTag("checkout-screen", id: "checkout.screen")
        .simDeckPublishSwiftUIViewTree("TaggedSwiftUIExample", id: "checkout.screen")
    }
}
#endif
