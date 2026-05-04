# Ratakierros Android

Native Android app (Kotlin / Jetpack Compose). MVP scope:

- Show the closest athletics track using the device's location.
- Register / log in with email + password.
- Log a run time on the closest track.
- View the per-track leaderboard (top 10) and personal best.

Talks to the API at `https://ratakierros.fi` (overridable via the
`API_BASE` `BuildConfig` field in `app/build.gradle.kts`).

## Build locally

Requires:

- JDK 17 (Temurin recommended)
- Android SDK 34 platform + build-tools (set `ANDROID_HOME`)
- Gradle 8.9 — *or* Android Studio (which bundles its own Gradle)

If you don't have the Gradle wrapper jar yet (the repo intentionally
doesn't ship it — see "About the wrapper" below), bootstrap it once:

```sh
cd android
gradle wrapper --gradle-version=8.9 --distribution-type=bin
```

Then:

```sh
./gradlew assembleDebug          # builds an APK
./gradlew installDebug           # installs on a connected device
```

Or open `android/` in Android Studio and run.

## About the wrapper

`gradle-wrapper.jar` is not committed because it's a binary that
requires a working Gradle install to regenerate, and the project
maintainer did not have Gradle locally when scaffolding the app.

CI gets around this by using
[`gradle/actions/setup-gradle`](https://github.com/gradle/actions),
which provides Gradle and the wrapper. Run `gradle wrapper` once
locally (with any Gradle install) to materialise it for offline use.

## Layout

```
android/
├── settings.gradle.kts
├── build.gradle.kts
├── gradle.properties
├── gradle/wrapper/gradle-wrapper.properties
└── app/
    ├── build.gradle.kts
    └── src/main/
        ├── AndroidManifest.xml
        ├── kotlin/fi/ratakierros/
        │   ├── MainActivity.kt        # NavHost, AppContainer
        │   ├── model/{Track,Run,AuthResponse}.kt
        │   ├── network/ApiClient.kt   # Retrofit + kotlinx-serialization
        │   ├── auth/{AuthRepository,LoginScreen}.kt
        │   ├── location/LocationProvider.kt
        │   └── ui/{ClosestTrackScreen,LogRunScreen,LeaderboardScreen}.kt
        └── res/values{,-fi}/strings.xml
```

`AuthRepository` persists the JWT in `EncryptedSharedPreferences`
(file `ratakierros_auth`).
