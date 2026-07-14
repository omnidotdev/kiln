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

- **Zero config** &mdash; detects your language and framework and builds a sensible,
  production-ready image out of the box
- **Transparent output** &mdash; emits a real Dockerfile you can read, version, and
  customize, not a black box
- **Portable** &mdash; the generated Dockerfile and image work with any registry and
  any Docker- or BuildKit-based platform
- **Fast and self-contained** &mdash; a single Rust binary with no runtime
  dependencies
- **Multi-stage by default** &mdash; slim final images that ship only what runs, not
  your build toolchain
- **Open source** &mdash; Apache-2.0 licensed

## Supported languages

C++, Deno, .NET, Elixir, Gleam, Go, Java, Node.js, PHP, Python, Ruby, Rust,
static sites, and shell projects. More providers are easy to add.

## Install

Build from source (requires a recent Rust toolchain):

```bash
cargo install --path crates/kiln-cli
```

This installs the `kiln` binary.

## Usage

```bash
# Detect the language of a project
kiln detect --path ./my-app

# Generate a build plan as JSON (what Kiln would do, without building)
kiln plan --path ./my-app

# Build a container image (requires a reachable BuildKit daemon; set its
# address with BUILDKIT_HOST or --buildkit-addr)
kiln build --path ./my-app --dest registry.example/my-app:latest
```

`kiln plan` is a good way to see exactly what Kiln intends to do before it runs,
and the generated Dockerfile is yours to keep and edit. `kiln build` can also
build straight from a Git source with `--source <url> --ref <sha>`.

## How it works

Kiln has two parts:

- **`kiln-core`** &mdash; language detection, build planning, and Dockerfile
  generation. Each supported language is a self-contained provider.
- **`kiln-cli`** &mdash; the `kiln` command-line interface, which drives detection,
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
