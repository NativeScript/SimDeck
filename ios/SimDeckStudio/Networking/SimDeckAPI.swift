import Foundation
import UIKit

enum SimDeckAPIError: LocalizedError {
    case authRequired
    case invalidResponse
    case requestFailed(Int, String)

    var errorDescription: String? {
        switch self {
        case .authRequired:
            "Pairing or an API token is required."
        case .invalidResponse:
            "SimDeck returned an invalid response."
        case let .requestFailed(status, message):
            "Request failed with status \(status): \(message)"
        }
    }
}

struct SimDeckAPI: Sendable {
    let endpoint: SimDeckEndpoint
    var baseURL: URL { endpoint.baseURL }

    func health(timeout: TimeInterval = 5) async throws -> HealthResponse {
        try await decode(path: "/api/health", timeout: timeout)
    }

    func simulators() async throws -> [SimulatorMetadata] {
        let response: SimulatorsResponse = try await decode(path: "/api/simulators")
        return response.simulators
    }

    func simulatorCreateOptions() async throws -> SimulatorCreateOptionsResponse {
        try await decode(path: "/api/simulators/create-options")
    }

    func createSimulator(_ payload: CreateSimulatorRequest) async throws -> CreateSimulatorResponse {
        try await decode(path: "/api/simulators", method: "POST", body: payload, timeout: 300)
    }

    func pair(code: String) async throws -> String? {
        let payload = ["code": code]
        let (data, response) = try await requestWithHTTPResponse(
            path: "/api/pair",
            method: "POST",
            body: payload,
            timeout: 10
        )
        let pairResponse = try? JSONDecoder().decode(PairResponse.self, from: data)
        return pairResponse?.accessToken?.nilIfBlank ?? accessToken(from: response)
    }

    func bootSimulator(udid: String) async throws {
        let _: EmptyResponse = try await decode(
            path: "/api/simulators/\(udid.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? udid)/boot",
            method: "POST",
            body: Optional<String>.none,
            timeout: 300
        )
    }

    func postWebRTCOffer(_ offer: WebRTCOfferPayload, udid: String) async throws -> WebRTCAnswerPayload {
        try await decode(
            path: "/api/simulators/\(udid.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? udid)/webrtc/offer",
            method: "POST",
            body: offer,
            timeout: 15
        )
    }

    func chromeProfile(udid: String) async throws -> ChromeProfile {
        try await decode(
            path: "/api/simulators/\(udid.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? udid)/chrome-profile"
        )
    }

    func chromeImage(udid: String) async throws -> UIImage {
        let data = try await request(
            path: "/api/simulators/\(udid.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? udid)/chrome.png",
            method: "GET",
            body: Optional<String>.none,
            timeout: 10
        )
        guard let image = UIImage(data: data) else {
            throw SimDeckAPIError.invalidResponse
        }
        return image
    }

    func postControl(_ payload: some Encodable, path: String) async throws {
        let _: EmptyResponse = try await decode(path: path, method: "POST", body: payload)
    }

    private func decode<T: Decodable>(
        path: String,
        method: String = "GET",
        body: (some Encodable)? = Optional<String>.none,
        timeout: TimeInterval = 10
    ) async throws -> T {
        let data = try await request(path: path, method: method, body: body, timeout: timeout)
        if data.isEmpty, T.self == EmptyResponse.self {
            return EmptyResponse() as! T
        }
        return try JSONDecoder().decode(T.self, from: data)
    }

    private func request(
        path: String,
        method: String,
        body: (some Encodable)?,
        timeout: TimeInterval
    ) async throws -> Data {
        let (data, _) = try await requestWithHTTPResponse(path: path, method: method, body: body, timeout: timeout)
        return data
    }

    private func requestWithHTTPResponse(
        path: String,
        method: String,
        body: (some Encodable)?,
        timeout: TimeInterval
    ) async throws -> (Data, HTTPURLResponse) {
        var request = URLRequest(url: url(for: path))
        request.httpMethod = method
        request.timeoutInterval = timeout
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue(originHeaderValue, forHTTPHeaderField: "Origin")
        if let token = endpoint.token?.nilIfBlank {
            request.setValue(token, forHTTPHeaderField: "X-SimDeck-Token")
        }
        if let body {
            request.httpBody = try JSONEncoder().encode(AnyEncodable(body))
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        }

        let (data, response) = try await URLSession.shared.data(for: request)
        guard let httpResponse = response as? HTTPURLResponse else {
            throw SimDeckAPIError.invalidResponse
        }
        if httpResponse.statusCode == 401 {
            throw SimDeckAPIError.authRequired
        }
        guard (200..<300).contains(httpResponse.statusCode) else {
            let message = String(data: data, encoding: .utf8) ?? HTTPURLResponse.localizedString(forStatusCode: httpResponse.statusCode)
            throw SimDeckAPIError.requestFailed(httpResponse.statusCode, message)
        }
        return (data, httpResponse)
    }

    private func accessToken(from response: HTTPURLResponse) -> String? {
        let headerFields = response.allHeaderFields.reduce(into: [String: String]()) { result, item in
            guard let key = item.key as? String else { return }
            result[key] = String(describing: item.value)
        }
        return HTTPCookie
            .cookies(withResponseHeaderFields: headerFields, for: baseURL)
            .first { $0.name == "simdeck_token" }?
            .value
            .nilIfBlank
    }

    private var originHeaderValue: String {
        guard var components = URLComponents(url: baseURL, resolvingAgainstBaseURL: false) else {
            return baseURL.absoluteString
        }
        components.path = ""
        components.query = nil
        components.fragment = nil
        return components.url?.absoluteString.trimmingTrailingSlashes() ?? baseURL.absoluteString
    }

    private func url(for path: String) -> URL {
        guard var components = URLComponents(url: baseURL, resolvingAgainstBaseURL: false) else {
            return baseURL.appendingPathComponent(path)
        }
        let prefix = components.path.trimmingTrailingSlashes()
        let suffix = path.hasPrefix("/") ? path : "/\(path)"
        components.path = "\(prefix)\(suffix)"
        return components.url ?? baseURL.appendingPathComponent(path)
    }
}

private struct EmptyResponse: Codable {}

private struct PairResponse: Decodable {
    let ok: Bool
    let accessToken: String?
}

private struct AnyEncodable: Encodable {
    private let encodeValue: (Encoder) throws -> Void

    init(_ value: some Encodable) {
        encodeValue = value.encode
    }

    func encode(to encoder: Encoder) throws {
        try encodeValue(encoder)
    }
}
