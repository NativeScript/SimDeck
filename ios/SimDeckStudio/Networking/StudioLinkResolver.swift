import Foundation

enum StudioLinkResolver {
    static func route(for url: URL) -> AppRoute? {
        if let endpoint = endpointFromCustomScheme(url) {
            return .endpoint(endpoint, autoStart: true)
        }
        guard ["http", "https"].contains(url.scheme?.lowercased() ?? "") else {
            return nil
        }
        if let endpoint = endpointFromStudioURL(url) {
            return .endpoint(endpoint, autoStart: true)
        }
        return .endpoint(
            SimDeckEndpoint(
                name: url.host ?? "SimDeck",
                baseURL: url,
                source: source(for: url.host),
                preferredSimulatorID: queryValue("device", in: url) ?? queryValue("udid", in: url)
            ),
            autoStart: true
        )
    }

    static func endpointFromAddress(_ value: String, token: String? = nil) -> SimDeckEndpoint? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        let withScheme = trimmed.contains("://") ? trimmed : "http://\(trimmed)"
        guard let url = URL(string: withScheme), url.host != nil else { return nil }
        if let endpoint = endpointFromStudioURL(url) {
            var endpointWithToken = endpoint
            endpointWithToken.token = token?.nilIfBlank
            return endpointWithToken
        }
        return SimDeckEndpoint(name: url.host ?? "SimDeck", baseURL: url, source: source(for: url.host), token: token)
    }

    private static func endpointFromCustomScheme(_ url: URL) -> SimDeckEndpoint? {
        guard url.scheme?.lowercased() == "simdeck" else { return nil }
        if let rawURL = queryValue("url", in: url), let nested = URL(string: rawURL) {
            return endpointFromStudioURL(nested) ?? SimDeckEndpoint(name: nested.host ?? "SimDeck", baseURL: nested, source: .manual)
        }
        guard let host = queryValue("host", in: url) ?? url.host else { return nil }
        let port = queryValue("port", in: url).flatMap(Int.init)
        var components = URLComponents()
        components.scheme = queryValue("scheme", in: url) ?? "http"
        components.host = host
        components.port = port
        guard let baseURL = components.url else { return nil }
        return SimDeckEndpoint(
            name: host,
            baseURL: baseURL,
            source: source(for: host),
            token: queryValue("token", in: url),
            preferredSimulatorID: queryValue("device", in: url) ?? queryValue("udid", in: url)
        )
    }

    private static func endpointFromStudioURL(_ url: URL) -> SimDeckEndpoint? {
        let parts = url.pathComponents.filter { $0 != "/" }
        guard let simulatorIndex = parts.firstIndex(of: "simulator"),
              parts.indices.contains(simulatorIndex + 1) else {
            return nil
        }
        let previewID = parts[simulatorIndex + 1]
        guard !previewID.isEmpty,
              var components = URLComponents(url: url, resolvingAgainstBaseURL: false) else {
            return nil
        }
        components.path = "/api/provider-sessions/\(previewID)/simdeck"
        components.query = nil
        components.fragment = nil
        guard let baseURL = components.url else { return nil }
        return SimDeckEndpoint(
            name: "Studio \(previewID)",
            baseURL: baseURL,
            source: .studioLink,
            token: queryValue("simdeckToken", in: url) ?? queryValue("token", in: url),
            preferredSimulatorID: queryValue("device", in: url) ?? queryValue("udid", in: url)
        )
    }

    private static func queryValue(_ name: String, in url: URL) -> String? {
        URLComponents(url: url, resolvingAgainstBaseURL: false)?
            .queryItems?
            .first { $0.name == name }?
            .value?
            .nilIfBlank
    }

    private static func source(for host: String?) -> EndpointSource {
        guard let host, isTailscaleIPv4Host(host) else { return .manual }
        return .tailscale
    }

    private static func isTailscaleIPv4Host(_ host: String) -> Bool {
        let parts = host.split(separator: ".").compactMap { UInt8($0) }
        guard parts.count == 4 else { return false }
        return parts[0] == 100 && (parts[1] & 0b1100_0000) == 0b0100_0000
    }
}
