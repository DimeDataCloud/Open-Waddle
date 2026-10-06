# Contributing to Waddle

Thanks for helping. Bug reports, fixes, new tools and better docs are all welcome.

## Reporting a bug

Open an issue with:
- what you asked Waddle to do, what you expected, and what happened;
- your Windows (or macOS/Linux) version and device;
- the self-test report: right-click the duck → Settings → Diagnostics → **Run self-test**, then attach the `waddle-report-<time>.txt` it saves. It leaves out keys, file contents, window titles and what you've typed, but read it before posting.

Security problems go through [SECURITY.md](SECURITY.md) instead, not public issues.

## Building and testing

See [README.md](README.md#option-b-build-from-source) for the toolchain. Before opening a pull request, run what CI runs:

```bash
npm ci
npm run typecheck
npm test
npm run build                 # also rebuilds the Chrome extension; commit extension/build if it changed
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`cargo test` needs no API key and makes no network calls: the agent and session tests use a scripted mock model. The model benchmarks in `bench/` and `crates/waddle-core/tests/assist.rs` do call real models; they're `#[ignore]`d and cost money, so run them only when you change prompts or model defaults (see [docs/MODELS.md](docs/MODELS.md)).

## Making a release (maintainers)

1. Bump the version everywhere (`Cargo.toml`, `package.json`, `package-lock.json`, `src-tauri/tauri.conf.json`), update [CHANGELOG.md](CHANGELOG.md), and merge to `main`.
2. Push a tag that matches the version: `git tag v0.3.0 && git push origin v0.3.0`.
3. The **Release** workflow builds the Windows ARM64 and x64 installers, signs the update files, and creates a *draft* release with the installers and `latest.json`. Check it, then publish it. Installed copies find the update through `releases/latest/download/latest.json`, which only exists once the release is published.

The workflow needs two repository secrets (Settings → Secrets and variables → Actions), made once with `npx @tauri-apps/cli signer generate -w waddle.key`:
- `TAURI_SIGNING_PRIVATE_KEY`: the contents of `waddle.key`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: its password

The matching public key is in `src-tauri/tauri.conf.json` (`plugins.updater.pubkey`). If the private key is lost, installed copies can't verify new updates; they have to reinstall by hand. Never commit the private key.

## How the code is laid out

[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) explains the parts. In short: `crates/waddle-core` is the brain and has no OS code, so most logic can be tested there; `src-tauri` is the desktop shell; `src/` is the overlay and Settings frontend.

## Ground rules for changes

- **Safety tiers are fixed rules** in `crates/waddle-core/src/safety.rs`. A change that lets a model lower a tier, approve an action, or reach keys, endpoints, folders, the budget or MCP servers won't be merged. New tools need a tier rule and a test.
- **Anything from the screen, files, web pages, email or tools is untrusted** and must stay wrapped as untrusted text.
- **No telemetry.** Waddle sends nothing anywhere except the model and service calls the user set up.
- Keep messages to the user short and plain, and match the style of the code around your change.
- Add or update tests with the change, and a line in [CHANGELOG.md](CHANGELOG.md) under the next version.

By contributing you agree that your work is released under the [MIT license](LICENSE).
