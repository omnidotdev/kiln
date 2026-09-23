# kiln

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
