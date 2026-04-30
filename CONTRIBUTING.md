# Contributing to Stormchaser

First off, thank you for considering contributing to Stormchaser! It's people like you that make Stormchaser such a great tool.

Stormchaser is an open source project and we love to receive contributions from our community — you! There are many ways to contribute, from writing tutorials or blog posts, improving the documentation, submitting bug reports and feature requests or writing code which can be incorporated into Stormchaser itself.

Following these guidelines helps to communicate that you respect the time of the developers managing and developing this open source project. In return, they should reciprocate that respect in addressing your issue or assessing patches and features.

## Code of Conduct

We have a [Code of Conduct](CODE_OF_CONDUCT.md) that we expect all contributors to adhere to. Please read it before contributing.

## How to Report a Bug

If you find a bug, please open an issue on our GitHub repository. Please include as much information as possible, including:

* A clear and descriptive title.
* A description of the problem.
* Steps to reproduce the problem.
* The expected behavior.
* The actual behavior.
* Your system information (OS, Rust version, etc.).

## How to Suggest a Feature or Enhancement

If you have an idea for a new feature or an enhancement to an existing one, please open an issue on our GitHub repository. Please include:

* A clear and descriptive title.
* A detailed description of the proposed feature or enhancement.
* The motivation for the new feature.
* Any examples or mockups that might help to illustrate the new feature.

## Your First Code Contribution

Unsure where to begin contributing to Stormchaser? You can start by looking through these `good first issue` and `help wanted` issues:

* [Good first issues](https://github.com/paninfracon/stormchaser/labels/good%20first%20issue) - issues which should only require a few lines of code, and a test or two.
* [Help wanted issues](https://github.com/paninfracon/stormchaser/labels/help%20wanted) - issues which should be a bit more involved than `good first issue` issues.

### Getting the code

1. Fork the repository on GitHub.
2. Clone your fork locally: `git clone https://github.com/paninfracon/stormchaser.git`
3. Create a new branch for your changes: `git checkout -b my-new-feature`

### Making changes

* Make your changes to the code.
* Ensure that the code is formatted with `rustfmt`. The project includes a `rustfmt.toml` file with the project's formatting rules.
* This project uses `pre-commit` hooks. Please make sure to install them by running `pre-commit install`.
* Add tests for your changes. You can run the tests with `cargo test -- --test-threads=2`.
* Make sure the tests pass.

## Pull Request Process

1. Push your changes to your fork: `git push origin my-new-feature`
2. Open a pull request on the Stormchaser repository.
3. The pull request will be reviewed by the maintainers.
4. Once the pull request is approved, it will be merged into the `main` branch.

Thank you for your contribution!
