import SwiftUI
import CoreLocation

struct ClosestTrackView: View {
    @Environment(AuthStore.self) private var auth
    @State private var location = LocationProvider()
    @State private var track: Track?
    @State private var loading = false
    @State private var error: String?
    @State private var showingLogin = false
    @State private var showingLogRun = false
    @State private var showingLeaderboard = false
    private let api = APIClient()

    var body: some View {
        NavigationStack {
            content
                .navigationTitle("Lähin rata")
                .toolbar {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button {
                            if auth.isLoggedIn {
                                auth.signOut()
                            } else {
                                showingLogin = true
                            }
                        } label: {
                            Image(systemName: auth.isLoggedIn ? "person.fill" : "person")
                        }
                        .accessibilityLabel(auth.isLoggedIn ? "Kirjaudu ulos" : "Kirjaudu sisään")
                    }
                    ToolbarItem(placement: .topBarLeading) {
                        Button {
                            location.request()
                        } label: {
                            Image(systemName: "location.fill")
                        }
                        .accessibilityLabel("Päivitä sijainti")
                    }
                }
                .onAppear { location.request() }
                .onChange(of: location.lastLocation?.latitude) { _, _ in fetch() }
                .onChange(of: location.lastLocation?.longitude) { _, _ in fetch() }
                .sheet(isPresented: $showingLogin) {
                    LoginView().environment(auth)
                }
                .sheet(isPresented: $showingLogRun) {
                    if let track { LogRunView(track: track).environment(auth) }
                }
                .sheet(isPresented: $showingLeaderboard) {
                    if let track { LeaderboardView(track: track).environment(auth) }
                }
        }
    }

    @ViewBuilder
    private var content: some View {
        if loading {
            ProgressView("Haetaan…")
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let error {
            VStack(spacing: 12) {
                Text(error).multilineTextAlignment(.center)
                Button("Yritä uudelleen") { fetch() }
            }
            .padding()
        } else if let track {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    TrackCard(track: track)
                    HStack(spacing: 12) {
                        Button {
                            if auth.isLoggedIn { showingLogRun = true } else { showingLogin = true }
                        } label: {
                            Label("Kirjaa aika", systemImage: "stopwatch")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.borderedProminent)

                        Button {
                            showingLeaderboard = true
                        } label: {
                            Label("Tulokset", systemImage: "list.number")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.bordered)
                    }
                }
                .padding()
            }
        } else if location.status == .denied || location.status == .restricted {
            VStack(spacing: 12) {
                Text("Sijaintilupa tarvitaan lähimmän radan löytämiseen.")
                    .multilineTextAlignment(.center)
                if let url = URL(string: UIApplication.openSettingsURLString) {
                    Link("Avaa asetukset", destination: url)
                }
            }
            .padding()
        } else {
            ProgressView("Haetaan sijaintia…")
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private func fetch() {
        guard let coord = location.lastLocation else { return }
        loading = true
        error = nil
        Task {
            do {
                let list = try await api.tracks(lat: coord.latitude, lon: coord.longitude)
                track = list.first
                if track == nil { error = "Ei ratoja löytynyt." }
            } catch {
                self.error = (error as? APIError)?.errorDescription ?? error.localizedDescription
            }
            loading = false
        }
    }
}

struct TrackCard: View {
    let track: Track
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(track.displayName).font(.title2).bold()
            if let city = track.city {
                Text(city).font(.subheadline).foregroundStyle(.secondary)
            }
            HStack(spacing: 16) {
                if let dist = track.distanceLabel {
                    Label(dist, systemImage: "location")
                }
                if let lanes = track.lanes {
                    Label("\(lanes) rataa", systemImage: "rectangle.split.3x1")
                }
                if let surface = track.surface {
                    Label(surface, systemImage: "square.dashed")
                }
            }
            .font(.footnote)
            .foregroundStyle(.secondary)
            if let r = track.recordLabel {
                Label("Ennätys \(r)", systemImage: "trophy")
                    .font(.footnote)
                    .foregroundStyle(.orange)
            }
        }
        .padding()
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(.secondarySystemBackground))
        .clipShape(RoundedRectangle(cornerRadius: 12))
    }
}
