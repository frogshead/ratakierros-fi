import SwiftUI

struct LoginView: View {
    @Environment(AuthStore.self) private var auth
    @Environment(\.dismiss) private var dismiss
    @State private var mode: Mode = .login
    @State private var email = ""
    @State private var password = ""
    @State private var displayName = ""
    @State private var error: String?
    @State private var working = false
    private let api = APIClient()

    enum Mode { case login, register }

    var body: some View {
        NavigationStack {
            Form {
                Picker("Tila", selection: $mode) {
                    Text("Kirjaudu").tag(Mode.login)
                    Text("Rekisteröidy").tag(Mode.register)
                }
                .pickerStyle(.segmented)

                Section {
                    TextField("Sähköposti", text: $email)
                        .keyboardType(.emailAddress)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    if mode == .register {
                        TextField("Nimimerkki", text: $displayName)
                            .textInputAutocapitalization(.words)
                    }
                    SecureField("Salasana", text: $password)
                }

                if let error {
                    Text(error).foregroundStyle(.red)
                }

                Section {
                    Button(action: submit) {
                        HStack {
                            if working { ProgressView() }
                            Text(mode == .login ? "Kirjaudu" : "Rekisteröidy")
                        }
                    }
                    .disabled(working || !canSubmit)
                }
            }
            .navigationTitle("Tili")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Sulje") { dismiss() }
                }
            }
        }
    }

    private var canSubmit: Bool {
        !email.isEmpty && password.count >= 6 &&
            (mode == .login || !displayName.isEmpty)
    }

    private func submit() {
        working = true
        error = nil
        Task {
            do {
                let resp: AuthResponse
                switch mode {
                case .login:
                    resp = try await api.login(email: email, password: password)
                case .register:
                    resp = try await api.register(email: email, displayName: displayName, password: password)
                }
                auth.setSession(token: resp.token, displayName: resp.displayName)
                dismiss()
            } catch {
                self.error = (error as? APIError)?.errorDescription ?? error.localizedDescription
            }
            working = false
        }
    }
}
