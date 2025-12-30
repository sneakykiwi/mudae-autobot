# Mudae Selfbot

> **⚠️ WARNING: This is a Discord selfbot. Using selfbots violates Discord's Terms of Service and can result in your account being banned. Use at your own risk.**

A Discord selfbot for automating Mudae interactions. Got fed up with friends taking your characters? This bot will automatically roll, claim your wishlist characters, react to kakera, and run daily commands so you never miss out on your favorites.

## Screenshot

<!-- Add a screenshot of the TUI here -->

## Features

- **Auto Roll**: Automatically executes roll commands across multiple channels with configurable cooldowns
- **Wishlist System**: Automatically claims characters from your wishlist when they appear
  - Fuzzy matching support for character name variations
  - Priority-based claiming
  - Character verification system
- **Auto Kakera React**: Automatically reacts to kakera drops to collect them
- **Auto Daily**: Automatically runs `$daily` and `$dk` commands at a scheduled time
- **Interactive TUI**: Beautiful terminal interface for monitoring and control
  - Real-time statistics and activity feed
  - Connection status monitoring
  - Roll history and claim tracking
  - Settings management
  - Character search functionality
- **Statistics Tracking**: Tracks rolls executed, characters claimed, kakera collected, and more
- **Multi-Channel Support**: Monitor and interact with multiple Mudae channels simultaneously
- **Database Persistence**: Settings, statistics, credentials, and channel data are saved in a local SQLite database
- **Character Search**: Search for characters directly from the TUI
- **Smart Cooldown Management**: Tracks command cooldowns to maximize efficiency

## Installation

### Prerequisites

- Rust (latest stable version)
- A Discord user token (⚠️ **Do not share your token with anyone**)

### Building

```bash
git clone https://github.com/yourusername/mudae-selfbot.git
cd mudae-selfbot
cargo build --release
```

The binary will be in `target/release/mudae-selfbot.exe` (Windows) or `target/release/mudae-selfbot` (Linux/macOS).

## Usage

### First Time Setup

Run the bot without any arguments to launch the setup wizard:

```bash
cargo run
# or
./target/release/mudae-selfbot
```

The setup wizard will guide you through:
- Setting your Discord token
- Configuring channel IDs
- Setting up roll commands
- Configuring cooldowns

### Command Line Options

```bash
mudae-selfbot [OPTIONS]

Options:
  -t, --token <TOKEN>        Your Discord user token
  -c, --channels <CHANNELS>  Channel IDs (comma-separated)
      --no-tui              Disable TUI and use plain logging
      --setup               Force setup wizard even if already configured
```

### Configuration

Settings, statistics, credentials, and channel information are stored in a local SQLite database. You can modify settings through the TUI.

#### Wishlist

The wishlist is stored as a JSON file. Create a `wishlist.json` file in the project root (see `wishlist.example.json` for format):

```json
{
  "characters": [
    {
      "name": "Character Name",
      "series": "Series Name",
      "priority": 1,
      "notes": "Optional notes"
    }
  ]
}
```

## TUI Controls

- **Arrow Keys**: Navigate menus
- **Enter**: Select/Confirm
- **Esc**: Go back/Cancel
- **Tab**: Switch between panels
- **q**: Quit (when in dashboard)

## Project Structure

```
mudae-selfbot/
├── src/
│   ├── main.rs          # Entry point
│   ├── client.rs        # Discord client wrapper
│   ├── commands.rs      # Command execution logic
│   ├── config.rs        # Configuration management
│   ├── database.rs      # SQLite database operations
│   ├── handler.rs       # Message and event handling
│   ├── parser.rs        # Mudae message parsing
│   ├── search.rs        # Character search functionality
│   ├── setup.rs         # Setup wizard
│   ├── stats.rs         # Statistics tracking
│   ├── tui.rs           # Terminal user interface
│   ├── utils.rs         # Utility functions
│   ├── verifier.rs      # Character verification
│   └── wishlist.rs      # Wishlist management
├── schema.sql           # Database schema
├── Cargo.toml           # Rust dependencies
└── README.md            # This file
```

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Disclaimer

This software is provided "as is" without warranty of any kind. Using Discord selfbots violates Discord's Terms of Service. The authors and contributors are not responsible for any account bans or other consequences resulting from the use of this software. Use at your own risk.

## Acknowledgments

- Built with [Serenity](https://github.com/serenity-rs/serenity) (Discord API wrapper)
- TUI powered by [Ratatui](https://github.com/ratatui-org/ratatui)
- Inspired by the frustration of missing out on favorite characters
