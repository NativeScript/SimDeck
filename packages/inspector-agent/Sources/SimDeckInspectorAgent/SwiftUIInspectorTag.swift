#if canImport(SwiftUI)
import SwiftUI
import UIKit

@available(iOS 13.0, *)
public extension View {
    func simDeckInspectorTag(
        _ name: String,
        id: String? = nil,
        metadata: [String: String] = [:]
    ) -> some View {
        background(
            SimDeckInspectorTagRepresentable(
                payload: SimDeckInspectorTagPayload(
                    id: id,
                    name: name,
                    metadata: metadata
                )
            )
        )
    }
}

@available(iOS 13.0, *)
private struct SimDeckInspectorTagRepresentable: UIViewRepresentable {
    var payload: SimDeckInspectorTagPayload

    func makeUIView(context: Context) -> SimDeckInspectorProbeUIView {
        SimDeckInspectorProbeUIView(payload: payload)
    }

    func updateUIView(_ uiView: SimDeckInspectorProbeUIView, context: Context) {
        uiView.payload = payload
    }
}
#endif
