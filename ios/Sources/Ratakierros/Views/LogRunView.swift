import SwiftUI

struct LogRunView: View {
    @Environment(AuthStore.self) private var auth
    @Environment(\.dismiss) private var dismiss
    let track: Track
    @State private var secondsText = ""
    @State private var error: String?
    @State private var success = false
    @State private var working = false
    private let api = APIClient()

    var body: some View {
        NavigationStack {
            Form {
                Section(header: Text("Rata")) {
                    Text(track.displayName).font(.headline)
                    if let city = track.city {
                        Text(city).foregroundStyle(.secondary)
                    }
                }
                Section(header: Text("Aika sekunteina (30–600)")) {
                    TextField("esim. 65.4", text: $secondsText)
                        .keyboardType(.decimalPad)
                }
                if let error {
                    Text(error).foregroundStyle(.red)
                }
                if success {
                    Label("Aika kirjattu!", systemImage: "checkmark.circle.fill")
                        .foregroundStyle(.green)
                }
                Section {
                    Button(action: submit) {
                        HStack {
                            if working { ProgressView() }
                            Text("Tallenna")
                        }
                    }
                    .disabled(working || !canSubmit)
                }
            }
            .navigationTitle("Kirjaa aika")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Sulje") { dismiss() }
                }
            }
        }
    }

    private var seconds: Double? {
        let normalized = secondsText.replacingOccurrences(of: ",", with: ".")
        return Double(normalized)
    }

    private var canSubmit: Bool {
        guard let s = seconds else { return false }
        return s >= 30 && s <= 600 && auth.isLoggedIn
    }

    private func submit() {
        guard let s = seconds, let token = auth.token else { return }
        working = true
        error = nil
        success = false
        Task {
            do {
                try await api.logRun(trackId: track.id, seconds: s, token: token)
                success = true
            } catch {
                self.error = (error as? APIError)?.errorDescription ?? error.localizedDescription
            }
            working = false
        }
    }
}
