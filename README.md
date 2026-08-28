<div align="center">

# 🦊 Tailery

[![CI](https://github.com/tailery-dev/tailery/workflows/CI/badge.svg)](https://github.com/tailery-dev/tailery/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-blue.svg)](https://www.rust-lang.org)
[![Security Policy](https://img.shields.io/badge/security-policy-green.svg)](SECURITY.md)

**A powerful, blazing-fast MCP and skill manager for AI Coding Assistants.**

[Features](#-features) •
[Installation](#-installation) •
[Getting Started](#-getting-started) •
[CLI & Diagnostics](#-cli--diagnostics) •
[Contributing](#-contributing)

</div>

---

Tailery acts as the definitive hub for all your coding assistant's needs, managing **Model Context Protocol (MCP)** servers, environment variables, security sandboxing, and diverse AI skills seamlessly across **Cursor**, **Claude Code**, **Zed**, and **Google Antigravity**.

## ✨ Features

- **🚀 Blazing Fast:** Written in Rust for near-instant startup times and minimal memory footprint.
- **🔌 MCP Native:** First-class support for the Model Context Protocol, effortlessly integrating various AI agents.
- **🧠 Profile & Skill Management:** Organize, toggle, and configure servers for specific profiles dynamically.
- **🖥️ TUI Interface:** A beautiful, responsive terminal user interface built with Ratatui.
- **🐳 Docker Integration:** Isolated and secure execution of tools and skills using Docker containers (Docker Desktop, OrbStack, Colima, Podman).
- **🩺 Built-in Doctor:** Comprehensive diagnostics verifying socket connectivity, adapter configuration health, and permissions.
- **🔒 Automated Backups:** Rolling backup snapshots ensuring your client configs are always safe before modifications.

## 📦 Installation

### Quick Install (macOS / Linux)
You can quickly install Tailery using our installation script:

```bash
curl -fsSL https://raw.githubusercontent.com/tailery-dev/tailery/main/install.sh | bash
```

### Via Cargo Binstall (Fast Precompiled Binary)
```bash
cargo binstall tailery
```

### From Source
If you have [Rust and Cargo](https://rustup.rs/) (1.85+) installed:

```bash
git clone https://github.com/tailery-dev/tailery.git
cd tailery
cargo install --path .
```

## 🚀 Getting Started

Once installed, launch the interactive dashboard:

```bash
tailery
```

Or sync your active profile to all installed AI assistants directly from the command line:

```bash
tailery sync
```

## 🩺 CLI & Diagnostics

### System Health & Diagnostics (`tailery doctor`)
Run comprehensive health checks on your host environment, container runtime, client adapter files, and backup storage:

```bash
tailery doctor
```

### Shell Auto-Completions (`tailery completions`)
Generate shell auto-completions for your shell:

```bash
# Bash
tailery completions bash > ~/.local/share/bash-completion/completions/tailery

# Zsh
tailery completions zsh > ~/.zfunc/_tailery

# Fish
tailery completions fish > ~/.config/fish/completions/tailery.fish

# PowerShell
tailery completions powershell > $PROFILE
```

### Profile & Backup Management
```bash
# List profiles and configured servers
tailery list

# Synchronize specific client or profile
tailery sync --profile default --client cursor

# List or restore client backups
tailery backup list
tailery restore --client cursor
```

## 🤝 Contributing

We love our contributors! Please see our [CONTRIBUTING.md](CONTRIBUTING.md) for details on getting started. We also provide a pre-commit hook via `just install-hooks` to ensure formatting and linting standards before committing.

By participating in this project, you agree to abide by our [Code of Conduct](CODE_OF_CONDUCT.md).

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
