# 📡 LogPulse
[![Ask DeepWiki](https://devin.ai/assets/askdeepwiki.png)](https://deepwiki.com/jpmasangkay/logpulse)

### Real-Time Terminal UI for Log Exploration & System Monitoring — Pure Rust

**Tail. Filter. Monitor. Pure Rust.**

A crisp, responsive terminal UI for real-time log exploration and live host/process monitoring built entirely in Rust. Features background-threaded log ingestion over `std::sync::mpsc` (`<FILE>` tailing with 256 KB seek, rotation/truncation detection, and partial-line buffering, `--demo` weighted traffic generator, and `--system` live macOS Unified Log streaming with cross-platform `sysinfo` fallback), structured JSON and plain-text parsing, live case-insensitive search and severity chip filtering, ID-anchored selection when paused, scrollable JSON/process inspection popups, and a live system dashboard tracking CPU, RAM, swap, primary disk, network throughput, 60-second sparklines, and sortable processes.

[![Rust](https://img.shields.io/badge/Rust-1.95+-DEA584?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Edition](https://img.shields.io/badge/Edition-2021-blue?style=for-the-badge)](https://www.rust-lang.org)
[![ratatui](https://img.shields.io/badge/ratatui-0.30.2-00ADD8?style=for-the-badge)](https://ratatui.rs)
[![crossterm](https://img.shields.io/badge/crossterm-0.29.0-333333?style=for-the-badge)](https://github.com/crossterm-rs/crossterm)
[![sysinfo](https://img.shields.io/badge/sysinfo-0.40.3-4E8EE9?style=for-the-badge)](https://github.com/GuillaumeGomez/sysinfo)
[![clap](https://img.shields.io/badge/clap-4.6.8-orange?style=for-the-badge)](https://github.com/clap-rs/clap)

🖥️ **Repository:** [github.com/jpmasangkay/logpulse](https://github.com/jpmasangkay/logpulse)

---

## ✨ Features

### 📜 Real-Time Log Ingestion & Parsing

- **Resilient file tailer (`<FILE>`)** — validates file existence and readability before entering raw mode, seeks roughly the last 256 KB on startup (skipping the initial partial line), polls every ~100 ms for new bytes, buffers incomplete lines until a newline arrives, resets automatically on file truncation or rotation, and decodes non-UTF-8 sequences via `String::from_utf8_lossy`.
- **Structured JSON & plain-text parser** — extracts severity from `level` / `severity` / `lvl` (plus macOS `messageType`), message from `msg` / `message` (plus macOS `eventMessage` with `processImagePath` tags), and timestamps from `timestamp` / `time` / `ts` (RFC 3339, Unix seconds/milliseconds, or `HH:MM:SS`). Plain-text lines detect `ERROR`/`ERR`, `WARN`/`WARNING`, `INFO`, and `DEBUG`/`TRACE` case-insensitively and default to `Info` / current local time.
- **Built-in `--demo` generator** — emits weighted log levels (~60% `INFO`, 20% `DEBUG`, 13% `WARN`, 7% `ERROR`) with ~30% structured JSON and ~70% plain-text lines across multiple fake services with request IDs and random traffic bursts so the 60-second sparkline comes alive.
- **Direct `--system` mode** — launches into the live host/process monitor while streaming macOS Unified Logs (`/usr/bin/log stream --style ndjson --level info`) in the background, with automatic `sysinfo` telemetry fallback on non-macOS or sandboxed environments.

### 🔍 Interactive Filtering, Search & Inspection

- **Real-time `/` search** — filters logs (case-insensitive substring match over the raw line) or processes (by name or PID) on every keystroke, displaying live `matched/total` counts and terminal cursor positioning.
- **Severity filter chips (`1`–`4`)** — toggle `ERROR`, `WARN`, `INFO`, and `DEBUG` visibility independently with color-coded chips in the bottom bar.
- **Pause & ID-anchored scrolling** — `Space` pauses live follow (scrolling up also pauses automatically; `G` resumes). Incoming lines continue ingesting and incrementing totals while the viewport and selection remain anchored to the selected entry `id` (or `pid` in the process table).
- **Centered detail popup (`Enter`)** — clears the underlying area with `Clear`, wraps long text cleanly, and renders full raw log lines plus pretty-printed JSON (or full executable paths, memory, and CPU metrics for processes) with `j`/`k` scrolling.

### 📊 Live System & Process Monitoring

- **1-second host telemetry tick** — monitors global CPU usage (`Gauge`), physical RAM (`used / total GB` and percentage `Gauge`), swap usage, primary disk utilization, 1/5/15m load averages, uptime, and non-loopback network `Rx`/`Tx` byte rates via `sysinfo`.
- **60-second rolling sparkline** — plots `Logs/s` (current and peak rate) in the `Logs` view and `CPU%` history in the `Processes` view.
- **Interactive process table (`Tab`)** — switch between `Logs` and `Processes` in any mode (`--system`, `--demo`, or `<FILE>`), sort live processes by `CPU%` (`c`) or `MEM` (`m`), filter by name/PID, and inspect process details.

### 🛡️ Terminal Safety & Responsive Layout

- **Panic-safe terminal lifecycle** — uses `ratatui::try_init()` and `ratatui::restore()` to manage raw mode, alternate screen, and panic hooks cleanly on normal exit, error exit, `Ctrl+C`, or panic.
- **Adaptive layout** — 75/25 split with rounded borders, severity background chips, compact layout adaptation below 20 rows, and a friendly `"Terminal too small"` guard below `60×16`.

---

## 🛠️ Tech Stack & Source Layout

| Component / Crate | Purpose |
|-------------------|---------|
| **[src/main.rs](src/main.rs)** | `clap` CLI parsing (`<FILE>`, `--demo`, `--system`), pre-TUI file validation, terminal init/restore, and 50 ms `crossterm` event loop |
| **[src/app.rs](src/app.rs)** | Application state, `VecDeque` 10,000-entry ring buffer, JSON & plain-text parsers, file tailer, demo generator, `--system` streamer, and `sysinfo` polling |
| **[src/ui.rs](src/ui.rs)** | All `ratatui` layout and widget rendering (`Table`, `Gauge`, `Sparkline`, `Paragraph`, `Clear` popups, and compact/small-terminal guards) |
| **Rust 2021 Edition (1.95+)** | Memory-safe systems programming with owned types (`String`, `Vec`, `VecDeque`) and `anyhow` error propagation |
| **ratatui `0.30.2`** | Terminal UI framework powering the layout, stateful tables, gauges, sparklines, and overlays |
| **crossterm `0.29.0`** | Cross-platform terminal raw mode, alternate screen, resize, and `KeyEventKind::Press` input handling |
| **sysinfo `0.40.3`** | Cross-platform CPU, RAM, swap, disk, network, load average, and process metrics |
| **clap `4.6.8` (`derive`)** | Declarative CLI argument parsing with mutually exclusive required source group |
| **serde `1.0.229` & serde_json `1.0.154`** | JSON log deserialization and pretty-printed detail inspection |
| **chrono `0.4.45`** | RFC 3339, Unix epoch, and local timestamp parsing and formatting |
| **rand `0.9.5`** | Weighted log level, service, latency, and burst timing generation in `--demo` mode |
| **anyhow `1.0.104`** | Ergonomic contextual error handling across CLI and terminal setup |

---

## 🚀 Getting Started

### Prerequisites

- [Rust SDK](https://rustup.rs/) (1.95 or later with `cargo` and `clippy`)
- macOS, Linux, or Windows terminal (minimum `60×16` recommended)

### Installation & Verification

```bash
# Clone the repository
git clone https://github.com/jpmasangkay/logpulse.git
cd logpulse

# Build the release binary
cargo build --release

# Run unit tests and Clippy lints
cargo test
cargo clippy --all-targets -- -D warnings
```

### Running LogPulse

```bash
# Monitor your machine live (processes, CPU/RAM/disk/network, and system logs)
cargo run -- --system

# Run with the built-in simulated traffic generator
cargo run -- --demo

# Tail a real log file (supports plain text, JSON/NDJSON, truncation, and rotation)
cargo run -- /path/to/app.log
```

---

## ⌨️ Controls

### Global & Navigation

| Action | Shortcut |
|--------|----------|
| **Switch Logs / Processes Tab** | `Tab` |
| **Enter Filter / Search Mode** | `/` |
| **Move Selection Down / Up** | `j` / `k` or `Down` / `Up` |
| **Page Down / Page Up** | `PageDown` / `PageUp` |
| **Jump to Top (pauses in Logs)** | `g` |
| **Jump to Bottom (resumes `● LIVE` in Logs)** | `G` |
| **Pause / Resume Updates** | `Space` |
| **Inspect Selected Entry / Process** | `Enter` |
| **Quit** | `q` or `Ctrl+C` |

### View-Specific, Search & Popup

| Context | Action | Shortcut |
|---------|--------|----------|
| **Logs View** | Toggle `ERROR` / `WARN` / `INFO` / `DEBUG` | `1` / `2` / `3` / `4` |
| **Processes View** | Sort by `CPU%` / `Memory` | `c` / `m` |
| **Search Mode** | Type / edit query | Characters, `Backspace` |
| **Search Mode** | Confirm filter / clear & exit | `Enter` / `Esc` |
| **Detail Popup** | Scroll content | `j` / `k`, `Up` / `Down`, `PageUp` / `PageDown`, `g` / `G` |
| **Detail Popup** | Close popup | `Esc` or `Enter` |

---

## 👥 Author

Built with ❤️ by **John Paul Masangkay** ([@jpmasangkay](https://github.com/jpmasangkay))

---

## 👏 Acknowledgements

- **Ratatui & Crossterm maintainers** — For a fast, ergonomic Rust terminal UI ecosystem
- **Guillaume Gomez (`sysinfo`)** — For cross-platform host and process telemetry in pure Rust
- **The Rust Community** — For `clap`, `serde`, `serde_json`, `chrono`, `rand`, and `anyhow`
