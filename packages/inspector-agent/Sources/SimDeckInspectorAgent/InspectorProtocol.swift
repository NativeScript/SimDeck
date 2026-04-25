import Foundation

struct InspectorRequest: Decodable {
    let id: JSONValue?
    let method: String
    let params: JSONValue?
    let token: String?
}

struct InspectorErrorResponse: Encodable {
    let code: Int
    let message: String
    let data: JSONValue?
}

enum InspectorFailure: Error {
    case invalidRequest(String)
    case unauthorized
    case methodNotFound(String)
    case targetNotFound(String)
    case unsupportedAction(String)
    case actionFailed(String)

    var code: Int {
        switch self {
        case .invalidRequest:
            return -32600
        case .methodNotFound:
            return -32601
        case .targetNotFound:
            return -32004
        case .unsupportedAction:
            return -32010
        case .actionFailed:
            return -32011
        case .unauthorized:
            return -32040
        }
    }

    var message: String {
        switch self {
        case let .invalidRequest(message):
            return message
        case .unauthorized:
            return "The inspector token is missing or invalid."
        case let .methodNotFound(method):
            return "Unknown inspector method: \(method)"
        case let .targetNotFound(id):
            return "No view was found for id \(id)."
        case let .unsupportedAction(action):
            return "Unsupported view action: \(action)"
        case let .actionFailed(message):
            return message
        }
    }
}

enum InspectorProtocol {
    static let version = "0.1"

    static let methods = [
        "Runtime.ping",
        "Inspector.getInfo",
        "View.getHierarchy",
        "View.get",
        "View.hitTest",
        "View.describeAtPoint",
        "View.listActions",
        "View.perform",
    ]

    static func success(id: JSONValue?, result: JSONValue) throws -> Data {
        try encode([
            "id": id ?? .null,
            "result": result,
        ])
    }

    static func failure(id: JSONValue?, _ error: Error) -> Data {
        let failure: InspectorFailure
        if let error = error as? InspectorFailure {
            failure = error
        } else {
            failure = .actionFailed(error.localizedDescription)
        }

        let payload: [String: JSONValue] = [
            "id": id ?? .null,
            "error": .object([
                "code": .number(Double(failure.code)),
                "message": .string(failure.message),
            ]),
        ]
        return (try? encode(payload)) ?? Data("{\"error\":{\"code\":-32603,\"message\":\"Internal encoding error\"}}\n".utf8)
    }

    static func event(_ name: String, params: JSONValue) -> Data {
        let payload: [String: JSONValue] = [
            "event": .string(name),
            "params": params,
        ]
        return (try? encode(payload)) ?? Data()
    }

    private static func encode(_ value: [String: JSONValue]) throws -> Data {
        var data = try JSONEncoder.simDeckInspector.encode(value)
        data.append(0x0A)
        return data
    }
}
