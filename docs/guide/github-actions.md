# GitHub Actions

SimDeck can run an iOS Simulator session from a pull request comment. A repository
needs two pieces:

1. A build workflow that uploads a zipped iOS Simulator `.app` artifact.
2. A comment workflow that calls SimDeck's session action when someone
   comments `simdeck run ios` on a pull request.

## Build Workflow

Build your app however your project normally builds a simulator target, then use
the upload action to package and publish the `.app` artifact:

```yaml
name: Build iOS Simulator

on:
  push:
    branches:
      - main
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

The uploaded artifact name defaults to `ios-simulator-app-<commit-sha>`. For pull
requests, the SHA is the PR head commit.

## Comment Workflow

Add a second workflow that delegates the hosted simulator session to SimDeck:

```yaml
name: SimDeck iOS Comment

on:
  issue_comment:
    types:
      - created

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

When triggered, the session action:

- reacts to the command comment immediately;
- installs `simdeck` and `cloudflared` on a single macOS runner;
- starts SimDeck with software encoding and the `tiny` stream profile by default;
- prefers `iPhone 17 Pro`, then falls back to the newest available iPhone simulator;
- restores the CoreSimulator device cache when available;
- posts the first bot comment only after a simulator UDID has been selected,
  mentioning the requester when `command_comment_author` is set;
- downloads the simulator app artifact for the PR head commit, installs it, and
  launches it;
- stops after 30 minutes by default, or earlier if the simulator shuts down.

## Comment Flags

The comment can include lightweight flags:

```text
simdeck run ios no-cache-sim
simdeck run ios latest-device
simdeck run ios small-device
simdeck run ios quality=low
simdeck run ios public-health
```

Supported quality values are `tiny`, `low`, `economy`, `fast`, `smooth`,
`balanced`, `full`, `quality`, and `ci-software`.

## Inputs

The most common session action inputs are:

| Input                    | Default                   | Purpose                                       |
| ------------------------ | ------------------------- | --------------------------------------------- |
| `bundle_id`              | empty                     | Fallback app bundle id to launch.             |
| `build_workflow`         | `build-ios-simulator.yml` | Workflow file that uploads the app artifact.  |
| `command_comment_id`     | empty                     | Comment id to react to immediately.           |
| `command_comment_author` | empty                     | GitHub user to mention when the URL is ready. |
| `artifact_prefix`        | `ios-simulator-app`       | Prefix used for `<prefix>-<sha>` artifacts.   |
| `simdeck_version`        | `latest`                  | npm version or dist-tag to install.           |
| `stream_profile`         | `tiny`                    | Default stream quality profile.               |
| `simulator_name`         | `iPhone 17 Pro`           | Preferred simulator device name.              |
| `keepalive_seconds`      | `1800`                    | Session lifetime after app launch.            |
| `simulator_cache`        | `true`                    | Restore and save CoreSimulator device cache.  |

The caller workflow owns job-level settings such as `runs-on`,
`timeout-minutes`, permissions, and concurrency. Pin `NativeScript/SimDeck` to a
release tag instead of `@main` when you want a stable integration point.
