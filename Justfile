# ==============================================================================
# tailery CLI - Multi-Platform Release & Local Development Justfile
# ==============================================================================

binary_name := "tailery"
cargo := "cargo"
dist_dir := "dist"

manifest_dir := `if [ -f Cargo.toml ]; then echo "."; elif [ -f tailery/Cargo.toml ]; then echo "tailery"; else echo "ERR_NO_MANIFEST"; fi`
manifest_path := manifest_dir + "/Cargo.toml"
cargo_target_dir := manifest_dir + "/target"

version := `[ -f Cargo.toml ] && sed -n -E 's/^version = "([^"]+)"/\1/p' Cargo.toml | head -n 1 || sed -n -E 's/^version = "([^"]+)"/\1/p' tailery/Cargo.toml | head -n 1`

host_os := os()
host_arch := arch()

host_target_cmd := `rustc -vV 2>/dev/null | sed -n 's/host: //p' || echo ""`
host_target_str := if host_target_cmd == "" { host_arch + "-" + host_os } else { host_target_cmd }

sha256_cmd := `if command -v sha256sum >/dev/null 2>&1; then echo "sha256sum"; else echo "shasum -a 256"; fi`

target_darwin_arm64 := "aarch64-apple-darwin"
target_darwin_amd64 := "x86_64-apple-darwin"
target_linux_arm64 := "aarch64-unknown-linux-musl"
target_linux_amd64 := "x86_64-unknown-linux-musl"
release_targets := target_darwin_arm64 + " " + target_darwin_amd64 + " " + target_linux_arm64 + " " + target_linux_amd64

# ==============================================================================
# Default & Help
# ==============================================================================

@_default: help

@help:
    echo "========================================================================"
    echo "  tailery CLI (v{{version}}) - Justfile"
    echo "========================================================================"
    echo ""
    echo "  Host Detected: {{host_target_str}} ({{host_os}} / {{host_arch}})"
    echo ""
    echo "  Local Development & Testing:"
    echo "    just test                 Run tests on current host arch/OS"
    echo "    just test-local           Alias for 'just test'"
    echo "    just build-local          Build release binary for current host"
    echo "    just check                Fast typecheck with cargo check"
    echo "    just clippy               Run linter (cargo clippy)"
    echo "    just fmt                  Check formatting (cargo fmt --check)"
    echo "    just fmt-fix              Apply formatting fixes (cargo fmt)"
    echo "    just doc                  Generate and check documentation"
    echo "    just run [ARGS=...]       Run tailery with optional arguments"
    echo ""
    echo "  Multi-Platform Release Builds:"
    echo "    just build-all            Build release binaries for all 4 platforms"
    echo "    just build-darwin         Build macOS (ARM64 + AMD64)"
    echo "    just build-darwin-arm64   Build macOS ARM64 (Apple Silicon: {{target_darwin_arm64}})"
    echo "    just build-darwin-amd64   Build macOS AMD64 (Intel: {{target_darwin_amd64}})"
    echo "    just build-darwin-universal Create macOS Universal Binary (lipo)"
    echo "    just build-linux          Build Linux (ARM64 + AMD64 musl static)"
    echo "    just build-linux-arm64    Build Linux ARM64 (musl: {{target_linux_arm64}})"
    echo "    just build-linux-amd64    Build Linux AMD64 (musl: {{target_linux_amd64}})"
    echo "    just build-target <tgt>   Build release binary for any rust target triple"
    echo ""
    echo "  Packaging & Distribution:"
    echo "    just release              Build all platforms and package tarballs + SHA256"
    echo "    just dist                 Alias for 'just release'"
    echo "    just package-all          Package all built release targets"
    echo "    just package-target <tgt> Package a specific target into .tar.gz + .sha256"
    echo ""
    echo "  Setup & Cleanup:"
    echo "    just setup                Install all rustup target toolchains"
    echo "    just clean                Clean target/ and {{dist_dir}}/ directories"
    echo "========================================================================"

# ==============================================================================
# Local Development & Testing Targets (DRY)
# ==============================================================================

alias test := test-local
alias lint := clippy
alias local := build-local

@test-local *args:
    echo "==> Running local test suite for host: {{host_target_str}}..."
    {{cargo}} test --manifest-path {{manifest_path}} {{args}}

@check *args:
    echo "==> Running cargo check..."
    {{cargo}} check --manifest-path {{manifest_path}} {{args}}

@clippy *args:
    echo "==> Running cargo clippy..."
    {{cargo}} clippy --manifest-path {{manifest_path}} {{args}} --all-targets -- -D warnings

@fmt *args:
    echo "==> Checking code formatting..."
    {{cargo}} fmt --manifest-path {{manifest_path}} {{args}} -- --check

@fmt-fix *args:
    echo "==> Applying code formatting..."
    {{cargo}} fmt --manifest-path {{manifest_path}} {{args}}

@doc *args:
    echo "==> Checking documentation..."
    RUSTDOCFLAGS="-D warnings" {{cargo}} doc --manifest-path {{manifest_path}} --no-deps --document-private-items --all-features --workspace --examples {{args}}

@build-local:
    mkdir -p {{dist_dir}}
    echo "==> Building {{binary_name}} (release) for current host: {{host_target_str}}..."
    {{cargo}} build --release --manifest-path {{manifest_path}}
    cp {{cargo_target_dir}}/release/{{binary_name}} {{dist_dir}}/{{binary_name}}
    echo "==> Local binary created at: {{dist_dir}}/{{binary_name}}"

@run *args:
    {{cargo}} run --manifest-path {{manifest_path}} -- {{args}}

# ==============================================================================
# Multi-Platform Release Build Engine
# ==============================================================================

@build-target target:
    #!/usr/bin/env bash
    set -e
    mkdir -p {{dist_dir}}
    echo "==> Building {{binary_name}} for target: {{target}} (release profile)..."
    if echo "{{target}}" | grep -q "darwin" && [ "{{host_os}}" = "macos" ]; then
        {{cargo}} build --release --manifest-path {{manifest_path}} --target {{target}}
    elif echo "{{target}}" | grep -q "linux" && [ "{{host_os}}" = "linux" ]; then
        {{cargo}} build --release --manifest-path {{manifest_path}} --target {{target}}
    elif command -v cross >/dev/null 2>&1; then
        echo "    Using 'cross' tool for cross-compilation target {{target}}..."
        cross build --release --manifest-path {{manifest_path}} --target {{target}}
    elif command -v cargo-zigbuild >/dev/null 2>&1; then
        echo "    Using 'cargo-zigbuild' for cross-compilation target {{target}}..."
        cargo zigbuild --release --manifest-path {{manifest_path}} --target {{target}}
    else
        echo "    Attempting cargo build for cross-compilation target {{target}}..."
        if ! {{cargo}} build --release --manifest-path {{manifest_path}} --target {{target}}; then
            echo ""
            echo "ERROR: Cross-compilation for target '{{target}}' failed."
            echo "To cross-compile between Linux and macOS seamlessly, install 'cross':"
            echo "  cargo install cross --git https://github.com/cross-rs/cross"
            echo "And make sure Docker or Podman is running."
            exit 1
        fi
    fi
    cp {{cargo_target_dir}}/{{target}}/release/{{binary_name}} {{dist_dir}}/{{binary_name}}-{{target}}
    echo "==> Artifact ready: {{dist_dir}}/{{binary_name}}-{{target}}"

@build-darwin-arm64: (build-target target_darwin_arm64)
    cp {{dist_dir}}/{{binary_name}}-{{target_darwin_arm64}} {{dist_dir}}/{{binary_name}}-darwin-arm64

@build-darwin-amd64: (build-target target_darwin_amd64)
    cp {{dist_dir}}/{{binary_name}}-{{target_darwin_amd64}} {{dist_dir}}/{{binary_name}}-darwin-amd64

@build-darwin-universal: build-darwin-arm64 build-darwin-amd64
    #!/usr/bin/env bash
    set -e
    if [ "{{host_os}}" = "macos" ]; then
        echo "==> Creating macOS Universal Binary via lipo..."
        lipo -create \
            {{dist_dir}}/{{binary_name}}-{{target_darwin_arm64}} \
            {{dist_dir}}/{{binary_name}}-{{target_darwin_amd64}} \
            -output {{dist_dir}}/{{binary_name}}-darwin-universal
        echo "==> Universal binary ready: {{dist_dir}}/{{binary_name}}-darwin-universal"
    else
        echo "WARNING: lipo is only available on macOS hosts. Skipping universal binary creation."
    fi

@build-linux-arm64: (build-target target_linux_arm64)
    cp {{dist_dir}}/{{binary_name}}-{{target_linux_arm64}} {{dist_dir}}/{{binary_name}}-linux-arm64

@build-linux-amd64: (build-target target_linux_amd64)
    cp {{dist_dir}}/{{binary_name}}-{{target_linux_amd64}} {{dist_dir}}/{{binary_name}}-linux-amd64

build-darwin: build-darwin-arm64 build-darwin-amd64
build-linux: build-linux-arm64 build-linux-amd64
build-all: build-darwin build-linux

# ==============================================================================
# Packaging & Distribution
# ==============================================================================

alias dist := release

@package-target target: (build-target target)
    #!/usr/bin/env bash
    set -e
    mkdir -p {{dist_dir}}
    echo "==> Packaging {{binary_name}}-v{{version}}-{{target}}..."
    TMP_PKG_DIR=$(mktemp -d)
    PKG_SUBDIR="$TMP_PKG_DIR/{{binary_name}}-v{{version}}-{{target}}"
    mkdir -p "$PKG_SUBDIR"
    cp {{dist_dir}}/{{binary_name}}-{{target}} "$PKG_SUBDIR/{{binary_name}}"
    [ -f {{manifest_dir}}/README.md ] && cp {{manifest_dir}}/README.md "$PKG_SUBDIR/" || true
    [ -f {{manifest_dir}}/LICENSE-MIT ] && cp {{manifest_dir}}/LICENSE-MIT "$PKG_SUBDIR/" || true
    [ -f {{manifest_dir}}/LICENSE-APACHE ] && cp {{manifest_dir}}/LICENSE-APACHE "$PKG_SUBDIR/" || true
    tar -czf {{dist_dir}}/{{binary_name}}-v{{version}}-{{target}}.tar.gz -C "$TMP_PKG_DIR" "{{binary_name}}-v{{version}}-{{target}}"
    rm -rf "$TMP_PKG_DIR"
    cd {{dist_dir}} && {{sha256_cmd}} {{binary_name}}-v{{version}}-{{target}}.tar.gz > {{binary_name}}-v{{version}}-{{target}}.tar.gz.sha256
    echo "==> Package ready: {{dist_dir}}/{{binary_name}}-v{{version}}-{{target}}.tar.gz"

@package-all: (package-target target_darwin_arm64) (package-target target_darwin_amd64) (package-target target_linux_arm64) (package-target target_linux_amd64)
    #!/usr/bin/env bash
    set -e
    echo "==> Generating combined checksums.sha256..."
    cd {{dist_dir}} && {{sha256_cmd}} {{binary_name}}-v{{version}}-*.tar.gz > checksums.sha256
    echo ""
    echo "========================================================================"
    echo "  Release Distribution Artifacts Ready in {{dist_dir}}/"
    echo "========================================================================"
    ls -lh {{dist_dir}}/{{binary_name}}-v{{version}}-*.tar.gz {{dist_dir}}/checksums.sha256

@release: package-all

# ==============================================================================
# Setup & Cleanup
# ==============================================================================

alias setup := setup-targets

@setup-targets:
    echo "==> Installing rustup target toolchains for multi-platform build..."
    rustup target add {{release_targets}}
    echo "==> All target toolchains installed."

@clean:
    echo "==> Cleaning build artifacts..."
    {{cargo}} clean --manifest-path {{manifest_path}}
    rm -rf {{dist_dir}}
    echo "==> Clean complete."
