import AppKit
import ImageIO
import Foundation

let board = NSPasteboard.general
let maxBytes = 8 * 1024 * 1024
func response(_ value: [String: Any]) {
    if let bytes = try? JSONSerialization.data(withJSONObject: value), let text = String(data: bytes, encoding: .utf8) {
        print(text)
        fflush(stdout)
    }
}
while let line = readLine() {
    autoreleasepool {
        do {
            guard line.utf8.count <= 12 * 1024 * 1024,
                  let bytes = line.data(using: .utf8),
                  let request = try JSONSerialization.jsonObject(with: bytes) as? [String: Any],
                  let op = request["op"] as? String else {
                response(["error": "invalid clipboard request"]); return
            }
            let before = board.changeCount
            if op == "write" {
                guard let expected = request["generation"] as? Int, expected == before else {
                    response(["error": "clipboard superseded"]); return
                }
                guard let type = request["format"] as? String, ["text", "png"].contains(type),
                      let encoded = request["data"] as? String,
                      let data = Data(base64Encoded: encoded), data.count <= maxBytes else {
                    response(["error": "invalid clipboard content"]); return
                }
                let item = NSPasteboardItem()
                guard item.setData(data, forType: type == "text" ? .string : .png) else {
                    response(["error": "pasteboard item failed"]); return
                }
                board.clearContents()
                guard board.writeObjects([item]) else {
                    response(["error": "pasteboard write failed"]); return
                }
                response(["generation": board.changeCount]); return
            }
            let types = board.types ?? []
            let sensitive = types.contains(NSPasteboard.PasteboardType("org.nspasteboard.ConcealedType")) || types.contains(NSPasteboard.PasteboardType("org.nspasteboard.TransientType"))
            var out: [String: Any] = ["generation": before, "sensitive": sensitive]
            if op == "read" {
                if types.contains(.png), let data = board.data(forType: .png) {
                    guard data.count <= maxBytes else {response(["error": "PNG exceeds limit"]); return}
                    out["format"] = "png"; out["data"] = data.base64EncodedString()
                } else if types.contains(.tiff), let data = board.data(forType: .tiff) {
                    guard data.count <= maxBytes,
                          let source = CGImageSourceCreateWithData(data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary),
                          let props = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
                          let width = props[kCGImagePropertyPixelWidth] as? Int,
                          let height = props[kCGImagePropertyPixelHeight] as? Int,
                          width > 0, height > 0, width <= 16384, height <= 16384,
                          width * height <= 4 * 1024 * 1024,
                          let cgImage = CGImageSourceCreateImageAtIndex(source, 0, nil),
                          let png = NSBitmapImageRep(cgImage: cgImage).representation(using: .png, properties: [:]), png.count <= maxBytes else {
                        response(["error": "TIFF conversion exceeds limits"]); return
                    }
                    out["format"] = "png"; out["data"] = png.base64EncodedString()
                } else if types.contains(.string), let data = board.data(forType: .string) {
                    guard data.count <= 1024 * 1024 else {response(["error": "text exceeds limit"]); return}
                    out["format"] = "text"; out["data"] = data.base64EncodedString()
                }
                guard before == board.changeCount else {response(["error": "clipboard changed during capture"]); return}
            } else if op != "inspect" {response(["error": "unsupported operation"]); return}
            response(out)
        } catch { response(["error": "invalid clipboard request"]) }
    }
}
