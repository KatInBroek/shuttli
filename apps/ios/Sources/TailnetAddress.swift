import Foundation
import Darwin

enum TailnetAddress {
    static func currentIPv4() -> String? {
        var first: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&first) == 0 else { return nil }
        defer { freeifaddrs(first) }
        var cursor = first
        while let node = cursor {
            defer { cursor = node.pointee.ifa_next }
            let interface = node.pointee
            guard String(cString: interface.ifa_name).hasPrefix("utun"),
                  let address = interface.ifa_addr,
                  address.pointee.sa_family == UInt8(AF_INET) else { continue }
            var ipv4 = UnsafeRawPointer(address).assumingMemoryBound(to: sockaddr_in.self).pointee.sin_addr
            var buffer = [CChar](repeating: 0, count: Int(INET_ADDRSTRLEN))
            let converted = withUnsafePointer(to: &ipv4) { pointer in
                buffer.withUnsafeMutableBufferPointer { chars in
                    inet_ntop(AF_INET, pointer, chars.baseAddress, socklen_t(chars.count))
                }
            }
            guard converted != nil else { continue }
            let text = String(cString: buffer)
            let octets = text.split(separator: ".").compactMap { UInt8($0) }
            if octets.count == 4 && octets[0] == 100 && (64...127).contains(octets[1]) {
                return text
            }
        }
        return nil
    }
}
