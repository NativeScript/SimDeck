#if canImport(SwiftUI)
import SwiftUI
import UIKit

@available(iOS 13.0, *)
public extension View {
    func xcwInspectorTag(
        _ name: String,
        id: String? = nil,
        metadata: [String: String] = [:]
    ) -> some View {
        background(
            XcodeCanvasInspectorTagRepresentable(
                payload: XcodeCanvasInspectorTagPayload(
                    id: id,
                    name: name,
                    metadata: metadata
                )
            )
        )
    }
}

@available(iOS 13.0, *)
private struct XcodeCanvasInspectorTagRepresentable: UIViewRepresentable {
    var payload: XcodeCanvasInspectorTagPayload

    func makeUIView(context: Context) -> XcodeCanvasInspectorProbeUIView {
        XcodeCanvasInspectorProbeUIView(payload: payload)
    }

    func updateUIView(_ uiView: XcodeCanvasInspectorProbeUIView, context: Context) {
        uiView.payload = payload
    }
}
#endif
