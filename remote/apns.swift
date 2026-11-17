// Native HTTP/2 APNs transport. Credentials never cross argv/stdout.
import Foundation
import CryptoKit

struct Config: Decodable { let keyPath: String; let keyID: String; let teamID: String }
struct Input: Decodable { let token: String; let environment: String; let collapse: String; let payload: Payload }
struct Payload: Codable {
    struct APS: Codable {
        struct Alert: Codable { let title: String; let body: String }
        let alert: Alert; let sound: String; let threadID: String
        var category: String?
        enum CodingKeys: String, CodingKey { case alert, sound, category; case threadID = "thread-id" }
    }
    var aps: APS; let account: String; let folder: String; let uid: Int

    func forDelivery() -> Payload {
        var payload = self
        let refersToMail = !account.isEmpty && !folder.isEmpty && uid > 0 && uid <= Int(UInt32.max)
        payload.aps.category = refersToMail ? "CXMAIL_SMART_MAIL" : nil
        return payload
    }
}
func b64(_ data: Data) -> String {
    data.base64EncodedString().replacingOccurrences(of: "+", with: "-")
        .replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "")
}

Task {
    do {
        let config = try JSONDecoder().decode(Config.self, from: Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1])))
        let input = try JSONDecoder().decode(Input.self, from: FileHandle.standardInput.readDataToEndOfFile())
        guard ["development", "production"].contains(input.environment),
              input.token.range(of: "^[0-9a-f]{32,512}$", options: .regularExpression) != nil else { exit(1) }
        let key = try P256.Signing.PrivateKey(pemRepresentation: String(contentsOfFile: config.keyPath, encoding: .utf8))
        let header = b64(try JSONSerialization.data(withJSONObject: ["alg": "ES256", "kid": config.keyID]))
        let claims = b64(try JSONSerialization.data(withJSONObject: ["iss": config.teamID, "iat": Int(Date().timeIntervalSince1970)]))
        let unsigned = "\(header).\(claims)"
        let jwt = unsigned + "." + b64(try key.signature(for: Data(unsigned.utf8)).rawRepresentation)
        let host = input.environment == "development" ? "api.sandbox.push.apple.com" : "api.push.apple.com"
        var request = URLRequest(url: URL(string: "https://\(host)/3/device/\(input.token)")!)
        request.httpMethod = "POST"; request.timeoutInterval = 20
        request.setValue("bearer \(jwt)", forHTTPHeaderField: "authorization")
        request.setValue("app.cxmail.remote", forHTTPHeaderField: "apns-topic")
        request.setValue("alert", forHTTPHeaderField: "apns-push-type")
        request.setValue("10", forHTTPHeaderField: "apns-priority")
        request.setValue(String(Int(Date().timeIntervalSince1970) + 3600), forHTTPHeaderField: "apns-expiration")
        request.setValue(input.collapse, forHTTPHeaderField: "apns-collapse-id")
        request.httpBody = try JSONEncoder().encode(input.payload.forDelivery())
        let (data, response) = try await URLSession.shared.data(for: request)
        guard let http = response as? HTTPURLResponse else { exit(1) }
        let body = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
        let result: [String: Any] = ["status": http.statusCode, "reason": body?["reason"] as? String ?? ""]
        print(String(data: try JSONSerialization.data(withJSONObject: result), encoding: .utf8)!)
        exit(0)
    } catch { exit(1) }
}
dispatchMain()
