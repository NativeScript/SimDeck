import ObjectiveC
import UIKit

public struct SimDeckInspectorTagPayload: Codable, Equatable {
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
    func simDeckSetInspectorTag(id: String? = nil, name: String, metadata: [String: String] = [:]) {
        simDeckInspectorTagPayload = SimDeckInspectorTagPayload(
            id: id,
            name: name,
            metadata: metadata
        )
    }

    var simDeckInspectorTagPayload: SimDeckInspectorTagPayload? {
        get {
            objc_getAssociatedObject(self, &inspectorTagPayloadKey) as? SimDeckInspectorTagPayload
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

final class SimDeckInspectorProbeUIView: UIView {
    var payload: SimDeckInspectorTagPayload {
        didSet {
            simDeckInspectorTagPayload = payload
            accessibilityIdentifier = payload.id
            accessibilityLabel = payload.name
        }
    }

    init(payload: SimDeckInspectorTagPayload) {
        self.payload = payload
        super.init(frame: .zero)
        isUserInteractionEnabled = false
        isAccessibilityElement = false
        backgroundColor = .clear
        simDeckInspectorTagPayload = payload
        accessibilityIdentifier = payload.id
        accessibilityLabel = payload.name
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }
}
