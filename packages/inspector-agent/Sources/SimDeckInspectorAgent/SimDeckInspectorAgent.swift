import Foundation
import UIKit

public final class SimDeckInspectorAgent {
    public struct Configuration {
        public var port: UInt16
        public var portSearchLimit: UInt16
        public var bindToLocalhostOnly: Bool
        public var authToken: String?
        public var advertiseBonjour: Bool
        public var serviceName: String?

        public init(
            port: UInt16 = 47370,
            portSearchLimit: UInt16 = 32,
            bindToLocalhostOnly: Bool = true,
            authToken: String? = nil,
            advertiseBonjour: Bool = true,
            serviceName: String? = nil
        ) {
            self.port = port
            self.portSearchLimit = portSearchLimit
            self.bindToLocalhostOnly = bindToLocalhostOnly
            self.authToken = authToken
            self.advertiseBonjour = advertiseBonjour
            self.serviceName = serviceName
        }

        public static let debugDefault = Configuration()
    }

    public static let shared = SimDeckInspectorAgent()

    private let snapshotter = ViewHierarchySnapshotter()
    private let interactionPerformer = ViewInteractionPerformer()
    private var configuration = Configuration.debugDefault
    private var publishedHierarchySnapshot: PublishedHierarchySnapshot?
    private var server: InspectorTCPServer?
    private var activePort: UInt16?

    private init() {}

    @discardableResult
    public func start(configuration: Configuration = .debugDefault) throws -> UInt16 {
        if let activePort {
            return activePort
        }

        self.configuration = configuration
        let server = InspectorTCPServer(configuration: configuration) { [weak self] data, respond in
            self?.handle(data, respond: respond)
        }
        let port = try server.start()
        self.server = server
        self.activePort = port
        NSLog("SimDeckInspectorAgent listening on TCP port \(port)")
        return port
    }

    public func stop() {
        server?.stop()
        server = nil
        activePort = nil
    }

    public func snapshot(includeHidden: Bool = false, maxDepth: Int? = nil) -> InspectorHierarchySnapshot {
        dispatchPrecondition(condition: .onQueue(.main))
        return snapshotter.snapshot(includeHidden: includeHidden, maxDepth: maxDepth)
    }

    public func publishHierarchySnapshot(source: String, snapshotJSON: String) throws {
        let data = Data(snapshotJSON.utf8)
        let snapshot = try JSONDecoder.simDeckInspector.decode(JSONValue.self, from: data)
        let source = source.trimmingCharacters(in: .whitespacesAndNewlines)
        let published = PublishedHierarchySnapshot(
            source: source.isEmpty ? "app" : source,
            snapshot: snapshot,
            publishedAt: ISO8601DateFormatter().string(from: Date())
        )

        if Thread.isMainThread {
            publishedHierarchySnapshot = published
        } else {
            DispatchQueue.main.async { [weak self] in
                self?.publishedHierarchySnapshot = published
            }
        }
    }

    public func clearPublishedHierarchySnapshot(source: String? = nil) {
        let clear = { [weak self] in
            guard let self else {
                return
            }
            if let source, publishedHierarchySnapshot?.source != source {
                return
            }
            publishedHierarchySnapshot = nil
        }

        if Thread.isMainThread {
            clear()
        } else {
            DispatchQueue.main.async(execute: clear)
        }
    }

    private func handle(_ data: Data, respond: @escaping (Data) -> Void) {
        let request: InspectorRequest
        do {
            request = try JSONDecoder.simDeckInspector.decode(InspectorRequest.self, from: data)
        } catch {
            respond(InspectorProtocol.failure(id: nil, InspectorFailure.invalidRequest("Request must be a JSON object with id, method, and optional params.")))
            return
        }

        DispatchQueue.main.async {
            do {
                if let token = self.configuration.authToken, request.token != token {
                    throw InspectorFailure.unauthorized
                }
                let result = try self.dispatch(request)
                respond(try InspectorProtocol.success(id: request.id, result: result))
            } catch {
                respond(InspectorProtocol.failure(id: request.id, error))
            }
        }
    }

    private func dispatch(_ request: InspectorRequest) throws -> JSONValue {
        let params = request.params?.objectValue ?? [:]

        switch request.method {
        case "Runtime.ping":
            return .object([
                "ok": .bool(true),
                "protocolVersion": .string(InspectorProtocol.version),
            ])

        case "Inspector.getInfo":
            return try info()

        case "View.getHierarchy":
            let includeHidden = params.bool("includeHidden") ?? false
            let maxDepth = params.int("maxDepth")
            if params.string("source") != "uikit", let publishedHierarchySnapshot {
                return try enrichPublishedHierarchySnapshot(publishedHierarchySnapshot)
            }
            return try simDeckJSONValue(snapshotter.snapshot(includeHidden: includeHidden, maxDepth: maxDepth))

        case "View.get":
            let id = try requiredString("id", in: params)
            guard let view = snapshotter.view(withId: id) else {
                throw InspectorFailure.targetNotFound(id)
            }
            return try simDeckJSONValue(snapshotter.node(for: view, includeHidden: true, maxDepth: params.int("maxDepth"), depth: 0))

        case "View.hitTest":
            let point = try point(in: params)
            guard let view = snapshotter.hitTest(screenPoint: point, windowId: params.string("windowId")) else {
                return .object(["view": .null])
            }
            return .object([
                "view": try simDeckJSONValue(snapshotter.node(for: view, includeHidden: true, maxDepth: params.int("maxDepth") ?? 2, depth: 0)),
            ])

        case "View.describeAtPoint":
            let point = try point(in: params)
            guard let view = snapshotter.hitTest(screenPoint: point, windowId: params.string("windowId")) else {
                return .object(["view": .null, "ancestors": .array([])])
            }
            return try describe(view: view)

        case "View.listActions":
            let id = try requiredString("id", in: params)
            guard let view = snapshotter.view(withId: id) else {
                throw InspectorFailure.targetNotFound(id)
            }
            return .object([
                "id": .string(ViewHierarchySnapshotter.id(for: view)),
                "actions": .array(ViewInteractionPerformer.actions(for: view).map(JSONValue.string)),
            ])

        case "View.perform":
            let id = try requiredString("id", in: params)
            let action = try requiredString("action", in: params)
            guard let view = snapshotter.view(withId: id) else {
                throw InspectorFailure.targetNotFound(id)
            }
            return try interactionPerformer.perform(action: action, on: view, params: params)

        default:
            throw InspectorFailure.methodNotFound(request.method)
        }
    }

    private func info() throws -> JSONValue {
        let screen = UIScreen.main
        let bundle = Bundle.main
        return .object([
            "protocolVersion": .string(InspectorProtocol.version),
            "transport": .string("tcp+ndjson"),
            "host": .string("127.0.0.1"),
            "port": .number(Double(activePort ?? configuration.port)),
            "processIdentifier": .number(Double(ProcessInfo.processInfo.processIdentifier)),
            "bundleIdentifier": .string(bundle.bundleIdentifier ?? ""),
            "bundleName": .string(bundle.object(forInfoDictionaryKey: "CFBundleName") as? String ?? ""),
            "displayScale": .number(Double(screen.scale)),
            "screenBounds": try simDeckJSONValue(InspectorRect(screen.bounds)),
            "coordinateSpace": .string("screen-points"),
            "methods": .array(InspectorProtocol.methods.map(JSONValue.string)),
            "appHierarchy": .object([
                "source": .string(publishedHierarchySnapshot?.source ?? ""),
                "available": .bool(publishedHierarchySnapshot != nil),
                "publishedAt": .string(publishedHierarchySnapshot?.publishedAt ?? ""),
            ]),
            "swiftUI": .object([
                "automaticHostDetection": .bool(true),
                "tagModifier": .string("View.simDeckInspectorTag(_:id:metadata:)"),
            ]),
        ])
    }

    private func enrichPublishedHierarchySnapshot(_ published: PublishedHierarchySnapshot) throws -> JSONValue {
        let screen = UIScreen.main
        let bundle = Bundle.main
        var object = published.snapshot.objectValue ?? [
            "roots": published.snapshot,
        ]
        object["source"] = object["source"] ?? .string(published.source)
        object["protocolVersion"] = object["protocolVersion"] ?? .string(InspectorProtocol.version)
        object["capturedAt"] = object["capturedAt"] ?? .string(published.publishedAt)
        object["processIdentifier"] = object["processIdentifier"] ?? .number(Double(ProcessInfo.processInfo.processIdentifier))
        object["bundleIdentifier"] = object["bundleIdentifier"] ?? .string(bundle.bundleIdentifier ?? "")
        object["displayScale"] = object["displayScale"] ?? .number(Double(screen.scale))
        object["coordinateSpace"] = object["coordinateSpace"] ?? .string("screen-points")
        return .object(object)
    }

    private func describe(view: UIView) throws -> JSONValue {
        var ancestors: [JSONValue] = []
        var current: UIView? = view
        while let item = current {
            ancestors.append(try simDeckJSONValue(snapshotter.node(for: item, includeHidden: true, maxDepth: 0, depth: 0)))
            current = item.superview
        }

        return .object([
            "view": try simDeckJSONValue(snapshotter.node(for: view, includeHidden: true, maxDepth: 2, depth: 0)),
            "ancestors": .array(ancestors),
        ])
    }

    private func point(in params: [String: JSONValue]) throws -> CGPoint {
        guard let x = params.double("x"), let y = params.double("y") else {
            throw InspectorFailure.invalidRequest("Point requests require numeric params.x and params.y in screen points.")
        }
        return CGPoint(x: x, y: y)
    }

    private func requiredString(_ key: String, in params: [String: JSONValue]) throws -> String {
        guard let value = params.string(key), !value.isEmpty else {
            throw InspectorFailure.invalidRequest("Request params.\(key) must be a non-empty string.")
        }
        return value
    }
}

private struct PublishedHierarchySnapshot {
    var source: String
    var snapshot: JSONValue
    var publishedAt: String
}
