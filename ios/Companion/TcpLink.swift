import Darwin
import Foundation

struct LinkError: Error, CustomStringConvertible {
    let description: String
}

/// A byte stream the app opened. The core does not care what carries it.
protocol ByteLink: AnyObject {
    /// How to pull over this link.
    var profile: LinkProfile { get }

    /// Blocks until bytes arrive. Returns how many, or 0 at the end.
    func read(into buffer: inout [UInt8]) throws -> Int

    func write(_ bytes: Data) throws

    /// Makes a blocked `read` or `write` return. Safe to call from any thread, more than once.
    func close()
}

/// A TCP connection to the Glass or to `omg sim-glass`, on POSIX sockets: a read in one thread
/// never waits for a write in another.
final class TcpLink: ByteLink {
    let profile = linkProfileFast()
    private let fd: Int32

    /// Opens the connection, giving up after five seconds or once `cancelled` returns true.
    init(host: String, port: UInt16, cancelled: () -> Bool) throws {
        var hints = addrinfo(
            ai_flags: 0, ai_family: AF_UNSPEC, ai_socktype: SOCK_STREAM, ai_protocol: IPPROTO_TCP,
            ai_addrlen: 0, ai_canonname: nil, ai_addr: nil, ai_next: nil
        )
        var found: UnsafeMutablePointer<addrinfo>?
        let rc = getaddrinfo(host, String(port), &hints, &found)
        guard rc == 0, let first = found else {
            throw LinkError(description: "cannot resolve \(host): \(String(cString: gai_strerror(rc)))")
        }
        defer { freeaddrinfo(found) }

        var lastError = "no address for \(host)"
        var next: UnsafeMutablePointer<addrinfo>? = first
        while let info = next {
            next = info.pointee.ai_next
            do {
                fd = try TcpLink.connect(info.pointee, cancelled: cancelled)
                return
            } catch let e as LinkError {
                lastError = e.description
            }
            if cancelled() { break }
        }
        throw LinkError(description: lastError)
    }

    deinit {
        Darwin.close(fd)
    }

    private static func connect(_ info: addrinfo, cancelled: () -> Bool) throws -> Int32 {
        let fd = socket(info.ai_family, info.ai_socktype, info.ai_protocol)
        guard fd >= 0 else { throw LinkError(description: errnoText("socket")) }
        var on: Int32 = 1
        setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &on, socklen_t(MemoryLayout<Int32>.size))
        setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))

        // Connect without blocking, so that a wanted link can be dropped while waiting.
        let flags = fcntl(fd, F_GETFL)
        _ = fcntl(fd, F_SETFL, flags | O_NONBLOCK)
        if Darwin.connect(fd, info.ai_addr, info.ai_addrlen) != 0 && errno != EINPROGRESS {
            let why = errnoText("connect")
            Darwin.close(fd)
            throw LinkError(description: why)
        }
        let deadline = Date().addingTimeInterval(connectTimeout)
        while true {
            var p = pollfd(fd: fd, events: Int16(POLLOUT), revents: 0)
            let n = poll(&p, 1, 100)
            if n > 0 {
                var err: Int32 = 0
                var len = socklen_t(MemoryLayout<Int32>.size)
                getsockopt(fd, SOL_SOCKET, SO_ERROR, &err, &len)
                if err != 0 {
                    Darwin.close(fd)
                    throw LinkError(description: String(cString: strerror(err)))
                }
                break
            }
            if n < 0 && errno != EINTR {
                let why = errnoText("poll")
                Darwin.close(fd)
                throw LinkError(description: why)
            }
            if cancelled() || Date() >= deadline {
                Darwin.close(fd)
                throw LinkError(description: cancelled() ? "cancelled" : "connection timed out")
            }
        }
        _ = fcntl(fd, F_SETFL, flags)
        return fd
    }

    func read(into buffer: inout [UInt8]) throws -> Int {
        while true {
            let n = buffer.withUnsafeMutableBytes { recv(fd, $0.baseAddress, $0.count, 0) }
            if n >= 0 { return n }
            if errno != EINTR { throw LinkError(description: TcpLink.errnoText("read")) }
        }
    }

    func write(_ bytes: Data) throws {
        try bytes.withUnsafeBytes { (raw: UnsafeRawBufferPointer) in
            var sent = 0
            while sent < raw.count {
                let n = send(fd, raw.baseAddress! + sent, raw.count - sent, 0)
                if n < 0 {
                    if errno == EINTR { continue }
                    throw LinkError(description: TcpLink.errnoText("write"))
                }
                sent += n
            }
        }
    }

    func close() {
        // Wakes a blocked read or write. The descriptor itself is closed with this object, after
        // the threads that use it are done.
        shutdown(fd, SHUT_RDWR)
    }

    private static let connectTimeout: TimeInterval = 5

    private static func errnoText(_ what: String) -> String {
        "\(what): \(String(cString: strerror(errno)))"
    }
}
