# logpulse

`logpulse` is a terminal log tailer and host/process monitor with live filtering, structured JSON inspection, and a 60-second rate/CPU sparkline. It is written in Rust using `ratatui`, `crossterm`, and `sysinfo` on top of standard OS threads and `std::sync::mpsc`.

<!-- TODO: drop docs/demo.gif or docs/screenshot.png here -->

## Why I Built This

When debugging services locally, I kept switching between `tail -f | jq`, `grep`, and `htop` across terminal splits just to check whether an error burst lined up with a CPU or memory spike. I wanted a single binary that opens directly on a log file, synthetic traffic, or live host telemetry without leaving the terminal or maintaining separate filter pipelines. Bounding history to 10,000 buffered entries and sampling host metrics once per second keeps memory predictable during long sessions while letting me pause, hold selection on a single log ID, and inspect nested JSON payloads in place.

## Quick Start

```bash
cargo build --release
cargo test
cargo clippy --all-targets -- -D warnings

cargo run --release -- /var/log/app.log
cargo run --release -- --system
cargo run --release -- --demo
```

<details>
<summary>Platform notes, JSON fields, and terminal constraints</summary>

- `--system` streams macOS Unified Logs via `/usr/bin/log stream --style ndjson --level info`. Where `/usr/bin/log` is missing or exits early (Linux, Windows, or restricted sandboxes), `logpulse` switches to emitting `sysinfo` JSON host/top-process snapshots every 2 seconds while keeping the `Processes` tab live.
- JSON lines inspect `level`/`severity`/`lvl`/`messageType`, `msg`/`message`/`eventMessage` (with `processImagePath` tags for Unified Logs), and `timestamp`/`time`/`ts` (RFC 3339, Unix seconds/milliseconds, or `HH:MM:SS`). Plain-text lines match `ERROR`/`WARN`/`INFO`/`DEBUG` tokens case-insensitively and default to `INFO` at local arrival time.
- Requires at least a `60x16` terminal (`< 20` rows triggers compact layout; smaller viewports render a resize prompt).

</details>

## Measured Numbers

Measured locally on Apple A18 Pro (6 cores, 8 GiB RAM), macOS 27.2.1 (26B131), `rustc 1.100.0`:

```text
release binary (target/release/logpulse)   2,131,120 bytes (2.0M)
source footprint (Cargo.toml + src/*.rs)   3,213 lines
cargo test --release                       5 passed, 0 failed, 0 ignored (0.04s)
cargo clippy --all-targets -- -D warnings  0 warnings
cold start to first PTY frame (120x40)     27.1 ms (--system, 11.8 MiB RSS)
                                           34.5 ms (--demo, 11.3 MiB RSS)
10,000-line tail burst (721,890 bytes)     163.7 ms append-to-render (includes 100 ms poll), 22.7 MiB RSS
```

## Architecture & Trade-offs

Each input mode (`<FILE>`, `--demo`, or `--system`) spawns one background `std::thread` pushing parsed lines over an unbounded `std::sync::mpsc` channel, while the main thread polls `crossterm` input every 50 ms and refreshes host metrics once per second. Skipping an async runtime keeps the release binary at 2.0M and the event loop synchronous, at the cost of fixed polling intervals (100 ms in the file tailer, 50 ms UI drain) and no channel backpressure if a writer outpaces the main thread.

Buffered history lives in an in-memory `VecDeque` capped at `10,000` entries, and opening an existing file seeks back `256 KiB` from EOF and drops the leading partial line. Avoiding a disk index keeps memory bounded near 23 MiB at full capacity, though entries older than the buffer window or before the initial `256 KiB` offset cannot be scrolled back.

When new entries arrive or the search/severity filters change, `drain_incoming` scans the buffer to rebuild the filtered index list, and `render_log_table` allocates table rows across all matched indices. While paused (`Space` or scrolling up), navigation tracks the selected item ID (or process PID) instead of visual row offset so ring-buffer evictions do not shift what you are reading. Full-buffer scans and row allocations are less optimal than incremental indices or viewport-only slicing, but keep state management simple within the 10,000-entry limit.

Host and process telemetry are initialized upfront in `App::new` regardless of CLI flag so switching tabs with `Tab` populates `Processes` immediately, with per-process `CPU%` starting at `0.0` until the first 1 s sample delta completes. Disk utilization inspects only `/` (or the first enumerated volume) to prevent macOS APFS container double-counting, and `--system` wraps `/usr/bin/log` with a `sysinfo` JSON snapshot fallback instead of integrating native `journald` or Windows Event Log APIs.

## Known Limits & Rough Edges

- History stops at 10,000 buffered entries in memory; there is no disk-backed scrollback once lines are evicted or before the startup 256 KiB seek.
- The file tailer polls `metadata().len()` every 100 ms rather than watching filesystem events and only resets to byte `0` when a file shrinks (`len < pos`), so an in-place replacement with an equal-or-larger file goes undetected.
- Live OS log streaming relies on `/usr/bin/log` on macOS; non-macOS hosts only receive 2-second `sysinfo` host snapshots in the `Logs` tab under `--system`.
- Only the primary mount (`/`) is measured in the disk gauge, and both pause state (`paused`) and the `/` search string carry across `Logs` and `Processes` instead of staying scoped per tab.
- Runtime behavior and unit tests (`cargo test`) have only been executed on macOS `arm64`; Linux and Windows behavior relies on `crossterm` and `sysinfo` without direct verification on those OSes.

## Controls

| Key | Scope | Action |
|-----|-------|--------|
| `Tab` | Global | Switch between `Logs` and `Processes` |
| `/` | Global | Edit search filter (`Enter` keeps filter, `Esc` clears) |
| `1`-`4` | Logs | Toggle `ERROR`, `WARN`, `INFO`, `DEBUG` visibility |
| `c` / `m` | Processes | Sort table by `CPU%` or `Memory` |
| `j` / `k`, `Up` / `Down` | List / Popup | Move selection or scroll detail popup |
| `PageUp` / `PageDown` | List / Popup | Scroll by page |
| `g` / `G` | List / Popup | Jump to top (pauses in Logs) or bottom (resumes live in Logs) |
| `Space` | Global | Pause or resume live updates |
| `Enter` | List / Popup | Open or close inspection popup |
| `q` / `Ctrl+C` | Global | Exit cleanly |
