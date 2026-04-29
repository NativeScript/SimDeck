# Contributing to SimDeck

Thanks for your interest in working on SimDeck. This guide covers how to build
and run the project from a source checkout, package the VS Code extension
locally, preview the documentation site, and cut releases.

For a high-level tour of the codebase — including the boundary between the
Rust server, the Objective-C native bridge, and the browser client — read
[AGENTS.md](AGENTS.md).

## Requirements

- macOS
- Xcode or Command Line Tools
- Node.js 18+
- Rust toolchain (`cargo`)

The npm package ships a prebuilt native binary, so Rust is only required for
contributors building from source.

## Build

The default build covers the native CLI and the browser client:

```sh
npm run build           # alias for build:app (CLI + client)
```

To also build the inspector packages and the `simdeck-test` helper, run:

```sh
npm run build:all       # build:app + build:packages
```

Granular targets:

```sh
npm run build:cli                    # Rust server -> build/simdeck-bin
npm run build:client                 # browser bundle -> client/dist
npm run build:inspectors             # nativescript + react-native inspectors
npm run build:simdeck-test           # simdeck/test subpath export
npm run build:vscode-extension       # alias for package:vscode-extension
```

`scripts/build-cli.sh` builds the Rust server in `server/` and copies the
resulting binary to `build/simdeck-bin`. Set `SIMDECK_BUILD_UNIVERSAL=1` to
produce a fat `arm64+x86_64` binary via `lipo` (the release workflow uses
this); the default is a host-arch build for fast iteration.

## Run from a source checkout

After a successful build, run the CLI directly:

```sh
./build/simdeck list
./build/simdeck daemon start --port 4310
```

Or install the local checkout globally so the `simdeck` command is on your
PATH:

```sh
npm install -g .
```

## Lint, format, and test

```sh
npm run lint           # prettier + clippy + client typecheck
npm run format         # prettier + cargo fmt
npm run test           # cargo test + client unit tests
npm run test:integration:cli
npm run test:integration:js-api
```

The full CI matrix runs as `npm run ci`.

## VS Code extension

Package the local VS Code extension from this checkout:

```sh
npm run package:vscode-extension
```

The shorter aliases `npm run package:vscode` and `npm run package:vsix` do the
same thing. This writes `build/vscode/simdeck-vscode.vsix`.

Install that local package into VS Code:

```sh
npm run install:vscode-extension     # or: npm run install:vscode
```

The install script packages the extension first if the `.vsix` does not exist,
then runs the VS Code CLI with `--install-extension build/vscode/simdeck-vscode.vsix --force`.
If the `code` command is not available, install it from VS Code with
`Shell Command: Install 'code' command in PATH`.

Then run `SimDeck: Open Simulator View` from the Command Palette.

## Package tarballs

For local sanity-checking what the release workflow will produce:

```sh
npm run package:npm     # npm pack -> simdeck-<version>.tgz
npm run package:all     # build:all + VS Code .vsix + npm pack
```

Use `npm publish --dry-run` from a package directory to inspect what would be
published without actually publishing.

## Documentation site

The public docs at [simdeck.nativescript.org](https://simdeck.nativescript.org/)
are built from [docs/](docs/) with VitePress and deployed by
`.github/workflows/docs.yml`. Preview locally:

```sh
npm run docs:dev
npm run docs:build
npm run docs:preview
```

## Codex skill

Install the agent skill from a source checkout with [skills.sh](https://skills.sh/):

```sh
npx skills add NativeScript/SimDeck --skill simdeck -a codex -g
```

The npm postinstall message also prints this command after a global install.

## Releasing

Releases are published from the `Release` GitHub Actions workflow at
[.github/workflows/release.yml](.github/workflows/release.yml). Trigger it
manually from the Actions tab and pick the package, bump type, and dist-tag.

The workflow:

- Bumps the chosen package's version, commits, and tags as `<slug>-v<version>`
- Builds a universal native binary for the root `simdeck` package, codesigns
  with the team's Developer ID Application certificate, and notarizes with
  Apple's notary service
- Publishes npm packages over OIDC trusted publishing (no NPM_TOKEN required)
- Publishes the VS Code extension via `vsce` using a stored PAT
- Creates a GitHub Release with the signed binary attached

Each npm package must have a Trusted Publisher rule configured on
[npmjs.com](https://www.npmjs.com/) before the workflow can publish a new
version of it. The first publish of a brand-new package still needs a one-time
manual `npm publish` (with a token) so the package exists; after that the
trusted-publisher rule takes over.
