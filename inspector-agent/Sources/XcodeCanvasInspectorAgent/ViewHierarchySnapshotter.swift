import UIKit

final class ViewHierarchySnapshotter {
    func snapshot(includeHidden: Bool, maxDepth: Int?) -> InspectorHierarchySnapshot {
        let roots = windows()
            .filter { includeHidden || (!$0.isHidden && $0.alpha > 0) }
            .map { node(for: $0, includeHidden: includeHidden, maxDepth: maxDepth, depth: 0) }

        return InspectorHierarchySnapshot(
            protocolVersion: InspectorProtocol.version,
            capturedAt: ISO8601DateFormatter().string(from: Date()),
            processIdentifier: ProcessInfo.processInfo.processIdentifier,
            bundleIdentifier: Bundle.main.bundleIdentifier,
            displayScale: Double(UIScreen.main.scale),
            coordinateSpace: "screen-points",
            roots: roots
        )
    }

    func view(withId id: String) -> UIView? {
        for window in windows() {
            if let view = findView(withId: id, in: window) {
                return view
            }
        }
        return nil
    }

    func hitTest(screenPoint: CGPoint, windowId: String?) -> UIView? {
        let candidateWindows: [UIWindow]
        if let windowId, let window = view(withId: windowId) as? UIWindow {
            candidateWindows = [window]
        } else {
            candidateWindows = windows().reversed()
        }

        for window in candidateWindows where !window.isHidden && window.alpha > 0 {
            let point = window.convert(screenPoint, from: nil)
            if window.point(inside: point, with: nil), let hit = window.hitTest(point, with: nil) {
                return hit
            }
        }

        return nil
    }

    func node(for view: UIView, includeHidden: Bool, maxDepth: Int?, depth: Int) -> InspectorViewNode {
        let children: [InspectorViewNode]
        if let maxDepth, depth >= maxDepth {
            children = []
        } else {
            children = view.subviews
                .filter { includeHidden || (!$0.isHidden && $0.alpha > 0) }
                .map { node(for: $0, includeHidden: includeHidden, maxDepth: maxDepth, depth: depth + 1) }
        }

        let className = NSStringFromClass(type(of: view))
        return InspectorViewNode(
            id: Self.id(for: view),
            className: className,
            moduleName: Self.moduleName(for: className),
            debugDescription: String(describing: view),
            frame: InspectorRect(view.frame),
            bounds: InspectorRect(view.bounds),
            frameInScreen: InspectorRect(frameInScreen(for: view)),
            center: InspectorPoint(view.center),
            transform: String(describing: view.transform),
            alpha: Double(view.alpha),
            isHidden: view.isHidden,
            isOpaque: view.isOpaque,
            clipsToBounds: view.clipsToBounds,
            isUserInteractionEnabled: view.isUserInteractionEnabled,
            backgroundColor: InspectorColor(view.backgroundColor),
            tintColor: InspectorColor(view.tintColor),
            accessibility: accessibilityInfo(for: view),
            swiftUI: swiftUIInfo(for: view, className: className),
            viewController: viewControllerInfo(for: view),
            text: textValue(for: view),
            placeholder: placeholderValue(for: view),
            imageName: imageName(for: view),
            scroll: scrollInfo(for: view),
            control: controlInfo(for: view),
            children: children
        )
    }

    static func id(for view: UIView) -> String {
        let address = UInt(bitPattern: Unmanaged.passUnretained(view).toOpaque())
        return String(format: "view:0x%llx", UInt64(address))
    }

    static func id(for viewController: UIViewController) -> String {
        let address = UInt(bitPattern: Unmanaged.passUnretained(viewController).toOpaque())
        return String(format: "vc:0x%llx", UInt64(address))
    }

    private func windows() -> [UIWindow] {
        let sceneWindows: [UIWindow]
        if #available(iOS 13.0, *) {
            sceneWindows = UIApplication.shared.connectedScenes
                .compactMap { $0 as? UIWindowScene }
                .flatMap(\.windows)
        } else {
            sceneWindows = []
        }

        if !sceneWindows.isEmpty {
            return sceneWindows.sorted { lhs, rhs in
                if lhs.windowLevel == rhs.windowLevel {
                    return String(describing: lhs) < String(describing: rhs)
                }
                return lhs.windowLevel.rawValue < rhs.windowLevel.rawValue
            }
        }

        return UIApplication.shared.windows
    }

    private func findView(withId id: String, in root: UIView) -> UIView? {
        if Self.id(for: root) == id || root.xcwInspectorTagPayload?.id == id {
            return root
        }

        for child in root.subviews {
            if let match = findView(withId: id, in: child) {
                return match
            }
        }

        return nil
    }

    private func frameInScreen(for view: UIView) -> CGRect {
        view.convert(view.bounds, to: nil)
    }

    private func accessibilityInfo(for view: UIView) -> InspectorAccessibilityInfo {
        InspectorAccessibilityInfo(
            identifier: view.accessibilityIdentifier,
            label: view.accessibilityLabel,
            value: view.accessibilityValue,
            hint: view.accessibilityHint,
            traits: UInt64(view.accessibilityTraits.rawValue),
            isElement: view.isAccessibilityElement
        )
    }

    private func swiftUIInfo(for view: UIView, className: String) -> InspectorSwiftUIInfo? {
        let payload = view.xcwInspectorTagPayload
        let isHost = className.contains("SwiftUI")
            || className.contains("UIHosting")
            || String(describing: type(of: view.next)).contains("UIHosting")
        let isProbe = view is XcodeCanvasInspectorProbeUIView || payload != nil

        guard isHost || isProbe else {
            return nil
        }

        return InspectorSwiftUIInfo(
            isHost: isHost,
            isProbe: isProbe,
            tag: payload?.name,
            tagId: payload?.id,
            metadata: payload?.metadata ?? [:]
        )
    }

    private func viewControllerInfo(for view: UIView) -> InspectorViewControllerInfo? {
        guard let viewController = nearestViewController(for: view) else {
            return nil
        }

        return InspectorViewControllerInfo(
            id: Self.id(for: viewController),
            className: NSStringFromClass(type(of: viewController)),
            title: viewController.title
        )
    }

    private func nearestViewController(for view: UIView) -> UIViewController? {
        var responder: UIResponder? = view.next
        while let current = responder {
            if let viewController = current as? UIViewController {
                return viewController
            }
            responder = current.next
        }
        return nil
    }

    private func textValue(for view: UIView) -> String? {
        switch view {
        case let label as UILabel:
            return label.text
        case let button as UIButton:
            return button.title(for: button.state)
        case let textField as UITextField:
            return textField.text
        case let textView as UITextView:
            return textView.text
        case let segmented as UISegmentedControl:
            guard segmented.selectedSegmentIndex >= 0 else {
                return nil
            }
            return segmented.titleForSegment(at: segmented.selectedSegmentIndex)
        default:
            return nil
        }
    }

    private func placeholderValue(for view: UIView) -> String? {
        switch view {
        case let textField as UITextField:
            return textField.placeholder
        default:
            return nil
        }
    }

    private func imageName(for view: UIView) -> String? {
        switch view {
        case let imageView as UIImageView:
            return imageView.image?.accessibilityIdentifier
        case let button as UIButton:
            return button.image(for: button.state)?.accessibilityIdentifier
        default:
            return nil
        }
    }

    private func scrollInfo(for view: UIView) -> InspectorScrollInfo? {
        guard let scrollView = view as? UIScrollView else {
            return nil
        }

        return InspectorScrollInfo(
            contentOffset: InspectorPoint(scrollView.contentOffset),
            contentSize: InspectorSize(scrollView.contentSize),
            adjustedContentInset: InspectorInsets(scrollView.adjustedContentInset),
            isScrollEnabled: scrollView.isScrollEnabled
        )
    }

    private func controlInfo(for view: UIView) -> InspectorControlInfo? {
        guard let control = view as? UIControl else {
            return nil
        }

        return InspectorControlInfo(
            controlState: control.state.rawValue,
            controlEvents: control.allControlEvents.rawValue,
            isSelected: control.isSelected,
            isHighlighted: control.isHighlighted,
            actions: ViewInteractionPerformer.actions(for: view)
        )
    }

    private static func moduleName(for className: String) -> String? {
        className.split(separator: ".").first.map(String.init)
    }
}
