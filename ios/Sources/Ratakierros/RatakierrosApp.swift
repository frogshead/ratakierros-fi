import SwiftUI

@main
struct RatakierrosApp: App {
    @State private var auth = AuthStore()

    var body: some Scene {
        WindowGroup {
            ClosestTrackView()
                .environment(auth)
        }
    }
}
