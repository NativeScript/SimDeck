import UIKit

public struct InspectorPoint: Codable, Equatable {
    public var x: Double
    public var y: Double
}

public struct InspectorSize: Codable, Equatable {
    public var width: Double
    public var height: Double
}

public struct InspectorRect: Codable, Equatable {
    public var x: Double
    public var y: Double
    public var width: Double
    public var height: Double
}

public struct InspectorInsets: Codable, Equatable {
    public var top: Double
    public var left: Double
    public var bottom: Double
    public var right: Double
}

public struct InspectorColor: Codable, Equatable {
    public var red: Double
    public var green: Double
    public var blue: Double
    public var alpha: Double
    public var hex: String
}

public struct InspectorViewControllerInfo: Codable, Equatable {
    public var id: String
    public var className: String
    public var title: String?
}

public struct InspectorAccessibilityInfo: Codable, Equatable {
    public var identifier: String?
    public var label: String?
    public var value: String?
    public var hint: String?
    public var traits: UInt64
    public var isElement: Bool
}

public struct InspectorSwiftUIInfo: Codable, Equatable {
    public var isHost: Bool
    public var isProbe: Bool
    public var tag: String?
    public var tagId: String?
    public var metadata: [String: String]
}

public struct InspectorScrollInfo: Codable, Equatable {
    public var contentOffset: InspectorPoint
    public var contentSize: InspectorSize
    public var adjustedContentInset: InspectorInsets
    public var isScrollEnabled: Bool
}

public struct InspectorControlInfo: Codable, Equatable {
    public var controlState: UInt
    public var controlEvents: UInt
    public var isSelected: Bool
    public var isHighlighted: Bool
    public var actions: [String]
}

public struct InspectorViewNode: Codable, Equatable {
    public var id: String
    public var className: String
    public var moduleName: String?
    public var debugDescription: String
    public var frame: InspectorRect
    public var bounds: InspectorRect
    public var frameInScreen: InspectorRect
    public var center: InspectorPoint
    public var transform: String
    public var alpha: Double
    public var isHidden: Bool
    public var isOpaque: Bool
    public var clipsToBounds: Bool
    public var isUserInteractionEnabled: Bool
    public var backgroundColor: InspectorColor?
    public var tintColor: InspectorColor?
    public var accessibility: InspectorAccessibilityInfo
    public var swiftUI: InspectorSwiftUIInfo?
    public var viewController: InspectorViewControllerInfo?
    public var text: String?
    public var placeholder: String?
    public var imageName: String?
    public var scroll: InspectorScrollInfo?
    public var control: InspectorControlInfo?
    public var children: [InspectorViewNode]
}

public struct InspectorHierarchySnapshot: Codable, Equatable {
    public var protocolVersion: String
    public var capturedAt: String
    public var processIdentifier: Int32
    public var bundleIdentifier: String?
    public var displayScale: Double
    public var coordinateSpace: String
    public var roots: [InspectorViewNode]
}

extension InspectorRect {
    init(_ rect: CGRect) {
        self.init(
            x: Double(rect.origin.x),
            y: Double(rect.origin.y),
            width: Double(rect.size.width),
            height: Double(rect.size.height)
        )
    }
}

extension InspectorPoint {
    init(_ point: CGPoint) {
        self.init(x: Double(point.x), y: Double(point.y))
    }
}

extension InspectorSize {
    init(_ size: CGSize) {
        self.init(width: Double(size.width), height: Double(size.height))
    }
}

extension InspectorInsets {
    init(_ insets: UIEdgeInsets) {
        self.init(
            top: Double(insets.top),
            left: Double(insets.left),
            bottom: Double(insets.bottom),
            right: Double(insets.right)
        )
    }
}

extension InspectorColor {
    init?(_ color: UIColor?) {
        guard let color else {
            return nil
        }

        var red: CGFloat = 0
        var green: CGFloat = 0
        var blue: CGFloat = 0
        var alpha: CGFloat = 0

        if !color.getRed(&red, green: &green, blue: &blue, alpha: &alpha) {
            guard let components = color.cgColor.components else {
                return nil
            }
            if components.count >= 2 {
                red = components[0]
                green = components[0]
                blue = components[0]
                alpha = components[1]
            } else {
                return nil
            }
        }

        let r = max(0, min(255, Int(round(red * 255))))
        let g = max(0, min(255, Int(round(green * 255))))
        let b = max(0, min(255, Int(round(blue * 255))))
        let a = max(0, min(255, Int(round(alpha * 255))))
        self.init(
            red: Double(red),
            green: Double(green),
            blue: Double(blue),
            alpha: Double(alpha),
            hex: String(format: "#%02X%02X%02X%02X", r, g, b, a)
        )
    }
}
