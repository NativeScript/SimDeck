#if canImport(SwiftUI)
import SwiftUI
import UIKit

@available(iOS 13.0, *)
public extension View {
    func simDeckPublishSwiftUIViewTree(
        _ name: String? = nil,
        id: String? = nil,
        metadata: [String: String] = [:],
        maxDepth: Int = 80
    ) -> some View {
        modifier(
            SimDeckSwiftUIViewTreePublisher(
                rootView: self,
                name: name,
                id: id,
                metadata: metadata,
                maxDepth: maxDepth
            )
        )
    }
}

@available(iOS 13.0, *)
public extension SimDeckInspectorAgent {
    func publishSwiftUIViewTree<Root: View>(
        _ rootView: Root,
        name: String? = nil,
        id: String? = nil,
        metadata: [String: String] = [:],
        maxDepth: Int = 80
    ) throws {
        let snapshot = SwiftUIViewTreeSnapshotter().snapshot(
            rootView,
            name: name,
            id: id,
            metadata: metadata,
            maxDepth: maxDepth
        )
        let data = try JSONEncoder.simDeckInspector.encode(snapshot)
        guard let json = String(data: data, encoding: .utf8) else {
            throw InspectorFailure.actionFailed("Unable to encode SwiftUI hierarchy snapshot.")
        }
        try publishHierarchySnapshot(source: "swiftui", snapshotJSON: json)
    }
}

@available(iOS 13.0, *)
private struct SimDeckSwiftUIViewTreePublisher<Root: View>: ViewModifier {
    var rootView: Root
    var name: String?
    var id: String?
    var metadata: [String: String]
    var maxDepth: Int

    func body(content: Content) -> some View {
        content.background(
            SimDeckSwiftUIViewTreePublisherRepresentable(
                rootView: rootView,
                name: name,
                id: id,
                metadata: metadata,
                maxDepth: maxDepth
            )
            .frame(width: 0, height: 0)
            .allowsHitTesting(false)
        )
    }
}

@available(iOS 13.0, *)
private struct SimDeckSwiftUIViewTreePublisherRepresentable<Root: View>: UIViewRepresentable {
    var rootView: Root
    var name: String?
    var id: String?
    var metadata: [String: String]
    var maxDepth: Int

    func makeUIView(context: Context) -> UIView {
        let view = UIView(frame: .zero)
        view.isUserInteractionEnabled = false
        view.isAccessibilityElement = false
        view.backgroundColor = .clear
        return view
    }

    func updateUIView(_ uiView: UIView, context: Context) {
        try? SimDeckInspectorAgent.shared.publishSwiftUIViewTree(
            rootView,
            name: name,
            id: id,
            metadata: metadata,
            maxDepth: maxDepth
        )
    }
}

private struct InspectorSwiftUIViewHierarchySnapshot: Codable, Equatable {
    var protocolVersion: String
    var capturedAt: String
    var processIdentifier: Int32
    var bundleIdentifier: String?
    var displayScale: Double
    var coordinateSpace: String
    var source: String
    var roots: [InspectorSwiftUIViewNode]
}

private struct InspectorSwiftUIViewNode: Codable, Equatable {
    var id: String
    var type: String
    var title: String
    var role: String
    var AXLabel: String?
    var AXIdentifier: String?
    var AXUniqueId: String
    var source: String
    var swiftUI: InspectorSwiftUIInfo
    var children: [InspectorSwiftUIViewNode]
}

@available(iOS 13.0, *)
private protocol SimDeckSwiftUIExpandable {
    func simDeckSwiftUIChildren(
        parentPath: String,
        depth: Int,
        maxDepth: Int
    ) -> [InspectorSwiftUIViewNode]
}

@available(iOS 13.0, *)
extension ForEach: SimDeckSwiftUIExpandable where Content: View {
    fileprivate func simDeckSwiftUIChildren(
        parentPath: String,
        depth: Int,
        maxDepth: Int
    ) -> [InspectorSwiftUIViewNode] {
        data.enumerated().map { index, element in
            SwiftUIViewTreeSnapshotter.makeViewNode(
                content(element),
                path: "\(parentPath).row-\(index)",
                depth: depth,
                maxDepth: maxDepth
            )
        }
    }
}

@available(iOS 13.0, *)
private struct SwiftUIViewTreeSnapshotter {
    func snapshot<Root: View>(
        _ rootView: Root,
        name: String?,
        id: String?,
        metadata: [String: String],
        maxDepth: Int
    ) -> InspectorSwiftUIViewHierarchySnapshot {
        let root = Self.makeViewNode(
            rootView,
            explicitName: name,
            explicitId: id,
            metadata: metadata,
            path: "root",
            depth: 0,
            maxDepth: max(0, maxDepth)
        )

        return InspectorSwiftUIViewHierarchySnapshot(
            protocolVersion: InspectorProtocol.version,
            capturedAt: ISO8601DateFormatter().string(from: Date()),
            processIdentifier: ProcessInfo.processInfo.processIdentifier,
            bundleIdentifier: Bundle.main.bundleIdentifier,
            displayScale: Double(UIScreen.main.scale),
            coordinateSpace: "screen-points",
            source: "swiftui",
            roots: [root]
        )
    }

    fileprivate static func makeViewNode<V: View>(
        _ view: V,
        explicitName: String? = nil,
        explicitId: String? = nil,
        metadata: [String: String] = [:],
        path: String,
        depth: Int,
        maxDepth: Int
    ) -> InspectorSwiftUIViewNode {
        if let collapsed = modifiedContentNode(
            view,
            explicitName: explicitName,
            explicitId: explicitId,
            metadata: metadata,
            path: path,
            depth: depth,
            maxDepth: maxDepth
        ) {
            return collapsed
        }

        let rawType = String(reflecting: V.self)
        let displayType = displayTypeName(rawType)
        let children: [InspectorSwiftUIViewNode]
        if depth >= maxDepth {
            children = []
        } else if let expandable = view as? SimDeckSwiftUIExpandable {
            children = expandable.simDeckSwiftUIChildren(
                parentPath: path,
                depth: depth + 1,
                maxDepth: maxDepth
            )
        } else if V.Body.self != Never.self {
            children = [
                makeAnyNode(
                    view.body,
                    label: "body",
                    path: "\(path).body",
                    depth: depth + 1,
                    maxDepth: maxDepth
                ),
            ].compactMap { $0 }
        } else {
            children = reflectedChildren(
                of: view,
                parentPath: path,
                depth: depth + 1,
                maxDepth: maxDepth
            )
        }

        let directText = extractedText(from: view)
        let semanticText = directText ?? childSemanticText(children)
        let title = clean(explicitName) ?? semanticText ?? displayType
        let nodeId = "swiftui:\(clean(explicitId) ?? path)"

        return InspectorSwiftUIViewNode(
            id: nodeId,
            type: displayType,
            title: title,
            role: "SwiftUI View",
            AXLabel: semanticText,
            AXIdentifier: clean(explicitId),
            AXUniqueId: nodeId,
            source: "swiftui",
            swiftUI: InspectorSwiftUIInfo(
                isHost: false,
                isProbe: false,
                tag: clean(explicitName),
                tagId: clean(explicitId),
                metadata: metadata,
                isViewTreeNode: true,
                valueType: rawType,
                bodyType: String(reflecting: V.Body.self),
                path: path,
                modifiers: nil
            ),
            children: children
        )
    }

    private static func modifiedContentNode<V: View>(
        _ view: V,
        explicitName: String?,
        explicitId: String?,
        metadata: [String: String],
        path: String,
        depth: Int,
        maxDepth: Int
    ) -> InspectorSwiftUIViewNode? {
        guard String(reflecting: V.self).contains("ModifiedContent<") else {
            return nil
        }

        let mirror = Mirror(reflecting: view)
        let content = mirror.children.first { $0.label == "content" }?.value
        let modifier = mirror.children.first { $0.label == "modifier" }?.value
        guard var node = content.flatMap({
            makeAnyNode(
                $0,
                label: "content",
                path: path,
                depth: depth,
                maxDepth: maxDepth
            )
        }) else {
            return nil
        }

        if let explicitName = clean(explicitName) {
            node.title = explicitName
            node.swiftUI.tag = explicitName
        }
        if let explicitId = clean(explicitId) {
            node.id = "swiftui:\(explicitId)"
            node.AXIdentifier = explicitId
            node.AXUniqueId = node.id
            node.swiftUI.tagId = explicitId
        }
        if !metadata.isEmpty {
            node.swiftUI.metadata = metadata
        }
        if let modifier {
            node.swiftUI.modifiers = (node.swiftUI.modifiers ?? []) + [
                displayTypeName(String(reflecting: type(of: modifier))),
            ]
        }
        return node
    }

    private static func reflectedChildren(
        of value: Any,
        parentPath: String,
        depth: Int,
        maxDepth: Int
    ) -> [InspectorSwiftUIViewNode] {
        Mirror(reflecting: value).children.enumerated().flatMap { index, child in
            let label = child.label ?? String(index)
            return nodesFromReflectedValue(
                child.value,
                label: label,
                path: "\(parentPath).\(pathComponent(label, fallback: index))",
                depth: depth,
                maxDepth: maxDepth
            )
        }
    }

    private static func nodesFromReflectedValue(
        _ value: Any,
        label: String,
        path: String,
        depth: Int,
        maxDepth: Int
    ) -> [InspectorSwiftUIViewNode] {
        if label == "action" || label == "modifier" || label == "root" {
            return []
        }

        if let node = makeAnyNode(value, label: label, path: path, depth: depth, maxDepth: maxDepth) {
            return [node]
        }

        let mirror = Mirror(reflecting: value)
        if mirror.displayStyle == .tuple
            || mirror.displayStyle == .optional
            || mirror.displayStyle == .enum
            || label == "_tree"
        {
            return mirror.children.enumerated().flatMap { index, child in
                nodesFromReflectedValue(
                    child.value,
                    label: child.label ?? String(index),
                    path: "\(path).\(pathComponent(child.label, fallback: index))",
                    depth: depth,
                    maxDepth: maxDepth
                )
            }
        }

        let rawType = String(reflecting: type(of: value))
        if rawType.contains("TupleView<") || rawType.contains("Tree<") {
            return mirror.children.enumerated().flatMap { index, child in
                nodesFromReflectedValue(
                    child.value,
                    label: child.label ?? String(index),
                    path: "\(path).\(pathComponent(child.label, fallback: index))",
                    depth: depth,
                    maxDepth: maxDepth
                )
            }
        }

        if label == "content" || label == "label" || label == "value" || label.hasPrefix(".") {
            return mirror.children.enumerated().flatMap { index, child in
                nodesFromReflectedValue(
                    child.value,
                    label: child.label ?? String(index),
                    path: "\(path).\(pathComponent(child.label, fallback: index))",
                    depth: depth,
                    maxDepth: maxDepth
                )
            }
        }

        return []
    }

    private static func makeAnyNode(
        _ value: Any,
        label: String?,
        path: String,
        depth: Int,
        maxDepth: Int
    ) -> InspectorSwiftUIViewNode? {
        guard let view = value as? any View else {
            return nil
        }
        return view.simDeckSwiftUIViewTreeNode(
            path: path,
            depth: depth,
            maxDepth: maxDepth
        )
    }

    private static func childSemanticText(_ children: [InspectorSwiftUIViewNode]) -> String? {
        let labels = children.compactMap { clean($0.AXLabel ?? $0.title) }
        guard labels.count == 1 else {
            return nil
        }
        return labels.first
    }

    private static func extractedText(from value: Any) -> String? {
        guard displayTypeName(String(reflecting: type(of: value))) == "Text" else {
            return nil
        }
        return firstStringValue(in: value, preferredLabels: ["verbatim", "key"], depth: 0)
    }

    private static func firstStringValue(
        in value: Any,
        preferredLabels: Set<String>,
        depth: Int
    ) -> String? {
        if depth > 8 {
            return nil
        }
        let mirror = Mirror(reflecting: value)
        for child in mirror.children {
            if let label = child.label, preferredLabels.contains(label), let text = child.value as? String {
                return clean(text)
            }
        }
        for child in mirror.children {
            if let text = firstStringValue(
                in: child.value,
                preferredLabels: preferredLabels,
                depth: depth + 1
            ) {
                return text
            }
        }
        return nil
    }

    private static func pathComponent(_ label: String?, fallback: Int) -> String {
        let raw = label ?? String(fallback)
        let filtered = raw.map { character -> Character in
            character.isLetter || character.isNumber ? character : "-"
        }
        let value = String(filtered).trimmingCharacters(in: CharacterSet(charactersIn: "-"))
        return value.isEmpty ? String(fallback) : value
    }

    private static func displayTypeName(_ rawType: String) -> String {
        let base = rawType.split(separator: "<", maxSplits: 1).first.map(String.init) ?? rawType
        let name = base.split(separator: ".").last.map(String.init) ?? base
        return name.trimmingCharacters(in: CharacterSet(charactersIn: "_"))
    }

    private static func clean(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed?.isEmpty == false ? trimmed : nil
    }
}

@available(iOS 13.0, *)
private extension View {
    func simDeckSwiftUIViewTreeNode(
        path: String,
        depth: Int,
        maxDepth: Int
    ) -> InspectorSwiftUIViewNode {
        SwiftUIViewTreeSnapshotter.makeViewNode(
            self,
            path: path,
            depth: depth,
            maxDepth: maxDepth
        )
    }
}
#endif
