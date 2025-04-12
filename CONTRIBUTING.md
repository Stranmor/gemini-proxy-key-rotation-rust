# Contributing to Gemini Proxy Key Rotation (Rust)

First off, thank you for considering contributing! We welcome any help, from reporting bugs and suggesting features to submitting pull requests.

## How Can I Contribute?

### Reporting Bugs

-   Ensure the bug was not already reported by searching on GitHub under [Issues](https://github.com/stranmor/gemini-proxy-key-rotation-rust/issues).
-   If you're unable to find an open issue addressing the problem, [open a new one](https://github.com/stranmor/gemini-proxy-key-rotation-rust/issues/new). Be sure to include a **title and clear description**, as much relevant information as possible, and a **code sample or an executable test case** demonstrating the expected behavior that is not occurring.

### Suggesting Enhancements

-   Open a new issue to discuss your suggestion. Provide a clear description of the enhancement and why it would be beneficial.

### Pull Requests

1.  **Fork the repository** on GitHub.
2.  **Clone your fork** locally: `git clone git@github.com:your-username/gemini-proxy-key-rotation-rust.git`
3.  **Create a new branch** for your changes: `git checkout -b feature/your-feature-name` or `git checkout -b fix/your-bug-fix-name`.
4.  **Make your changes.** Ensure you:
    -   Follow the existing code style.
    -   Add comments where necessary (in English).
    -   Add tests for new functionality or bug fixes if applicable.
    -   Update documentation (`README.md`, code comments) if necessary.
5.  **Format and lint your code:**
    ```sh
    cargo fmt
    cargo clippy --all-targets --all-features -- -D warnings
    ```
6.  **Run tests:**
    ```sh
    cargo test --all-targets --all-features
    ```
7.  **Commit your changes** using a descriptive commit message.
8.  **Push your branch** to GitHub: `git push origin feature/your-feature-name`.
9.  **Open a pull request** to the `main` branch of the original repository. Provide a clear description of the changes and link any relevant issues.

## Code Style

-   Please follow the standard Rust formatting guidelines (`cargo fmt`).
-   Use `clippy` to catch common mistakes and improve code quality.

## Code of Conduct

This project adheres to the Contributor Covenant Code of Conduct. By participating, you are expected to uphold this code. Please read the [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) file.

Thank you for your contribution!