import SwiftUI
import UserNotifications

struct NotificationSettings: Decodable {
    let configured: Bool
    let trustedSenders: [String]
}

@MainActor @Observable
final class PushNotifications: NSObject, UNUserNotificationCenterDelegate {
    static let shared = PushNotifications()
    var status = "Notifications are off"
    var settings: NotificationSettings?
    var error: String?
    var selection: MessageRef?
    private var api: APIClient?
    private var token: String?
    private let defaults: UserDefaults
    private var syncTask: Task<Void, Never>?
    init(api: APIClient? = nil, defaults: UserDefaults = .standard) {
        self.api = api; self.defaults = defaults
        super.init()
    }
    private var device: String {
        if let saved = defaults.string(forKey: "pushDevice") { return saved }
        let id = UUID().uuidString; defaults.set(id, forKey: "pushDevice"); return id
    }
    var enabled: Bool { defaults.bool(forKey: "pushEnabled") }

    func connect(_ api: APIClient) async {
        guard !api.fixture else { return }
        self.api = api
        UNUserNotificationCenter.current().delegate = self
        await refresh()
        let permission = await UNUserNotificationCenter.current().notificationSettings()
        if !enabled {
            if permission.authorizationStatus == .authorized || permission.authorizationStatus == .provisional {
                UIApplication.shared.registerForRemoteNotifications()
            }
            await sync()
        }
        if enabled {
            if permission.authorizationStatus == .authorized || permission.authorizationStatus == .provisional {
                UIApplication.shared.registerForRemoteNotifications()
                await sync()
            } else {
                // Stop the server on the next foreground after permission revocation.
                defaults.set(false, forKey: "pushEnabled")
                await sync()
                status = "Notifications are disabled in iOS Settings"
            }
        }
    }
    func refresh() async {
        guard let api, !api.fixture else { return }
        do { settings = try await api.get("/api/notifications"); error = nil }
        catch { self.error = error.localizedDescription }
    }
    func enable() async {
        guard let api, !api.fixture else { return }
        do {
            let allowed = try await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge])
            guard allowed else { status = "Allow notifications in iOS Settings"; return }
            defaults.set(true, forKey: "pushEnabled")
            status = "Registering with Apple…"
            UIApplication.shared.registerForRemoteNotifications()
        } catch { self.error = error.localizedDescription }
    }
    func disable() async {
        guard let api, !api.fixture else { return }
        defaults.set(false, forKey: "pushEnabled")
        status = "Turning off notifications…"
        await sync()
    }
    func registered(_ data: Data) async {
        token = data.map { String(format: "%02x", $0) }.joined()
        await sync()
    }
    private func sync() async {
        // Registration, token refresh and opt-out must reach the mini in order.
        // Canceling a URLSession request cannot undo an already-arriving write.
        let previous = syncTask
        let task = Task { @MainActor in
            await previous?.value
            await self.performSync()
        }
        syncTask = task
        await task.value
    }
    private func performSync() async {
        guard let api, !api.fixture else { return }
        if !enabled {
            do {
                settings = try await api.post("/api/notifications/disable", body: ["device": device])
                status = "Notifications are off"; error = nil
            } catch { self.error = error.localizedDescription }
            return
        }
        guard let token else { return }
        do {
            settings = try await api.post("/api/notifications/device", body: [
                "device": device, "token": token, "environment": Self.environment, "enabled": enabled])
            status = !enabled ? "Notifications are off" : settings?.configured == true
                ? "Smart notifications are on" : "Waiting for Apple Push setup on the mini"
            error = nil
        } catch { self.error = error.localizedDescription }
    }
    func trust(_ email: String, trusted: Bool) async {
        guard let api, !api.fixture else { return }
        do {
            settings = try await api.post("/api/notifications/trust", body: ["email": email, "trusted": trusted])
            error = nil
        } catch { self.error = error.localizedDescription }
    }
    // A development-signed Release install still uses Apple's sandbox.
    static var environment: String {
        guard let url = Bundle.main.url(forResource: "embedded", withExtension: "mobileprovision"),
              let data = try? Data(contentsOf: url), let text = String(data: data, encoding: .isoLatin1),
              let range = text.range(of: "<key>aps-environment</key>") else { return "production" }
        return text[range.upperBound...].prefix(80).contains("development") ? "development" : "production"
    }
    nonisolated static func messageReference(_ info: [AnyHashable: Any]) -> MessageRef? {
        guard let account = info["account"] as? String, !account.isEmpty,
              let folder = info["folder"] as? String, !folder.isEmpty,
              let uid = info["uid"] as? Int, uid > 0, uid <= Int(UInt32.max) else { return nil }
        return MessageRef(account: account, folder: folder, uid: uid)
    }
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter,
        willPresent notification: UNNotification, withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        completionHandler([.banner, .sound, .list])
    }
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse, withCompletionHandler completionHandler: @escaping () -> Void) {
        let info = response.notification.request.content.userInfo
        if let ref = Self.messageReference(info) {
            Task { @MainActor in self.selection = ref }
        }
        completionHandler()
    }
}

final class PushAppDelegate: NSObject, UIApplicationDelegate {
    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        UNUserNotificationCenter.current().delegate = PushNotifications.shared
        return true
    }
    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        Task { await PushNotifications.shared.registered(deviceToken) }
    }
    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) {
        PushNotifications.shared.error = "Apple Push registration failed: \(error.localizedDescription)"
    }
}

struct NotificationSettingsSection: View {
    @State private var push = PushNotifications.shared
    @State private var sender = ""
    var body: some View {
        Section {
            Text(push.status)
            Button("Enable smart notifications") { Task { await push.enable() } }
            Button("Turn off notifications") { Task { await push.disable() } }
            ForEach(push.settings?.trustedSenders ?? [], id: \.self) { email in
                HStack {
                    Text(email)
                    Spacer()
                    Button("Remove", role: .destructive) { Task { await push.trust(email, trusted: false) } }
                }
            }
            TextField("Trusted sender email", text: $sender)
                .textContentType(.emailAddress).keyboardType(.emailAddress).textInputAutocapitalization(.never).autocorrectionDisabled()
            Button("Trust sender for notifications") {
                Task { await push.trust(sender.trimmingCharacters(in: .whitespacesAndNewlines), trusted: true); if push.error == nil { sender = "" } }
            }.disabled(sender.isEmpty)
            if let error = push.error { Text(error).foregroundStyle(.red) }
        } header: { Text("Smart notifications") } footer: {
            Text("New mail from trusted addresses, or mail Luna identifies as needing you or time sensitive. Known junk and read mail are skipped. Alerts hide sender and subject on the lock screen. The mini must be awake and syncing. Notification trust is separate from image loading.")
        }
    }
}
