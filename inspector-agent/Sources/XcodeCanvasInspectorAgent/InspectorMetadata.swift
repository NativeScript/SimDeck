import ObjectiveC
import UIKit

public struct XcodeCanvasInspectorTagPayload: Codable, Equatable {
    public var id: String?
    public var name: String
    public var metadata: [String: String]

    public init(id: String? = nil, name: String, metadata: [String: String] = [:]) {
        self.id = id
        self.name = name
        self.metadata = metadata
    }
}

private var inspectorTagPayloadKey: UInt8 = 0

public extension UIView {
    func xcwSetInspectorTag(id: String? = nil, name: String, metadata: [String: String] = [:]) {
        xcwInspectorTagPayload = XcodeCanvasInspectorTagPayload(
            id: id,
            name: name,
            metadata: metadata
        )
    }

    var xcwInspectorTagPayload: XcodeCanvasInspectorTagPayload? {
        get {
            objc_getAssociatedObject(self, &inspectorTagPayloadKey) as? XcodeCanvasInspectorTagPayload
        }
        set {
            objc_setAssociatedObject(
                self,
                &inspectorTagPayloadKey,
                newValue,
                .OBJC_ASSOCIATION_RETAIN_NONATOMIC
            )
        }
    }
}

final class XcodeCanvasInspectorProbeUIView: UIView {
    var payload: XcodeCanvasInspectorTagPayload {
        didSet {
            xcwInspectorTagPayload = payload
            accessibilityIdentifier = payload.id
            accessibilityLabel = payload.name
        }
    }

    init(payload: XcodeCanvasInspectorTagPayload) {
        self.payload = payload
        super.init(frame: .zero)
        isUserInteractionEnabled = false
        isAccessibilityElement = false
        backgroundColor = .clear
        xcwInspectorTagPayload = payload
        accessibilityIdentifier = payload.id
        accessibilityLabel = payload.name
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }
}
