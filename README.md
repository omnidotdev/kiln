# Kiln

**Source code, fired into containers.**

Kiln is a zero-config build tool that turns a source repository into a container
image. Point it at a project and it detects the language, plans the build, and
produces a ready-to-run image, with no Dockerfile to write and no configuration
to maintain.

Unlike builders that emit an opaque, proprietary build graph, Kiln generates a
**plain, readable Dockerfile** that you own. Inspect it, commit it, tweak it, or
hand it to any Docker- or BuildKit-compatible toolchain. There is no lock-in:
the output is a standard artifact that works everywhere.

## Why Kiln

- **Zero config**: detects your language and framework and builds a sensible,
  production-ready image out of the box
- **Transparent output**: emits a real Dockerfile you can read, version, and
  customize, not a black box
- **Portable**: the generated Dockerfile and image work with any registry and
  any Docker- or BuildKit-based platform
- **Fast and self-contained**: a single Rust binary with no runtime
  dependencies
- **Multi-stage by default**: slim final images that ship only what runs, not
  your build toolchain
- **Open source**: Apache-2.0 licensed

## Supported languages

C++, Deno, .NET, Elixir, Gleam, Go, Java, Node.js, PHP, Python, Ruby, Rust,
static sites, and shell projects. More providers are easy to add.

## Install

| Platform | Channel | Command / Link |
| --- | --- | --- |
| All | [GitHub Releases](https://github.com/omnidotdev/kiln/releases) | Download the prebuilt binary for your platform |
| macOS / Linux | [Homebrew](https://github.com/omnidotdev/homebrew-tap/blob/master/Formula/kiln.rb) | `brew install omnidotdev/tap/kiln` |
| Arch Linux | [AUR](https://aur.archlinux.org/packages/omnidotdev-kiln) / [AUR (bin)](https://aur.archlinux.org/packages/omnidotdev-kiln-bin) | `paru -S omnidotdev-kiln` or `paru -S omnidotdev-kiln-bin` |

### Build from source

Requires a recent Rust toolchain:

```bash
cargo install --path crates/kiln-cli
```

This installs the `kiln` binary.

`kiln detect` and `kiln plan` run anywhere; `kiln build` additionally needs a
reachable [BuildKit](https://github.com/moby/buildkit) daemon (set its address
with `BUILDKIT_HOST` or `--buildkit-addr`).

## Usage

```bash
# Detect the language of a project
kiln detect --path ./my-app

# Summarize what Kiln detects (provider, base images, start command, port)
kiln info --path ./my-app

# Generate a build plan as JSON (what Kiln would do, without building)
kiln plan --path ./my-app

# Build a container image (requires a reachable BuildKit daemon; set its
# address with BUILDKIT_HOST or --buildkit-addr)
kiln build --path ./my-app --dest registry.example/my-app:latest

# Emit the config-file JSON schema, or a shell completion script
kiln schema
kiln completion zsh
```

`kiln plan` is a good way to see exactly what Kiln intends to do before it runs,
and the generated Dockerfile is yours to keep and edit. `kiln build` can also
build straight from a Git source with `--source <url> --ref <sha>`.

## Configuration

Kiln is zero-config by default. When detection cannot infer something, drop a
`kiln.json` (or `kiln.toml`) in the project root to pin it. Every field is
optional.

```json
{
  "provider": "node",
  "version": "22",
  "package_manager": "pnpm",
  "install_command": "pnpm install --prod",
  "build_command": "pnpm build",
  "start_command": "node dist/main.js",
  "port": 8080,
  "env": { "NODE_ENV": "production" },

  "build_apt_packages": ["libpq-dev", "pkg-config"],
  "deploy_apt_packages": ["ca-certificates"],
  "build_image": "node:22-bookworm",
  "runtime_image": "gcr.io/distroless/nodejs22-debian12",
  "paths": ["/opt/bin"],
  "secrets": ["NPM_TOKEN"],
  "pre_build": ["protoc --version"],
  "post_build": ["node scripts/postbuild.js"]
}
```

- **Versions** also come from ecosystem version files, which `version`
  overrides: `.nvmrc` / `.node-version`, the `go` directive in `go.mod`,
  `.python-version`, `.ruby-version`, `rust-toolchain.toml`, and the shared
  `.tool-versions` (asdf/mise) and mise config (`mise.toml` / `.mise.toml`).
  Applies to all fourteen providers.
- **apt packages** install into the build stage (`build_apt_packages`) or the
  final image (`deploy_apt_packages`).
- **build_image / runtime_image** swap the base images; **paths** prepend
  directories to `PATH` in the runtime.
- **secrets** are `BuildKit` secret ids mounted on build-stage commands (and
  forwarded to the build), never written into a layer.
- **pre_build / post_build** run extra commands before and after the provider's
  own build steps.

**Environment variables.** A platform can set the same options without a config
file: `KILN_PROVIDER`, `KILN_VERSION`, `KILN_PACKAGE_MANAGER`, `KILN_INSTALL_CMD`,
`KILN_BUILD_CMD`, `KILN_START_CMD`, `KILN_PORT`, `KILN_BUILD_APT_PACKAGES`,
`KILN_DEPLOY_APT_PACKAGES`, `KILN_BUILD_IMAGE`, `KILN_RUNTIME_IMAGE`,
`KILN_PATHS`, and `KILN_SECRETS`. Precedence is **CLI flag > environment
variable > config file > auto-detection**. Run `kiln schema` for the full schema.

## How it works

Kiln has two parts:

- **`kiln-core`**: language detection, build planning, and Dockerfile
  generation. Each supported language is a self-contained provider.
- **`kiln-cli`**: the `kiln` command-line interface, which drives detection,
  planning, and building (via [BuildKit](https://github.com/moby/buildkit)).

Detection inspects a project's files (manifests, lockfiles, entrypoints) to
identify the language and framework, the provider produces a build plan, and the
plan is rendered into a multi-stage Dockerfile and built into an image.

## Contributing

Contributions are welcome, especially new language providers. A provider lives in
`crates/kiln-core/src/providers/` and implements detection plus build-plan
generation for one ecosystem.

## License

Licensed under the [Apache License 2.0](./LICENSE.md).
