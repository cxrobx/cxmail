import SwiftUI
import UserNotifications

struct NotificationSettings: Decodable {
    let configured: Bool
    let trustedSenders: [String]
    let accounts: [NotificationAccount]?
}

struct NotificationAccount: Decodable, Identifiable {
    let id: String
    let email: String
    let enabled: Bool
}

enum MailNotificationAction: Equatable {
    case read(MessageRef)
    case openSession(MessageRef)
}

@MainActor @Observable
final class PushNotifications: NSObject, UNUserNotificationCenterDelegate {
    static let shared = PushNotifications()
    nonisolated static let mailCategory = "CXMAIL_SMART_MAIL"
    nonisolated static let openSessionIdentifier = "CXMAIL_OPEN_SESSION"
    var status = "Notifications are off"
    var settings: NotificationSettings?
    private(set) var accounts: [NotificationAccount] = []
    private(set) var updatingAccounts = false
    var error: String?
    var pendingAction: MailNotificationAction?
    private(set) var offersSessionAction: Bool
    private var api: APIClient?
    private var token: String?
    private let defaults: UserDefaults
    private var syncTask: Task<Void, Never>?
    private var accountSettingsVersion = 0
    init(api: APIClient? = nil, defaults: UserDefaults = .standard) {
        self.api = api; self.defaults = defaults
        offersSessionAction = defaults.bool(forKey: "pushOffersSessionAction")
        super.init()
    }
    private var device: String {
        if let saved = defaults.string(forKey: "pushDevice") { return saved }
        let id = UUID().uuidString; defaults.set(id, forKey: "pushDevice"); return id
    }
    var enabled: Bool { defaults.bool(forKey: "pushEnabled") }

    func setSessionAction(_ enabled: Bool) {
        offersSessionAction = enabled
        defaults.set(enabled, forKey: "pushOffersSessionAction")
        registerCategories()
    }

    func registerCategories() {
        let center = UNUserNotificationCenter.current()
        guard offersSessionAction else {
            center.setNotificationCategories([])
            return
        }
        let action = UNNotificationAction(identifier: Self.openSessionIdentifier,
            title: "Open in Session", options: [.foreground, .authenticationRequired])
        center.setNotificationCategories([
            UNNotificationCategory(identifier: Self.mailCategory, actions: [action], intentIdentifiers: [], options: [])
        ])
    }

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
        guard let api, !api.fixture, !updatingAccounts else { return }
        let version = accountSettingsVersion
        do {
            let response: NotificationSettings = try await api.get("/api/notifications")
            guard version == accountSettingsVersion, !updatingAccounts else { return }
            settings = response
            accounts = response.accounts ?? []
            error = nil
        } catch {
            guard version == accountSettingsVersion else { return }
            self.error = error.localizedDescription
        }
    }

    func setAccount(_ account: NotificationAccount, enabled: Bool) async {
        guard let api, !api.fixture, !updatingAccounts else { return }
        updatingAccounts = true
        accountSettingsVersion += 1
        defer { updatingAccounts = false }
        do {
            let response: NotificationSettings = try await api.post("/api/notifications/account",
                body: ["account": account.id, "enabled": enabled])
            accounts = response.accounts ?? []
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
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
    nonisolated static func action(identifier: String, category: String, ref: MessageRef,
                                   sessionActionEnabled: Bool) -> MailNotificationAction? {
        switch identifier {
        case UNNotificationDefaultActionIdentifier:
            return .read(ref)
        case openSessionIdentifier where category == mailCategory && sessionActionEnabled:
            return .openSession(ref)
        default:
            return nil
        }
    }
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter,
        willPresent notification: UNNotification, withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        completionHandler([.banner, .sound, .list])
    }
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse, withCompletionHandler completionHandler: @escaping () -> Void) {
        let content = response.notification.request.content
        if let ref = Self.messageReference(content.userInfo) {
            let identifier = response.actionIdentifier
            let category = content.categoryIdentifier
            Task { @MainActor in
                if let action = Self.action(identifier: identifier, category: category, ref: ref,
                                            sessionActionEnabled: self.enabled && self.offersSessionAction) {
                    self.pendingAction = action
                }
                completionHandler()
            }
        } else {
            completionHandler()
        }
    }
}

final class PushAppDelegate: NSObject, UIApplicationDelegate {
    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("--uitesting") { return true }
        #endif
        UNUserNotificationCenter.current().delegate = PushNotifications.shared
        PushNotifications.shared.registerCategories()
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
    let fixture: Bool
    @State private var push = PushNotifications.shared
    @State private var sender = ""
    @State private var fixtureSessionAction = false
    var body: some View {
        Section {
            Text(push.status)
            Button("Enable smart notifications") { Task { await push.enable() } }
            Button("Turn off notifications") { Task { await push.disable() } }
            Toggle("Open in Session action", isOn: fixture ? $fixtureSessionAction : Binding(
                get: { push.offersSessionAction }, set: { push.setSessionAction($0) }))
            if fixture ? fixtureSessionAction : push.offersSessionAction {
                Text("Press and hold a new alert to start a T3 session for that email.")
                    .font(.footnote).foregroundStyle(.secondary)
            }
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
            Text("Alerts show the sender, subject, and a short preview. Lock Screen previews follow your iOS notification settings. Alerts cover trusted senders and mail Luna identifies as important.")
        }
        if !push.accounts.isEmpty {
            Section {
                ForEach(push.accounts) { account in
                    Toggle(account.email, isOn: Binding(
                        get: { account.enabled },
                        set: { enabled in Task { await push.setAccount(account, enabled: enabled) } }))
                }
                .disabled(push.updatingAccounts)
            } header: { Text("Notification accounts") } footer: {
                Text("Only selected accounts can send alerts, including alerts from trusted senders. This applies to all your CXMail devices.")
            }
        }
    }
}
