import SwiftUI

struct LeaderboardView: View {
    @Environment(AuthStore.self) private var auth
    @Environment(\.dismiss) private var dismiss
    let track: Track
    @State private var data: RecordsResponse?
    @State private var loading = true
    @State private var error: String?
    private let api = APIClient()

    var body: some View {
        NavigationStack {
            Group {
                if loading {
                    ProgressView()
                } else if let error {
                    VStack(spacing: 12) {
                        Text(error).multilineTextAlignment(.center)
                        Button("Yritä uudelleen") { load() }
                    }
                } else if let data {
                    List {
                        if let pb = data.personalBest {
                            Section {
                                Label("Oma ennätys: \(formatSeconds(pb))", systemImage: "person.crop.circle.badge.checkmark")
                                    .foregroundStyle(.blue)
                            }
                        }
                        Section(header: Text("Top 10")) {
                            if data.records.isEmpty {
                                Text("Ei vielä aikoja.").foregroundStyle(.secondary)
                            }
                            ForEach(data.records) { rec in
                                HStack {
                                    Text(rankLabel(rec.rank))
                                        .frame(width: 36, alignment: .leading)
                                    VStack(alignment: .leading) {
                                        Text(rec.displayName)
                                        Text(rec.loggedAt.prefix(10))
                                            .font(.caption2)
                                            .foregroundStyle(.secondary)
                                    }
                                    Spacer()
                                    Text(formatSeconds(rec.timeSeconds))
                                        .monospacedDigit()
                                        .bold()
                                }
                            }
                        }
                    }
                }
            }
            .navigationTitle(track.displayName)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Sulje") { dismiss() }
                }
            }
            .onAppear { load() }
        }
    }

    private func rankLabel(_ r: Int) -> String {
        switch r {
        case 1: return "🥇"
        case 2: return "🥈"
        case 3: return "🥉"
        default: return "\(r)."
        }
    }

    private func load() {
        loading = true
        error = nil
        Task {
            do {
                data = try await api.records(trackId: track.id, token: auth.token)
            } catch {
                self.error = (error as? APIError)?.errorDescription ?? error.localizedDescription
            }
            loading = false
        }
    }
}
