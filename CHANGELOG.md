# kiln

## 0.2.0

### Minor Changes

- [#17](https://github.com/omnidotdev/kiln/pull/17) [`075571d`](https://github.com/omnidotdev/kiln/commit/075571d3a1c309dea432bd110ee94852f2cc9289) Thanks [@coopbri](https://github.com/coopbri)! - Expand the build-configuration surface. Runtime version pinning now works for all
  fourteen providers and reads the shared ecosystem version files (`.tool-versions`,
  mise config, `rust-toolchain.toml`) in addition to the language-native ones. New
  `kiln.json`/`kiln.toml` fields: `build_apt_packages` / `deploy_apt_packages`,
  `build_image` / `runtime_image`, `paths`, `secrets` (BuildKit secrets mounted on
  build-stage commands and forwarded to the build), and `pre_build` / `post_build`
  command hooks. The same options can be set through `KILN_*` environment variables
  (precedence: CLI flag > env var > config file > auto-detect). Static-site
  detection now also covers SvelteKit (static adapter) and Angular. The generated
  output remains a single, readable Dockerfile.

## 0.1.0

Initial release.

Kiln turns a source repository into a container image with zero configuration: it
detects the language, generates a plain, readable Dockerfile you own, and builds
it into an image via BuildKit. There is no proprietary build graph and no
lock-in, just a standard Dockerfile and image that work with any registry and
any Docker or BuildKit based toolchain.

Supported languages: C++, Deno, .NET, Elixir, Gleam, Go, Java, Node.js, PHP,
Python, Ruby, Rust, static sites, and shell.

Later entries are managed with Changesets and generated on release.
