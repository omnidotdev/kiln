---
"@omnidotdev/kiln": minor
---

Expand the build-configuration surface. Runtime version pinning now works for all
fourteen providers and reads the shared ecosystem version files (`.tool-versions`,
mise config, `rust-toolchain.toml`) in addition to the language-native ones. New
`kiln.json`/`kiln.toml` fields: `build_apt_packages` / `deploy_apt_packages`,
`build_image` / `runtime_image`, `paths`, `secrets` (BuildKit secrets mounted on
build-stage commands and forwarded to the build), and `pre_build` / `post_build`
command hooks. The same options can be set through `KILN_*` environment variables
(precedence: CLI flag > env var > config file > auto-detect). Static-site
detection now also covers SvelteKit (static adapter) and Angular. The generated
output remains a single, readable Dockerfile.
