# Contributing to Tailery

First off, thank you for considering contributing to Tailery! It's people like you that make open-source software such a great community.

## Where do I go from here?

If you've noticed a bug or have a feature request, make one! It's generally best if you get confirmation of your bug or approval for your feature request this way before starting to code.

## Setting up your environment

1. **Fork the repo** and clone it locally.
2. Ensure you have **Rust and Cargo** installed (you can use [rustup](https://rustup.rs/)).
3. (Optional but recommended) Ensure you have **Docker** installed and running, as some tests and features rely on Docker integrations.

## Building and Testing

To build the project:
```bash
cargo build
```

To run the tests:
```bash
cargo test
```

## Making Changes

1. Create a new branch for your feature or bugfix: `git checkout -b feature/my-awesome-feature`.
2. Make your changes in the codebase.
3. Ensure your code complies with our formatting standards:
   ```bash
   cargo fmt
   cargo clippy -- -D warnings
   ```
4. Commit your changes. We recommend using conventional commits.

## Submitting a Pull Request

1. Push your branch to your fork.
2. Open a Pull Request against the `main` branch of the `tailery-dev/tailery` repository.
3. Fill out the Pull Request template completely.
4. Wait for a maintainer to review your code. We try to be as responsive as possible!

## Community

Remember to follow our [Code of Conduct](CODE_OF_CONDUCT.md) when interacting with the community.
