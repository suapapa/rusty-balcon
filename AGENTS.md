# AGENTS.md

## Project Overview

**Rusty Balcon** is a Rust-based firmware for an ESP32-C3 acting as a 2-key barebones Bluetooth (BLE) keyboard. It leverages `esp-hal` (no_std) and the `bleps` crate for BLE functionality and HID over GATT (HOGP) capabilities. Key operational aspects involve state management handling pairing, and power management focusing on Deep Sleep and RTC generic wakeup.

### Key Technologies
- **Rust**: Edition 2024, utilizing bare-metal paradigms (`no_std`, `no_main`).
- **Target Architecture**: `riscv32imc-unknown-none-elf` (ESP32-C3).
- **HAL/Framework**: `esp-hal` (version `~1.0`).
- **Bluetooth Stack**: `bleps` and `esp-radio` (with `ble` and `async` features enabled).

## Setup Commands

- **Install Rust**: standard `rustup` toolchain (`1.88` is targeted).
- **Install ESP tooling**: Install standard `esp-rs/espup` and `cargo-espflash`.
- **Set Environment**: Make sure the standard ESP-IDF environment variables are exported (e.g., `. $HOME/export-esp.sh` if applicable, although this is mostly direct `esp-hal`).

## Development Workflow

- The main application entry point is `src/bin/main.rs`.
- Configuration and constants (e.g., Deep Sleep timeout, key mappings, timeouts) are consolidated under the `config` module inside `src/bin/main.rs`.
- The `hid` module holds the BLE Keyboard report descriptor map. Keep it compliant with standard USB HID usages.

## Build and Deployment

- **Check Project**: `cargo check --release` (always test with `--release` as debug can be physically constrained on the MCU).
- **Build**: `cargo build --release`
- **Flash to device**: `cargo run --release` (runs `espflash` per `.cargo/config.toml`) or `cargo espflash flash --release --monitor`.

## Architecture & Code Style Guidelines

- **State Machine**: The application logic runs on a continuous loop tracking state (Idle, Pairing, Connected) while comparing `Instant::now()` variables against `last_activity` timestamps for transitions.
- **Asynchronous Bluetooth**: The `bleps` framework handles characteristics closures via `do_work()` and `do_work_with_notification()`. Take note of stringent closure lifetimes and avoid moving borrowed context.
- **Deep Sleep**: Use `esp_hal::rtc_cntl::Rtc`. Wakeup from deep sleep relies on routing standard keys (`GPIO1` and `GPIO2`) into `RtcioWakeupSource`.
- **Lint & Format**: Observe `rustfmt` rules. Check code strictly with `cargo clippy --release`.

## Testing Instructions

- Unit tests (`cargo test`) are challenging due to `no_std` and embedded framework requirements. Most functional tests are inherently manual requiring physical ESP32-C3 hardware.
- Use `esp_println::println!` calls generously alongside JTAG/Serial monitoring (`--monitor` flag via `espflash`) to evaluate logs and debug connectivity problems.
- If designing generic business logic (e.g., purely math or byte parsing), separate it into `src/lib.rs` and apply `#[cfg(test)]` bounds so `cargo test --lib` works directly on the host.

## Debugging and Troubleshooting

- **Out of Memory Panic**: Look out for heap allocation panics! `esp_alloc` is given `66320` bytes. The application attempts minimal global allocations since most `bleps` allocations wrap inside `Box::leak` for the stack. 
- **Borrowing / Closure Lifetimes**: Handling variables via closures (for `gatt!`) within the continuous runtime loop is an ongoing challenge in Rust Embedded. Pay specific heed to variables getting unnecessarily `moved` leading to lifetime issues across loop iterations. 
- **Wakeup Pins**: Any modified setup for `RtcioWakeupSource` MUST ensure the targeted ESP32-C3 GPIO physically supports RTC operations. Only certain pins support waking the chip.
