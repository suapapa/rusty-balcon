# AGENTS.md

## Project Overview

**Rusty Balcon** is a Rust-based firmware for an ESP32-C3 acting as a 2-key barebones Bluetooth (BLE) keyboard. It utilizes the standard library (`std`) and `esp-idf-svc` for a more robust development environment. The firmware implements a HID Keyboard profile using the NimBLE stack via `esp32-nimble`.

### Key Technologies
- **Rust**: Edition 2024, Standard Library (`std`).
- **Target Architecture**: `riscv32imc-esp-espidf` (ESP32-C3).
- **Framework**: `esp-idf-svc` and `esp-idf-hal`.
- **Bluetooth Stack**: `esp32-nimble` (NimBLE host stack).

## Setup Commands

- **Install Rust**: standard `rustup` toolchain (`1.88+`).
- **Install ESP tooling**: 
  - `cargo install espup`
  - `espup install`
  - `cargo install ldproxy`
- **Set Environment**: Source the export file (e.g., `. $HOME/export-esp.sh`).

## Development Workflow

- The main application entry point is `src/bin/main.rs`.
- Configuration and constants are under `mod config` in `main.rs`.
- `sdkconfig.defaults` contains the necessary ESP-IDF configurations (NimBLE, NVS, etc.).

## Build and Deployment

- **Check Project**: `cargo check` (Uses `-Zbuild-std` as configured in `.cargo/config.toml`).
- **Build**: `cargo build --release`
- **Flash to device**: `cargo run --release` (runs `espflash` per `.cargo/config.toml`).

## Architecture & Code Style Guidelines

- **State Machine**: Periodic polling in a standard loop. States include `Idle`, `Pairing` (Advertising), and `Connected`.
- **NimBLE HID**: Uses `BLEHIDDevice` to send keyboard reports. Input reports must be locked before updating values and notifying.
- **Deep Sleep**: Triggered after `INACTIVITY_TIMEOUT`. Uses `esp_idf_sys` for low-level power management calls.
- **Lint & Format**: Observe `rustfmt` rules. Check with `cargo clippy`.

## Testing Instructions

- Functional tests require physical ESP32-C3 hardware.
- Use standard `println!` for logging (monitored via `espflash`).

## Debugging and Troubleshooting

- **Build Std**: The project uses `build-std` to compile the standard library for the ESP-IDF target. Ensure `rust-src` component is installed.
- **NimBLE Config**: If Bluetooth doesn't start, check `sdkconfig.defaults` for `CONFIG_BT_NIMBLE_ENABLED`.
- **Memory**: The `std` environment has more overhead but handles heap allocations more transparently via the ESP-IDF allocator.
