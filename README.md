# Rusty Balcon

**Rusty Balcon** is a firmware for a 2-key Bluetooth Low Energy (BLE) keyboard built with Rust. It runs on the ESP32-C3 microcontroller using `esp-hal` and the `bleps` Bluetooth stack. The project features Bluetooth HID over GATT (HOGP) communication, LED status indication, and aggressive power-saving via deep sleep.

## Hardware Configuration

- **Microcontroller**: ESP32-C3
- **Inputs**: 2 Push Buttons connected to RTC-capable GPIO pins:
- Key 1: `GPIO1` (Internal Pull-Up) - Maps to `ESC`
- Key 2: `GPIO2` (Internal Pull-Up) - Maps to `Print Screen` (`F13` on Mac)
- **Output**: 1 LED connected to `GPIO8`

## Features

- **Bluetooth HID Keyboard**: Acts as a standard BLE keyboard sending standard USB usage codes.
- **Power Management**: Enters Deep Sleep after 5 minutes of inactivity to conserve battery power. The device wakes up from deep sleep using RTC external interrupts triggered by a button press.
- **Pairing Mode**: Hold both keys simultaneously for 5 seconds to enter pairing mode. The LED will blink synchronously to indicate readiness.
- **Status LED**: 
  - Off: Idle / Deep Sleep
  - Blinking: Pairing Mode
  - Solid On: Connected

## Operating States

1. **Idle / Disconnected**: Device is awake but not connected or advertising. LED is OFF.
2. **Pairing Mode**: Activated by holding both keys for 5 seconds. BLE advertising begins. LED blinks every 500ms.
3. **Connected**: Successfully established a BLE connection. LED stays ON solidly. Pressing keys sends HID reports to the paired system.
4. **Deep Sleep**: Entered after 5 minutes of inactivity to conserve the battery. A hardware interrupt from Key 1 or Key 2 will wake the device up and effectively restart the logic.

## Prerequisites

- [Rust Toolchain](https://rustup.rs/) (version 1.88 is configured in `Cargo.toml`)
- [esp-rs/espup](https://github.com/esp-rs/espup) for configuring the ESP-RS environment
- `cargo-espflash` for flashing the ESP32-C3: (install via `cargo install cargo-espflash`)

## Build and Run

1. Clone the repository and navigate to the project directory.
2. Build the project:
   ```bash
   cargo build --release
   ```
3. Flash and run on the ESP32-C3:
   ```bash
   cargo run --release
   # or
   cargo espflash flash --release --monitor
   ```

## License

This project is open-source and available under the MIT License.
