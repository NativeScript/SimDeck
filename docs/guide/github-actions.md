# GitHub Actions

SimDeck can post a temporary hosted simulator or emulator session from a pull
request comment.

You need:

1. A build workflow that uploads a zipped iOS Simulator `.app` or Android APK.
2. A comment workflow that starts the SimDeck session when someone comments `simdeck run ios` or `simdeck run android`.

## iOS build workflow

```yaml
name: Build iOS Simulator

on:
  push:
    branches: [main]
  pull_request:

permissions:
  contents: read

jobs:
  build:
    runs-on: macos-26

    steps:
      - uses: actions/checkout@v5

      - name: Select Xcode
        run: sudo xcode-select -s /Applications/Xcode.app/Contents/Developer

      - name: Build simulator app
        run: npm run build:ios:simulator

      - name: Upload simulator app for SimDeck
        uses: NativeScript/SimDeck/actions/upload-ios-simulator-app@v0.1
        with:
          app-glob: platforms/ios/build/**/*-iphonesimulator/*.app
```

## Android build workflow

```yaml
name: Build Android APK

on:
  push:
    branches: [main]
  pull_request:

permissions:
  contents: read

jobs:
  build:
    runs-on: macos-26

    steps:
      - uses: actions/checkout@v5

      - name: Build Android APK
        run: ./gradlew assembleDebug

      - name: Upload APK for SimDeck
        uses: NativeScript/SimDeck/actions/upload-android-apk@v0.1
        with:
          apk-glob: app/build/outputs/apk/debug/*.apk
```

## iOS comment workflow

```yaml
name: SimDeck iOS Comment

on:
  issue_comment:
    types: [created]

permissions:
  actions: read
  contents: read
  issues: write
  pull-requests: write

concurrency:
  group: simdeck-ios-pr-${{ github.event.issue.number }}
  cancel-in-progress: true

jobs:
  simdeck-ios:
    if: github.event.issue.pull_request && startsWith(github.event.comment.body, 'simdeck run ios')
    runs-on: macos-26
    timeout-minutes: 35

    steps:
      - name: Run PR app in SimDeck
        uses: NativeScript/SimDeck/actions/run-ios-comment-session@v0.1
        with:
          pr_number: ${{ github.event.issue.number }}
          command: ${{ github.event.comment.body }}
          command_comment_id: ${{ github.event.comment.id }}
          command_comment_author: ${{ github.event.comment.user.login }}
          build_workflow: build-ios-simulator.yml
          bundle_id: com.example.app
          session_password: ${{ secrets.SIMDECK_PASSWORD }}
```

## Android comment workflow

```yaml
name: SimDeck Android Comment

on:
  issue_comment:
    types: [created]

permissions:
  actions: read
  contents: read
  issues: write
  pull-requests: write

concurrency:
  group: simdeck-android-pr-${{ github.event.issue.number }}
  cancel-in-progress: true

jobs:
  simdeck-android:
    if: github.event.issue.pull_request && startsWith(github.event.comment.body, 'simdeck run android')
    runs-on: macos-26
    timeout-minutes: 35

    steps:
      - name: Run PR Android app in SimDeck
        uses: NativeScript/SimDeck/actions/run-android-comment-session@v0.1
        with:
          pr_number: ${{ github.event.issue.number }}
          command: ${{ github.event.comment.body }}
          command_comment_id: ${{ github.event.comment.id }}
          command_comment_author: ${{ github.event.comment.user.login }}
          build_workflow: build-android-apk.yml
          package_name: com.example.app
          session_password: ${{ secrets.SIMDECK_PASSWORD }}
```

## Version pins

Use a SimDeck release tag instead of `@main` in project workflows. The examples
above pin to `@v0.1`, which follows the latest compatible `0.1.x` release.

Use a tighter pin when reproducibility matters more than automatic patch
updates:

```yaml
uses: NativeScript/SimDeck/actions/run-ios-comment-session@v0.1.22
```

For the strongest supply-chain pin, use a full commit SHA. Avoid `@main` outside
of local experiments because it can change without a release.

## Pull request commands

```text
simdeck run ios
simdeck run android
```

Optional iOS flags:

```text
simdeck run ios no-cache-sim
simdeck run ios latest-device
simdeck run ios small-device
simdeck run ios quality=low
simdeck run ios public-health
```

Optional Android flags:

```text
simdeck run android quality=low
simdeck run android public-health
```

Supported quality values include `tiny`, `low`, `economy`, `fast`, `smooth`, `balanced`, `full`, `quality`, and `ci-software`.

## Common inputs

| Input                   | Default                             | Purpose                                        |
| ----------------------- | ----------------------------------- | ---------------------------------------------- |
| `bundle_id`             | empty                               | Bundle ID to launch                            |
| `package_name`          | empty                               | Android package name to launch                 |
| `build_workflow`        | `build-ios-simulator.yml`           | Workflow file that uploads the app artifact    |
| `artifact_prefix`       | `ios-simulator-app` / `android-apk` | Artifact prefix                                |
| `simdeck_version`       | `latest`                            | npm version or dist-tag                        |
| `stream_profile`        | `tiny`                              | Default stream quality                         |
| `simulator_name`        | `iPhone 17 Pro`                     | Preferred simulator                            |
| `avd_name`              | `SimDeck_Pixel_CI`                  | Preferred Android emulator                     |
| `android_emulator_args` | empty                               | Extra emulator startup arguments, one per line |
| `keepalive_seconds`     | `1800`                              | Session lifetime after launch                  |
| `simulator_cache`       | `true`                              | Restore and save simulator cache               |
| `proxy_links`           | `true`                              | Post SimDeck CI proxy links                    |
| `ci_proxy_url`          | `https://ci.simdeck.sh`             | Optional SimDeck CI proxy URL                  |
| `session_password`      | empty                               | Optional password for proxy-gated sessions     |

## Password-protected links

Set a repository secret such as `SIMDECK_PASSWORD` and pass it as
`session_password`. The action posts a SimDeck proxy link instead of the raw
Cloudflare Tunnel URL by default. When a password is configured, the daemon
token is encrypted into the proxy payload, so decoding the URL alone does not
grant simulator access. Set `proxy_links: "false"` to post raw tunnel links.

## What the session does

- Installs SimDeck and tunnel tooling on a macOS runner.
- Picks or creates an iOS Simulator or Android emulator.
- Downloads the app artifact for the PR head commit.
- Installs and launches the app.
- Posts a browser URL back to the pull request after the simulator or emulator is booted. The iOS action waits until the public URL can load the selected booted simulator from `/api/simulators`.
- Stops after the configured keepalive window.
