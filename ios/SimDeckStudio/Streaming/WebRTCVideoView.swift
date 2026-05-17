import CoreImage
import SwiftUI
import UIKit
@preconcurrency import WebRTC

struct WebRTCVideoView: UIViewRepresentable {
    let client: WebRTCClient?
    let onVideoSize: (CGSize) -> Void
    let onFrameRendered: () -> Void
    let onFrameSnapshot: (UIImage) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(
            client: client,
            onVideoSize: onVideoSize,
            onFrameRendered: onFrameRendered,
            onFrameSnapshot: onFrameSnapshot
        )
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
        context.coordinator.onFrameRendered = onFrameRendered
        context.coordinator.onFrameSnapshot = onFrameSnapshot
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
        var onFrameRendered: () -> Void
        var onFrameSnapshot: (UIImage) -> Void
        private var lastReportedSize = CGSize.zero
        private var hasReportedRenderedFrame = false
        private var lastSnapshotAt = Date.distantPast
        private static let snapshotInterval: TimeInterval = 0.75
        private static let snapshotContext = CIContext(options: [.priorityRequestLow: true])

        init(
            client: WebRTCClient?,
            onVideoSize: @escaping (CGSize) -> Void,
            onFrameRendered: @escaping () -> Void,
            onFrameSnapshot: @escaping (UIImage) -> Void
        ) {
            self.client = client
            self.onVideoSize = onVideoSize
            self.onFrameRendered = onFrameRendered
            self.onFrameSnapshot = onFrameSnapshot
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
            guard let frame else { return }
            if !hasReportedRenderedFrame {
                hasReportedRenderedFrame = true
                Task { @MainActor in
                    onFrameRendered()
                }
            }
            captureSnapshotIfNeeded(from: frame)
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

        private func captureSnapshotIfNeeded(from frame: RTCVideoFrame) {
            let now = Date()
            guard now.timeIntervalSince(lastSnapshotAt) >= Self.snapshotInterval else { return }
            lastSnapshotAt = now

            guard let image = Self.image(from: frame) else { return }
            Task { @MainActor in
                onFrameSnapshot(image)
            }
        }

        private static func image(from frame: RTCVideoFrame) -> UIImage? {
            guard let buffer = frame.buffer as? RTCCVPixelBuffer else { return nil }
            var image = CIImage(cvPixelBuffer: buffer.pixelBuffer)
            if buffer.requiresCropping() {
                image = image.cropped(to: CGRect(
                    x: CGFloat(buffer.cropX),
                    y: CGFloat(buffer.cropY),
                    width: CGFloat(buffer.cropWidth),
                    height: CGFloat(buffer.cropHeight)
                ))
            }
            guard let cgImage = snapshotContext.createCGImage(image, from: image.extent) else { return nil }
            return UIImage(cgImage: cgImage, scale: 1, orientation: frame.uiImageOrientation)
        }
    }
}

private extension RTCVideoRotation {
    var uiImageOrientation: UIImage.Orientation {
        switch rawValue {
        case 90:
            return .right
        case 180:
            return .down
        case 270:
            return .left
        default:
            return .up
        }
    }
}

private extension RTCVideoFrame {
    var uiImageOrientation: UIImage.Orientation {
        rotation.uiImageOrientation
    }
}
