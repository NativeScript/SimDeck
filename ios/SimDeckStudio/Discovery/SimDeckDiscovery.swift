import Darwin
import Foundation
import Observation

@MainActor
@Observable
final class SimDeckDiscovery {
    var endpoints: [SimDeckEndpoint] = []
    var isScanning = false

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
        if let index = endpoints.firstIndex(where: { $0.baseURL == endpoint.baseURL }) {
            endpoints[index] = endpoint
            isNewEndpoint = false
        } else {
            endpoints.append(endpoint)
            isNewEndpoint = true
        }
        endpoints.sort {
            if $0.source == $1.source {
                return $0.name.localizedStandardCompare($1.name) == .orderedAscending
            }
            return sourceRank($0.source) < sourceRank($1.source)
        }
        if isNewEndpoint {
            onEndpoint?(endpoint)
        }
    }

    private func sourceRank(_ source: EndpointSource) -> Int {
        switch source {
        case .bonjour: 0
        case .tailscale: 1
        case .lan: 2
        case .studioLink: 3
        case .manual: 4
        case .recent: 5
        }
    }

    private static func scanLikelyHosts() async -> [SimDeckEndpoint] {
        let candidates = IPv4Interface.discoveryCandidates()
        let ports = [4310, 4311, 4312, 4313, 4314, 4320]
        return await scan(candidates: candidates, ports: ports)
    }

    private static func scanPriorityHosts() async -> [SimDeckEndpoint] {
        for port in [4313, 4310, 4311, 4312, 4314, 4320] {
            if let endpoint = await probe(host: "127.0.0.1", port: port, source: .manual) {
                return [endpoint]
            }
        }
        if let endpoint = await probe(host: "localhost", port: 4313, source: .manual) {
            return [endpoint]
        }
        if let endpoint = await probe(host: "simdeck.local", port: 4310, source: .bonjour) {
            return [endpoint]
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
                return SimDeckEndpoint(
                    name: "SimDeck \(host)",
                    baseURL: baseURL,
                    source: source,
                    requiresPairing: true
                )
            }
            guard http.statusCode == 200,
                  (try? JSONDecoder().decode(HealthResponse.self, from: data)).map(\.ok) == true else {
                return nil
            }
            return SimDeckEndpoint(name: endpointName(for: host), baseURL: baseURL, source: source)
        } catch {
            return nil
        }
    }

    private static func endpointName(for host: String) -> String {
        if host == "127.0.0.1" || host == "localhost" {
            return "Local SimDeck"
        }
        return "SimDeck \(host)"
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
        var components = URLComponents()
        components.scheme = "http"
        components.host = host
        components.port = sender.port
        guard let url = components.url else { return }
        onEndpoint?(
            SimDeckEndpoint(
                name: sender.name.isEmpty ? "SimDeck \(host)" : sender.name,
                baseURL: url,
                source: .bonjour
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
