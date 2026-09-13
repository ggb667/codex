<p align="center"><strong>Codex CLI</strong> is a coding agent from OpenAI that runs locally on your computer.
<p align="center">
  <img src="https://github.com/openai/codex/blob/main/.github/codex-cli-splash.png" alt="Codex CLI splash" width="80%" />
</p>
</br>
If you want Codex in your code editor (VS Code, Cursor, Windsurf), <a href="https://developers.openai.com/codex/ide">install in your IDE.</a>
</br>If you want the desktop app experience, run <code>codex app</code> or visit <a href="https://chatgpt.com/codex?app-landing-page=true">the Codex App page</a>.
</br>If you are looking for the <em>cloud-based agent</em> from OpenAI, <strong>Codex Web</strong>, go to <a href="https://chatgpt.com/codex">chatgpt.com/codex</a>.</p>

---

## Codex-side integration for multi-agent Codex orchestration such as the Agenic Pony System

This fork is the customized **Codex TUI** used by the separate **Agenic Pony System** runtime.

The Agenic Pony System is the orchestration layer: it defines agents, launchers, team configuration, shared coordination state, generated rosters, routing policy, personalities, and cross-project behavior. That runtime depends on this Codex fork for the Codex-side hooks it needs, but this fork does not inherently contain or launch the whole Agenic Pony System by itself.

Conceptually:

```text
Agenic Pony System
        │
        ▼
Customized Codex TUI
        │
        ▼
Upstream OpenAI Codex
```

The goal of the combined stack is to make several specialized agents behave more like a software team than a collection of isolated chat sessions.

### What this fork adds

- **Codex-side Pony IPC hooks** for inter-agent messaging, including `/tell` for direct agent-to-agent communication.
- **Generated roster routing support** so the TUI can address the current Agenic Pony team without hard-coded recipient lists.
- **Agent-specific TUI prompt styling** with configurable prompt glyphs and backgrounds, making parallel sessions easy to distinguish visually.
- **Terminal input recovery** so TUI input can restart after an underlying terminal event stream closes unexpectedly.

The separate Agenic Pony runtime supplies the persistent named agents, launcher scripts, generated session configuration, shared coordination files, and personalities such as **Twilight Sparkle** and **Princess Celestia Sol Invictus**.

For implementation details, see [Pony IPC](./docs/pony-ipc.md).

> **Status:** This is an experimental fork of OpenAI Codex. The Pony extensions are maintained separately from upstream Codex and are periodically rebased/ported onto newer Codex releases.

---

## Quickstart

### Installing and running Codex CLI

Run the following on Mac or Linux to install Codex CLI:

```shell
curl -fsSL https://chatgpt.com/codex/install.sh | sh
```

Run the following on Windows to install Codex CLI:

```shell
powershell -ExecutionPolicy ByPass -c "irm https://chatgpt.com/codex/install.ps1 | iex"
```

The standalone installers download from `https://releases.openai.com/codex` by default and fall back to GitHub Releases if a metadata or asset download is unavailable. To force GitHub Releases, set `CODEX_INSTALLER_USE_RELEASES_OPENAI_COM` to `false` (`0` and `no` are also accepted):

```shell
curl -fsSL https://chatgpt.com/codex/install.sh | CODEX_INSTALLER_USE_RELEASES_OPENAI_COM=false sh
```

```powershell
$env:CODEX_INSTALLER_USE_RELEASES_OPENAI_COM='false'; irm https://chatgpt.com/codex/install.ps1 | iex
```

Codex CLI can also be installed via the following package managers:

```shell
# Install using npm
npm install -g @openai/codex
```

```shell
# Install using Homebrew
brew install --cask codex
```

Then simply run `codex` to get started.

<details>
<summary>You can also go to the <a href="https://github.com/openai/codex/releases/latest">latest GitHub Release</a> and download the appropriate binary for your platform.</summary>

Each GitHub Release contains many executables, but in practice, you likely want one of these:

- macOS
  - Apple Silicon/arm64: `codex-aarch64-apple-darwin.tar.gz`
  - x86_64 (older Mac hardware): `codex-x86_64-apple-darwin.tar.gz`
- Linux
  - x86_64: `codex-x86_64-unknown-linux-musl.tar.gz`
  - arm64: `codex-aarch64-unknown-linux-musl.tar.gz`

Each archive contains a single entry with the platform baked into the name (e.g., `codex-x86_64-unknown-linux-musl`), so you likely want to rename it to `codex` after extracting it.

</details>

### Using Codex with your ChatGPT plan

Run `codex` and select **Sign in with ChatGPT**. We recommend signing into your ChatGPT account to use Codex as part of your Plus, Pro, Business, Edu, or Enterprise plan. [Learn more about what's included in your ChatGPT plan](https://help.openai.com/en/articles/11369540-codex-in-chatgpt).

You can also use Codex with an API key, but this requires [additional setup](https://developers.openai.com/codex/auth#sign-in-with-an-api-key).

## Docs

- [**Codex Documentation**](https://developers.openai.com/codex)
- [**Contributing**](./docs/contributing.md)
- [**Installing & building**](./docs/install.md)
- [**Open source fund**](./docs/open-source-fund.md)

This repository is licensed under the [Apache-2.0 License](LICENSE).
