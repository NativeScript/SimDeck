# GitHub Actions

SimDeck can post a temporary hosted simulator or emulator session from a pull
request comment.

You need:

1. A build workflow that uploads a zipped iOS Simulator `.app` or Android APK.
2. A comment workflow that starts the SimDeck session when someone comments `simdeck run ios` or `simdeck run android`.

## iOS Build Workflow

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
        uses: NativeScript/SimDeck/actions/upload-ios-simulator-app@main
        with:
          app-glob: platforms/ios/build/**/*-iphonesimulator/*.app
```

Pin the action to a release tag when you want a stable integration point.

## Android Build Workflow

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
        uses: NativeScript/SimDeck/actions/upload-android-apk@main
        with:
          apk-glob: app/build/outputs/apk/debug/*.apk
```

## iOS Comment Workflow

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
        uses: NativeScript/SimDeck/actions/run-ios-comment-session@main
        with:
          pr_number: ${{ github.event.issue.number }}
          command: ${{ github.event.comment.body }}
          command_comment_id: ${{ github.event.comment.id }}
          command_comment_author: ${{ github.event.comment.user.login }}
          build_workflow: build-ios-simulator.yml
          bundle_id: com.example.app
```

## Android Comment Workflow

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
        uses: NativeScript/SimDeck/actions/run-android-comment-session@main
        with:
          pr_number: ${{ github.event.issue.number }}
          command: ${{ github.event.comment.body }}
          command_comment_id: ${{ github.event.comment.id }}
          command_comment_author: ${{ github.event.comment.user.login }}
          build_workflow: build-android-apk.yml
          package_name: com.example.app
```

## Pull Request Commands

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

## Common Inputs

| Input               | Default                             | Purpose                                     |
| ------------------- | ----------------------------------- | ------------------------------------------- |
| `bundle_id`         | empty                               | Bundle ID to launch                         |
| `package_name`      | empty                               | Android package name to launch              |
| `build_workflow`    | `build-ios-simulator.yml`           | Workflow file that uploads the app artifact |
| `artifact_prefix`   | `ios-simulator-app` / `android-apk` | Artifact prefix                             |
| `simdeck_version`   | `latest`                            | npm version or dist-tag                     |
| `stream_profile`    | `tiny`                              | Default stream quality                      |
| `simulator_name`    | `iPhone 17 Pro`                     | Preferred simulator                         |
| `avd_name`          | `SimDeck_Pixel_CI`                  | Preferred Android emulator                  |
| `keepalive_seconds` | `1800`                              | Session lifetime after launch               |
| `simulator_cache`   | `true`                              | Restore and save simulator cache            |

## What The Session Does

- Installs SimDeck and tunnel tooling on a macOS runner.
- Picks or creates an iOS Simulator or Android emulator.
- Downloads the app artifact for the PR head commit.
- Installs and launches the app.
- Posts a browser URL back to the pull request after the simulator or emulator is booted.
- Stops after the configured keepalive window.
