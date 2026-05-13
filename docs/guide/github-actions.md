# GitHub Actions

SimDeck can post a temporary hosted simulator session from a pull request comment.

You need:

1. A build workflow that uploads a zipped iOS Simulator `.app`.
2. A comment workflow that starts the SimDeck session when someone comments `simdeck run ios`.

## Build Workflow

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

## Comment Workflow

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

## Pull Request Command

```text
simdeck run ios
```

Optional flags:

```text
simdeck run ios no-cache-sim
simdeck run ios latest-device
simdeck run ios small-device
simdeck run ios quality=low
simdeck run ios public-health
```

Supported quality values include `tiny`, `low`, `economy`, `fast`, `smooth`, `balanced`, `full`, `quality`, and `ci-software`.

## Common Inputs

| Input               | Default                   | Purpose                                     |
| ------------------- | ------------------------- | ------------------------------------------- |
| `bundle_id`         | empty                     | Bundle ID to launch                         |
| `build_workflow`    | `build-ios-simulator.yml` | Workflow file that uploads the app artifact |
| `artifact_prefix`   | `ios-simulator-app`       | Artifact prefix                             |
| `simdeck_version`   | `latest`                  | npm version or dist-tag                     |
| `stream_profile`    | `tiny`                    | Default stream quality                      |
| `simulator_name`    | `iPhone 17 Pro`           | Preferred simulator                         |
| `keepalive_seconds` | `1800`                    | Session lifetime after launch               |
| `simulator_cache`   | `true`                    | Restore and save simulator cache            |

## What The Session Does

- Installs SimDeck and tunnel tooling on a macOS runner.
- Picks or creates an iOS Simulator.
- Downloads the app artifact for the PR head commit.
- Installs and launches the app.
- Posts a browser URL back to the pull request.
- Stops after the configured keepalive window.
