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

## Run on a physical device

### 1. Enable developer options

On the phone: **Settings → About phone → tap "Build number" 7 times** to unlock
Developer options. Then go to **Settings → Developer options** and turn on
**USB debugging**.

### 2. Connect and verify

```sh
adb devices          # should list your device as "device" (not "unauthorized")
```

If it shows `unauthorized`, accept the RSA fingerprint prompt that appears on the
phone screen.

### 3. Install and launch

```sh
export JAVA_HOME=$HOME/jdk17
export ANDROID_HOME=$HOME/Android/Sdk
export PATH=$PATH:$ANDROID_HOME/platform-tools

# build + install in one step
./gradlew installDebug

# or install a pre-built APK directly
adb install app/build/outputs/apk/debug/app-debug.apk
```

The app starts automatically after `installDebug`. To launch it again later:

```sh
adb shell am start -n fi.ratakierros/.MainActivity
```

### 4. View logs

```sh
adb logcat -s RatakierrosApp          # filter by app tag
# or show everything (noisy):
adb logcat | grep -i ratakierros
```

In Android Studio: open **Logcat** (View → Tool Windows → Logcat) and filter by
package `fi.ratakierros`.

### 5. Grant location permission on first launch

The app requests `ACCESS_FINE_LOCATION` at runtime. Tap **Allow** when the
system dialog appears. If you dismissed it, re-grant via:
**Settings → Apps → Ratakierros → Permissions → Location → Allow all the time** (or
"While using").

---

## Run on an emulator

### 1. Install the emulator and a system image

```sh
# install emulator + a Google APIs x86_64 image for API 34
$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager \
  "emulator" \
  "system-images;android-34;google_apis;x86_64"
```

> The download is ~1 GB. `google_apis` includes the Play Services needed for the
> FusedLocationProvider used by the app.

### 2. Create an AVD (virtual device)

```sh
$ANDROID_HOME/cmdline-tools/latest/bin/avdmanager create avd \
  --name Pixel8_API34 \
  --package "system-images;android-34;google_apis;x86_64" \
  --device "pixel_8"
```

### 3. Start the emulator

```sh
$ANDROID_HOME/emulator/emulator -avd Pixel8_API34 &
# wait for it to boot, then verify:
adb devices
```

### 4. Set a mock location

The emulator has no real GPS. In Android Studio's **Extended Controls** panel
(the `...` button in the emulator toolbar) go to **Location** and enter
coordinates — for example, Helsinki city centre:

| Field | Value |
|-------|-------|
| Longitude | 24.9384 |
| Latitude | 60.1699 |

Then click **Send**. The app's location request will receive those coordinates.

From the command line:

```sh
adb emu geo fix 24.9384 60.1699
```

### 5. Install and run

Same as the physical device flow:

```sh
./gradlew installDebug
```

---

## Debug with Android Studio

1. Open the `android/` folder as a project in Android Studio.
2. Select the run target (physical device or AVD) in the toolbar dropdown.
3. Click **Run** (▶) for a normal run, or **Debug** (🐛) to attach the debugger.
4. Set breakpoints by clicking the gutter next to any Kotlin line.
5. Use **Logcat** (bottom panel) to see log output filtered to `fi.ratakierros`.

The **Network Inspector** (View → Tool Windows → App Inspection → Network
Inspector) shows the Retrofit calls to `ratakierros.fi` in real time.

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
