import Foundation

public enum JSONValue: Codable, Equatable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Double.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([JSONValue].self) {
            self = .array(value)
        } else {
            self = .object(try container.decode([String: JSONValue].self))
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null:
            try container.encodeNil()
        case let .bool(value):
            try container.encode(value)
        case let .number(value):
            try container.encode(value)
        case let .string(value):
            try container.encode(value)
        case let .array(value):
            try container.encode(value)
        case let .object(value):
            try container.encode(value)
        }
    }
}

extension JSONValue {
    var objectValue: [String: JSONValue]? {
        guard case let .object(value) = self else {
            return nil
        }
        return value
    }

    var arrayValue: [JSONValue]? {
        guard case let .array(value) = self else {
            return nil
        }
        return value
    }

    var stringValue: String? {
        switch self {
        case let .string(value):
            return value
        case let .number(value):
            return String(value)
        case let .bool(value):
            return String(value)
        default:
            return nil
        }
    }

    var doubleValue: Double? {
        switch self {
        case let .number(value):
            return value
        case let .string(value):
            return Double(value)
        default:
            return nil
        }
    }

    var intValue: Int? {
        doubleValue.map(Int.init)
    }

    var boolValue: Bool? {
        switch self {
        case let .bool(value):
            return value
        case let .string(value):
            switch value.lowercased() {
            case "true", "1", "yes":
                return true
            case "false", "0", "no":
                return false
            default:
                return nil
            }
        case let .number(value):
            return value != 0
        default:
            return nil
        }
    }
}

extension Dictionary where Key == String, Value == JSONValue {
    func string(_ key: String) -> String? {
        self[key]?.stringValue
    }

    func double(_ key: String) -> Double? {
        self[key]?.doubleValue
    }

    func int(_ key: String) -> Int? {
        self[key]?.intValue
    }

    func bool(_ key: String) -> Bool? {
        self[key]?.boolValue
    }
}

extension JSONEncoder {
    static let xcwInspector: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.withoutEscapingSlashes]
        return encoder
    }()
}

extension JSONDecoder {
    static let xcwInspector = JSONDecoder()
}

func xcwJSONValue<T: Encodable>(_ value: T) throws -> JSONValue {
    let data = try JSONEncoder.xcwInspector.encode(value)
    return try JSONDecoder.xcwInspector.decode(JSONValue.self, from: data)
}
