import Darwin
import Foundation
import Observation

@MainActor
@Observable
final class SimDeckDiscovery {
    var endpoints: [SimDeckEndpoint] = []
    var isScanning = false

    private static let launchAgentDiscoveryPorts = Array(4310...4320)
    private static let priorityDiscoveryPorts = [4313] + launchAgentDiscoveryPorts.filter { $0 != 4313 }

    @ObservationIgnored private let bonjour = BonjourDiscovery()
    @ObservationIgnored private var scanTask: Task<Void, Never>?
    @ObservationIgnored var onEndpoint: ((SimDeckEndpoint) -> Void)?

    init() {
        bonjour.onEndpoint = { [weak self] endpoint in
            Task { @MainActor in
                self?.upsert(endpoint)
            }
        }
    }

    func start() {
        bonjour.start()
        refresh()
    }

    func stop() {
        bonjour.stop()
        scanTask?.cancel()
        scanTask = nil
    }

    func refresh() {
        scanTask?.cancel()
        isScanning = true
        scanTask = Task {
            let local = await Self.scanPriorityHosts()
            guard !Task.isCancelled else { return }
            await MainActor.run {
                for endpoint in local {
                    upsert(endpoint)
                }
            }
            let found = await Self.scanLikelyHosts()
            guard !Task.isCancelled else { return }
            await MainActor.run {
                for endpoint in found {
                    upsert(endpoint)
                }
                isScanning = false
            }
        }
    }

    func upsert(_ endpoint: SimDeckEndpoint) {
        let isNewEndpoint: Bool
        let shouldNotify: Bool
        let notifiedEndpoint: SimDeckEndpoint
        if let index = endpoints.firstIndex(where: { Self.sameServer($0, endpoint) }) {
            let previous = endpoints[index]
            endpoints[index] = Self.mergedEndpoint(previous, endpoint)
            isNewEndpoint = false
            notifiedEndpoint = endpoints[index]
            shouldNotify = previous.baseURL != endpoints[index].baseURL
                || (previous.requiresPairing && !endpoints[index].requiresPairing)
                || previous.hostIdentityKey != endpoints[index].hostIdentityKey
        } else {
            endpoints.append(endpoint)
            isNewEndpoint = true
            shouldNotify = true
            notifiedEndpoint = endpoint
        }
        endpoints.sort {
            if $0.displayName != $1.displayName {
                return $0.displayName.localizedStandardCompare($1.displayName) == .orderedAscending
            }
            if $0.serverKindRank != $1.serverKindRank {
                return $0.serverKindRank < $1.serverKindRank
            }
            return sourceRank($0.source) < sourceRank($1.source)
        }
        if isNewEndpoint || shouldNotify {
            onEndpoint?(notifiedEndpoint)
        }
    }

    private static func sameServer(_ lhs: SimDeckEndpoint, _ rhs: SimDeckEndpoint) -> Bool {
        if let lhsHostID = lhs.normalizedHostID,
           let rhsHostID = rhs.normalizedHostID {
            return lhsHostID == rhsHostID
        }
        if let lhsID = lhs.serverID?.nilIfBlank,
           let rhsID = rhs.serverID?.nilIfBlank {
            return lhsID == rhsID
        }
        if lhs.normalizedHostID == nil,
           rhs.normalizedHostID == nil,
           let lhsHostName = lhs.normalizedHostName,
           let rhsHostName = rhs.normalizedHostName {
            return lhsHostName == rhsHostName
        }
        return lhs.baseURL == rhs.baseURL
            || lhs.alternateBaseURLs.contains(rhs.baseURL)
            || rhs.alternateBaseURLs.contains(lhs.baseURL)
    }

    private static func mergedEndpoint(_ lhs: SimDeckEndpoint, _ rhs: SimDeckEndpoint) -> SimDeckEndpoint {
        let preferred = preferredEndpoint(lhs, rhs)
        let other = preferred.baseURL == lhs.baseURL ? rhs : lhs
        var merged = preferred
        merged.serverID = preferred.serverID ?? other.serverID
        merged.hostID = preferred.hostID ?? other.hostID
        merged.hostName = preferred.hostName ?? other.hostName
        merged.serverKind = preferred.serverKind ?? other.serverKind
        merged.token = preferred.token ?? other.token
        merged.requiresPairing = preferred.requiresPairing && other.requiresPairing
        merged.preferredSimulatorID = preferred.preferredSimulatorID ?? other.preferredSimulatorID
        if let hostName = merged.hostName?.nilIfBlank {
            merged.name = hostName
        }
        merged.alternateBaseURLs = uniquedURLs(
            [lhs.baseURL, rhs.baseURL] + lhs.alternateBaseURLs + rhs.alternateBaseURLs
        )
        .filter { $0 != merged.baseURL }
        return merged
    }

    private static func preferredEndpoint(_ lhs: SimDeckEndpoint, _ rhs: SimDeckEndpoint) -> SimDeckEndpoint {
        if lhs.serverKindRank != rhs.serverKindRank {
            return lhs.serverKindRank < rhs.serverKindRank ? lhs : rhs
        }
        if lhs.requiresPairing != rhs.requiresPairing {
            return lhs.requiresPairing ? rhs : lhs
        }
        if sourceRankValue(lhs.source) != sourceRankValue(rhs.source) {
            return sourceRankValue(lhs.source) < sourceRankValue(rhs.source) ? lhs : rhs
        }
        return lhs
    }

    private static func uniquedURLs(_ urls: [URL]) -> [URL] {
        var seen = Set<URL>()
        var result: [URL] = []
        for url in urls.map({ $0.normalizedSimDeckBaseURL() }) where seen.insert(url).inserted {
            result.append(url)
        }
        return result
    }

    private func sourceRank(_ source: EndpointSource) -> Int {
        Self.sourceRankValue(source)
    }

    private static func sourceRankValue(_ source: EndpointSource) -> Int {
        switch source {
        case .bonjour: 0
        case .lan: 1
        case .tailscale: 2
        case .studioLink: 3
        case .manual: 4
        case .recent: 5
        }
    }

    private static func scanLikelyHosts() async -> [SimDeckEndpoint] {
        let candidates = IPv4Interface.discoveryCandidates()
        return await scan(candidates: candidates, ports: launchAgentDiscoveryPorts)
    }

    private static func scanPriorityHosts() async -> [SimDeckEndpoint] {
        for port in priorityDiscoveryPorts {
            if let endpoint = await probe(host: "127.0.0.1", port: port, source: .manual) {
                return [endpoint]
            }
        }
        for host in ["localhost", "simdeck.local"] {
            for port in priorityDiscoveryPorts {
                let source: EndpointSource = host == "simdeck.local" ? .bonjour : .manual
                if let endpoint = await probe(host: host, port: port, source: source) {
                    return [endpoint]
                }
            }
        }
        return []
    }

    private static func scan(candidates: [DiscoveryCandidate], ports: [Int]) async -> [SimDeckEndpoint] {
        var results: [SimDeckEndpoint] = []

        for batch in candidates.chunked(into: 16) {
            await withTaskGroup(of: SimDeckEndpoint?.self) { group in
                for candidate in batch {
                    for port in ports {
                        group.addTask {
                            await probe(host: candidate.host, port: port, source: candidate.source)
                        }
                    }
                }
                for await endpoint in group {
                    if let endpoint, !results.contains(where: { $0.baseURL == endpoint.baseURL }) {
                        results.append(endpoint)
                    }
                }
            }
        }
        return results
    }

    private static func probe(host: String, port: Int, source: EndpointSource) async -> SimDeckEndpoint? {
        var components = URLComponents()
        components.scheme = "http"
        components.host = host
        components.port = port
        guard let baseURL = components.url,
              let healthURL = URL(string: "/api/health", relativeTo: baseURL) else {
            return nil
        }
        var request = URLRequest(url: healthURL)
        request.cachePolicy = .reloadIgnoringLocalAndRemoteCacheData
        request.timeoutInterval = 1.25
        request.setValue(baseURL.absoluteString.trimmingTrailingSlashes(), forHTTPHeaderField: "Origin")
        do {
            let (data, response) = try await URLSession.shared.data(for: request)
            guard let http = response as? HTTPURLResponse else { return nil }
            if http.statusCode == 401 {
                let health = try? JSONDecoder().decode(HealthResponse.self, from: data)
                return SimDeckEndpoint(
                    name: endpointName(for: host, health: health),
                    baseURL: baseURL,
                    source: source,
                    requiresPairing: true,
                    serverID: health?.serverId,
                    hostID: health?.hostId,
                    hostName: health?.hostName,
                    serverKind: health?.serverKind,
                    alternateBaseURLs: alternateURLs(from: health, fallbackPort: port)
                )
            }
            guard http.statusCode == 200,
                  let health = try? JSONDecoder().decode(HealthResponse.self, from: data),
                  health.ok else {
                return nil
            }
            return SimDeckEndpoint(
                name: endpointName(for: host, health: health),
                baseURL: baseURL,
                source: source,
                serverID: health.serverId,
                hostID: health.hostId,
                hostName: health.hostName,
                serverKind: health.serverKind,
                alternateBaseURLs: alternateURLs(from: health, fallbackPort: port)
            )
        } catch {
            return nil
        }
    }

    private static func alternateURLs(from health: HealthResponse?, fallbackPort: Int) -> [URL] {
        guard let advertiseHost = health?.advertiseHost?.nilIfBlank else { return [] }
        var components = URLComponents()
        components.scheme = "http"
        components.host = advertiseHost
        components.port = health?.httpPort ?? fallbackPort
        return components.url.map { [$0] } ?? []
    }

    private static func endpointName(for host: String) -> String {
        if host == "127.0.0.1" || host == "localhost" {
            return "Local SimDeck"
        }
        return "SimDeck \(host)"
    }

    private static func endpointName(for host: String, health: HealthResponse?) -> String {
        health?.hostName?.nilIfBlank ?? endpointName(for: host)
    }
}

private final class BonjourDiscovery: NSObject, NetServiceBrowserDelegate, NetServiceDelegate {
    var onEndpoint: (@Sendable (SimDeckEndpoint) -> Void)?
    private let browser = NetServiceBrowser()
    private var services: [NetService] = []

    override init() {
        super.init()
        browser.delegate = self
    }

    func start() {
        browser.searchForServices(ofType: "_simdeck._tcp.", inDomain: "local.")
    }

    func stop() {
        browser.stop()
        services.removeAll()
    }

    func netServiceBrowser(_ browser: NetServiceBrowser, didFind service: NetService, moreComing: Bool) {
        services.append(service)
        service.delegate = self
        service.resolve(withTimeout: 2)
    }

    func netServiceDidResolveAddress(_ sender: NetService) {
        let host = sender.hostName?.trimmingTrailingSlashes() ?? "\(sender.name).local"
        let txt = NetService.dictionary(fromTXTRecord: sender.txtRecordData() ?? Data())
        let serverID = txt["sid"].flatMap { String(data: $0, encoding: .utf8) }?.nilIfBlank
        let hostID = txt["hid"].flatMap { String(data: $0, encoding: .utf8) }?.nilIfBlank
            ?? txt["hostId"].flatMap { String(data: $0, encoding: .utf8) }?.nilIfBlank
        let hostName = txt["hname"].flatMap { String(data: $0, encoding: .utf8) }?.nilIfBlank
            ?? txt["hostName"].flatMap { String(data: $0, encoding: .utf8) }?.nilIfBlank
        let serverKind = txt["kind"].flatMap { String(data: $0, encoding: .utf8) }?.nilIfBlank
            ?? txt["serverKind"].flatMap { String(data: $0, encoding: .utf8) }?.nilIfBlank
        let advertisedHost = txt["host"].flatMap { String(data: $0, encoding: .utf8) }?.nilIfBlank
        var components = URLComponents()
        components.scheme = "http"
        components.host = host
        components.port = sender.port
        guard let url = components.url else { return }
        var alternateURLs: [URL] = []
        if let advertisedHost {
            var advertised = URLComponents()
            advertised.scheme = "http"
            advertised.host = advertisedHost
            advertised.port = sender.port
            if let advertisedURL = advertised.url {
                alternateURLs.append(advertisedURL)
            }
        }
        onEndpoint?(
            SimDeckEndpoint(
                name: hostName ?? (sender.name.isEmpty ? "SimDeck \(host)" : sender.name),
                baseURL: url,
                source: .bonjour,
                serverID: serverID,
                hostID: hostID,
                hostName: hostName,
                serverKind: serverKind,
                alternateBaseURLs: alternateURLs
            )
        )
    }
}

private struct DiscoveryCandidate: Hashable {
    let host: String
    let source: EndpointSource
}

private struct IPv4Interface {
    let address: [UInt8]
    let isTailscale: Bool

    static func discoveryCandidates() -> [DiscoveryCandidate] {
        var candidates: [DiscoveryCandidate] = []
        var seen = Set<DiscoveryCandidate>()
        var interfaces: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&interfaces) == 0, let first = interfaces else { return candidates }
        defer { freeifaddrs(interfaces) }

        var cursor: UnsafeMutablePointer<ifaddrs>? = first
        while let current = cursor {
            defer { cursor = current.pointee.ifa_next }
            let flags = Int32(current.pointee.ifa_flags)
            guard flags & IFF_UP != 0, flags & IFF_LOOPBACK == 0 else { continue }
            guard current.pointee.ifa_addr.pointee.sa_family == UInt8(AF_INET) else { continue }
            let socketAddress = current.pointee.ifa_addr.withMemoryRebound(to: sockaddr_in.self, capacity: 1) { $0.pointee }
            let host = IPv4Interface.bytes(from: socketAddress.sin_addr)
            guard host.count == 4 else { continue }
            let source: EndpointSource = host[0] == 100 && (host[1] & 0b1100_0000) == 0b0100_0000 ? .tailscale : .lan
            for last in UInt8(1)...UInt8(254) where last != host[3] {
                let candidate = DiscoveryCandidate(host: "\(host[0]).\(host[1]).\(host[2]).\(last)", source: source)
                if seen.insert(candidate).inserted {
                    candidates.append(candidate)
                }
            }
        }
        return candidates
    }

    private static func bytes(from address: in_addr) -> [UInt8] {
        var address = address
        return withUnsafeBytes(of: &address.s_addr) { Array($0) }
    }
}

private extension Array {
    func chunked(into size: Int) -> [[Element]] {
        guard size > 0 else { return [self] }
        return stride(from: 0, to: count, by: size).map {
            Array(self[$0..<Swift.min($0 + size, count)])
        }
    }
}
