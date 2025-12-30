# Contributing to Mudae Selfbot

Thank you for considering contributing to Mudae Selfbot! This document provides guidelines and instructions for contributing.

## Getting Started

1. Fork the repository
2. Clone your fork: `git clone https://github.com/yourusername/mudae-selfbot.git`
3. Create a new branch: `git checkout -b feature/your-feature-name`

## Development Setup

1. Make sure you have Rust installed (latest stable version)
2. Install dependencies: `cargo build`
3. Run the project: `cargo run`

## Code Style

- Follow Rust standard formatting: `cargo fmt`
- Run clippy: `cargo clippy`
- Keep functions focused and well-documented
- Use meaningful variable and function names
- Add comments for complex logic

## Commit Messages

- Use clear, descriptive commit messages
- Start with a verb in imperative mood (e.g., "Add", "Fix", "Update")
- Keep the first line under 72 characters
- Add more details in the body if needed

Examples:
- `Add fuzzy matching threshold configuration`
- `Fix cooldown calculation for roll commands`
- `Update TUI to show connection status`

## Pull Request Process

1. Make sure your code compiles and passes tests
2. Update documentation if you've changed functionality
3. Add your changes to CHANGELOG.md if applicable
4. Submit a pull request with a clear description of changes
5. Be responsive to feedback and requested changes

## Areas for Contribution

- Bug fixes
- Performance improvements
- New features
- Documentation improvements
- Code refactoring
- Test coverage

## Questions?

Feel free to open an issue for questions or discussions about potential contributions.

Thank you for helping improve Mudae Selfbot!
