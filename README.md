# Rusty Balcon

**Rusty Balcon** is a Rust-based firmware for an ESP32-C3 acting as a 2-key barebones Bluetooth (BLE) keyboard. It leverages the standard library (`std`) and `esp-idf-svc` for a robust development environment, implementing a HID Keyboard profile using the NimBLE stack via `esp32-nimble`.

## Hardware Configuration

- **Microcontroller**: ESP32-C3
- **Inputs**: 2 Push Buttons
  - Key 1: `GPIO1` (Internal Pull-Up)
  - Key 2: `GPIO2` (Internal Pull-Up)
- **Output**: 1 LED connected to `GPIO8`

## Features

- **Standard Library (std)**: Robust development environment and memory management.
- **Bluetooth HID Keyboard**: Acts as a standard BLE keyboard using the NimBLE stack.
- **Power Management**: Enters Deep Sleep after 60 seconds of inactivity to conserve battery.
- **Pairing Mode**: Hold both keys simultaneously for 5 seconds to enter pairing mode. The LED blinks to indicate status.
- **Status LED**: 
  - Off: Idle / Deep Sleep
  - Blinking: Pairing Mode (Advertising)
  - Solid On: Connected

## Prerequisites

- [Rust Toolchain](https://rustup.rs/) (1.88+)
- [espup](https://github.com/esp-rs/espup) for ESP-RS toolchain setup
- [ldproxy](https://github.com/esp-rs/ldproxy) for linking
- `espflash` for flashing: `cargo install espflash`

## Build and Run

1.  **Environment Setup**: Ensure your ESP-IDF environment is sourced (e.g., `. $HOME/export-esp.sh`).
2.  **Build**:
    ```bash
    cargo build --release
    ```
3.  **Flash and Run**:
    ```bash
    cargo run --release
    ```

## License

This project is open-source and available under the MIT License.
