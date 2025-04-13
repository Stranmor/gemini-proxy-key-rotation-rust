# Contributing to Gemini Proxy Key Rotation (Rust)

We welcome contributions to improve this project! Please take a moment to review this document to understand how you can contribute.

## How to Contribute

1.  **Reporting Issues:** If you find a bug, have a feature request, or want to suggest an improvement, please open an issue on the GitHub repository. Provide as much detail as possible, including steps to reproduce (for bugs) or the motivation (for features).
2.  **Submitting Pull Requests:**
    *   Fork the repository.
    *   Create a new branch for your feature or bug fix (`git checkout -b my-feature-branch`).
    *   Make your changes. Ensure your code adheres to the project's style (run `cargo fmt`) and passes checks (`cargo check`, `cargo clippy -- -D warnings`).
    *   Add tests for your changes if applicable. Ensure all tests pass (`cargo test`).
    *   Commit your changes with clear and concise commit messages.
    *   Push your branch to your fork (`git push origin my-feature-branch`).
    *   Open a pull request against the `main` branch of the original repository. Provide a clear description of your changes.

## Development Setup

1.  Install Rust: [https://rustup.rs/](https://rustup.rs/)
2.  Install Docker: [https://docs.docker.com/engine/install/](https://docs.docker.com/engine/install/)
3.  Clone the repository: `git clone https://github.com/stranmor/gemini-proxy-key-rotation-rust.git`
4.  Build: `cargo build`
5.  Run checks: `cargo check`, `cargo clippy`, `cargo fmt --check`
6.  Run tests: `cargo test`

## Code Style

Please run `cargo fmt` before committing your changes to ensure consistent code style.

Thank you for contributing!