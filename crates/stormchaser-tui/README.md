# Stormchaser TUI

A Ratatui-based Terminal User Interface for Stormchaser.

## Features

- List workflow runs with their status.
- Real-time status updates for workflow runs and steps.
- Detailed view of workflow run steps.
- Tabbed interface for easy navigation.

## Usage

```bash
cargo run -p stormchaser-tui -- --url http://localhost:3000
```

Upon startup, if a token isn't provided, you will be prompted to press `Enter` to open your default web browser and securely log in via OIDC (Dex).

Alternatively, you can provide an existing token directly:

```bash
cargo run -p stormchaser-tui -- --url http://localhost:3000 --token YOUR_TOKEN
```

### Keybindings

- `q`: Quit
- `r`: Refresh runs
- `Tab`: Switch between Runs, Steps, and Logs tabs
- `j` / `Down`: Select next run
- `k` / `Up`: Select previous run
- `Enter`: Fetch details and start watching the selected run
