import Foundation
import Network

final class InspectorTCPServer {
    private let configuration: SimDeckInspectorAgent.Configuration
    private let requestHandler: (Data, @escaping (Data) -> Void) -> Void
    private let queue = DispatchQueue(label: "dev.simdeck.inspector-agent.tcp")
    private var listener: NWListener?
    private var connections: [ObjectIdentifier: InspectorClientConnection] = [:]

    init(
        configuration: SimDeckInspectorAgent.Configuration,
        requestHandler: @escaping (Data, @escaping (Data) -> Void) -> Void
    ) {
        self.configuration = configuration
        self.requestHandler = requestHandler
    }

    func start() throws -> UInt16 {
        let maxOffset = min(configuration.portSearchLimit, UInt16.max - configuration.port)
        var lastError: Error?

        for offset in 0...maxOffset {
            let rawPort = configuration.port + offset
            guard let port = NWEndpoint.Port(rawValue: rawPort) else {
                continue
            }

            do {
                let listener = try NWListener(using: listenerParameters(), on: port)
                let startup = ListenerStartupState()
                configure(listener, startup: startup)
                listener.start(queue: queue)
                if case let .failed(error) = startup.wait(timeout: .now() + .milliseconds(500)) {
                    listener.cancel()
                    lastError = error
                    continue
                }
                self.listener = listener
                return rawPort
            } catch {
                lastError = error
                continue
            }
        }

        if let lastError {
            throw lastError
        }
        throw InspectorFailure.invalidRequest("No valid TCP port was available for inspector agent.")
    }

    func stop() {
        queue.sync {
            for connection in connections.values {
                connection.cancel()
            }
            connections.removeAll()
            listener?.cancel()
            listener = nil
        }
    }

    private func listenerParameters() -> NWParameters {
        let parameters = NWParameters.tcp
        if configuration.bindToLocalhostOnly {
            parameters.requiredLocalEndpoint = .hostPort(host: .ipv4(.loopback), port: .any)
        }
        return parameters
    }

    private func configure(_ listener: NWListener, startup: ListenerStartupState) {
        if configuration.advertiseBonjour {
            listener.service = NWListener.Service(
                name: configuration.serviceName,
                type: "_simdeckinspector._tcp"
            )
        }

        listener.newConnectionHandler = { [weak self] connection in
            self?.accept(connection)
        }
        listener.stateUpdateHandler = { state in
            switch state {
            case .ready:
                startup.succeed()
            case let .failed(error):
                startup.fail(error)
                NSLog("SimDeckInspectorAgent listener failed: \(error)")
            default:
                break
            }
        }
    }

    private func accept(_ connection: NWConnection) {
        let client = InspectorClientConnection(
            connection: connection,
            queue: queue,
            requestHandler: requestHandler,
            onClose: { [weak self] identifier in
                self?.connections.removeValue(forKey: identifier)
            }
        )
        connections[client.identifier] = client
        client.start()
    }
}

private enum ListenerStartupResult {
    case ready
    case failed(NWError)
    case timedOut
}

private final class ListenerStartupState {
    private let semaphore = DispatchSemaphore(value: 0)
    private let lock = NSLock()
    private var result: ListenerStartupResult?

    func succeed() {
        resolve(.ready)
    }

    func fail(_ error: NWError) {
        resolve(.failed(error))
    }

    func wait(timeout: DispatchTime) -> ListenerStartupResult {
        if semaphore.wait(timeout: timeout) == .timedOut {
            return .timedOut
        }

        lock.lock()
        defer { lock.unlock() }
        return result ?? .timedOut
    }

    private func resolve(_ result: ListenerStartupResult) {
        lock.lock()
        defer { lock.unlock() }
        guard self.result == nil else {
            return
        }
        self.result = result
        semaphore.signal()
    }
}

private final class InspectorClientConnection {
    var identifier: ObjectIdentifier {
        ObjectIdentifier(self)
    }

    private let connection: NWConnection
    private let queue: DispatchQueue
    private let requestHandler: (Data, @escaping (Data) -> Void) -> Void
    private let onClose: (ObjectIdentifier) -> Void
    private var buffer = Data()
    private let maxFrameBytes = 1024 * 1024

    init(
        connection: NWConnection,
        queue: DispatchQueue,
        requestHandler: @escaping (Data, @escaping (Data) -> Void) -> Void,
        onClose: @escaping (ObjectIdentifier) -> Void
    ) {
        self.connection = connection
        self.queue = queue
        self.requestHandler = requestHandler
        self.onClose = onClose
    }

    func start() {
        connection.stateUpdateHandler = { [weak self] state in
            switch state {
            case .ready:
                self?.send(InspectorProtocol.event("Inspector.connected", params: .object([
                    "protocolVersion": .string(InspectorProtocol.version),
                    "framing": .string("ndjson"),
                ])))
            case .cancelled, .failed:
                guard let self else {
                    return
                }
                self.onClose(self.identifier)
            default:
                break
            }
        }
        connection.start(queue: queue)
        receive()
    }

    func cancel() {
        connection.cancel()
    }

    private func receive() {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 64 * 1024) { [weak self] data, _, isComplete, error in
            guard let self else {
                return
            }

            if let data, !data.isEmpty {
                self.buffer.append(data)
                self.drainFrames()
            }

            if isComplete || error != nil {
                self.connection.cancel()
                self.onClose(self.identifier)
                return
            }

            self.receive()
        }
    }

    private func drainFrames() {
        while let newline = buffer.firstIndex(of: 0x0A) {
            let frame = buffer[..<newline]
            buffer.removeSubrange(...newline)
            guard !frame.isEmpty else {
                continue
            }
            requestHandler(Data(frame)) { [weak self] response in
                self?.send(response)
            }
        }

        if buffer.count > maxFrameBytes {
            send(InspectorProtocol.failure(id: nil, InspectorFailure.invalidRequest("Inspector frame exceeded \(maxFrameBytes) bytes.")))
            buffer.removeAll()
        }
    }

    private func send(_ data: Data) {
        connection.send(content: data, completion: .contentProcessed { error in
            if let error {
                NSLog("SimDeckInspectorAgent send failed: \(error)")
            }
        })
    }
}
