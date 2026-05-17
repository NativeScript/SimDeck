import SwiftUI
@preconcurrency import WebRTC

struct WebRTCVideoView: UIViewRepresentable {
    let client: WebRTCClient?
    let onVideoSize: (CGSize) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(client: client, onVideoSize: onVideoSize)
    }

    func makeUIView(context: Context) -> RTCMTLVideoView {
        let view = RTCMTLVideoView(frame: .zero)
        view.videoContentMode = .scaleAspectFit
        view.backgroundColor = .black
        context.coordinator.attach(view)
        return view
    }

    func updateUIView(_ uiView: RTCMTLVideoView, context: Context) {
        context.coordinator.onVideoSize = onVideoSize
        if context.coordinator.client !== client {
            context.coordinator.detach(uiView)
            context.coordinator.client = client
            context.coordinator.attach(uiView)
        }
    }

    static func dismantleUIView(_ uiView: RTCMTLVideoView, coordinator: Coordinator) {
        coordinator.detach(uiView)
    }

    final class Coordinator: NSObject, RTCVideoRenderer, RTCVideoViewDelegate {
        var client: WebRTCClient?
        var onVideoSize: (CGSize) -> Void
        private var lastReportedSize = CGSize.zero

        init(client: WebRTCClient?, onVideoSize: @escaping (CGSize) -> Void) {
            self.client = client
            self.onVideoSize = onVideoSize
        }

        func attach(_ view: RTCMTLVideoView) {
            view.delegate = self
            client?.attachRenderer(view)
            client?.attachRenderer(self)
        }

        func detach(_ view: RTCMTLVideoView) {
            view.delegate = nil
            client?.detachRenderer(view)
            client?.detachRenderer(self)
        }

        func setSize(_ size: CGSize) {
            reportVideoSize(size)
        }

        func renderFrame(_ frame: RTCVideoFrame?) {
            client?.recordRenderedFrame(frame)
        }

        func videoView(_ videoView: any RTCVideoRenderer, didChangeVideoSize size: CGSize) {
            reportVideoSize(size)
        }

        private func reportVideoSize(_ size: CGSize) {
            guard size != lastReportedSize else { return }
            lastReportedSize = size
            Task { @MainActor in
                onVideoSize(size)
            }
        }
    }
}
