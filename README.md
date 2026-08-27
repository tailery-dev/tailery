<div align="center">

# 🦊 Tailery

[![CI](https://github.com/tailery-dev/tailery/workflows/CI/badge.svg)](https://github.com/tailery-dev/tailery/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.80%2B-blue.svg)](https://www.rust-lang.org)

**A powerful, blazing-fast MCP and skill manager for Coding Assistants.**

[Features](#-features) •
[Installation](#-installation) •
[Getting Started](#-getting-started) •
[Contributing](#-contributing)

</div>

---

Tailery acts as the definitive hub for all your coding assistant's needs, managing **Model Context Protocol (MCP)** servers and diverse AI skills seamlessly. Built with Rust, it prioritizes performance, safety, and a smooth developer experience.

## ✨ Features

- **🚀 Blazing Fast:** Written in Rust for near-instant startup times and minimal memory footprint.
- **🔌 MCP Native:** First-class support for the Model Context Protocol, effortlessly integrating various AI agents.
- **🧠 Skill Management:** Organize, toggle, and configure skills for your coding assistants dynamically.
- **🖥️ TUI Interface:** A beautiful, responsive terminal user interface built with Ratatui.
- **🐳 Docker Integration:** Isolated and secure execution of tools and skills using Docker containers.

## 📦 Installation

### Quick Install (macOS / Linux)
You can quickly install Tailery using our installation script:

```bash
curl -fsSL https://raw.githubusercontent.com/tailery-dev/tailery/main/install.sh | bash
```

### From Source
If you have [Rust and Cargo](https://rustup.rs/) installed:

```bash
git clone https://github.com/tailery-dev/tailery.git
cd tailery
cargo install --path .
```

## 🚀 Getting Started

Once installed, you can start Tailery directly from your terminal:

```bash
tailery start
```

For configuration examples, check out the [`examples/`](examples) directory in this repository!

## 🤝 Contributing

We love our contributors! Please see our [CONTRIBUTING.md](CONTRIBUTING.md) for details on how to get started. By participating in this project, you agree to abide by our [Code of Conduct](CODE_OF_CONDUCT.md).

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
