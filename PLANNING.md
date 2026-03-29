# 2-Key Bluetooth Keyboard Implementation Plan

## Overview
This document outlines the plan for building a 2-key Bluetooth (BLE) keyboard using the ESP32-C3 microcontroller, `esp-hal` (version `~1.0`), and the `bleps` Bluetooth stack. The firmware will manage key inputs, Bluetooth HID over GATT (HOGP) communication, LED status indication, and aggressive power saving via Deep Sleep.

## 1. Hardware Definition
- **MCU**: ESP32-C3 (configured via `cargo` and `esp-hal`).
- **Inputs**: 2 Push Buttons connected to RTC-capable GPIO pins to allow waking up from deep sleep. Keys should use internal pull-up resistors (`Input<'d, PullUp>`).
- **Outputs**: 1 LED connected to a standard output GPIO pin.
- **Power**: Battery-operated, necessitating power-saving states.

## 2. State Machine Design
The device operates in one of the following main states:
1. **Idle / Disconnected**: Device is awake but not connected or advertising. LED is OFF.
2. **Pairing Mode**: Activated by holding both keys for 5 seconds. BLE advertising begins. LED blinks synchronously.
3. **Connected**: Device successfully established a BLE connection with a host (PC/Phone). LED stays ON solidly.
4. **Deep Sleep**: Entered after 5 minutes of total inactivity to conserve battery. Device halts execution and waits for a hardware interrupt (RTC wakeup) to reboot.

## 3. Core Features & Implementation Details

### A. Key Input & Long-Press Detection
- Use non-blocking timers (`esp_hal::time::Instant`) to track button states instead of blocking delays.
- Track variables for when both keys were first depressed.
- If both buttons remain depressed continuously for `5000` milliseconds, trigger the transition to **Pairing Mode**.
- Reset the hold timer immediately if either button is released.

### B. LED Status Control
- **State: Pairing**: Implement a periodic blink (e.g., toggle every 500ms) comparing `Instant::now()` against a stored timestamp.
- **State: Connected**: Assert the GPIO high to keep the LED ON.
- **State: Deep Sleep / Idle**: De-assert the GPIO to keep the LED OFF.

### C. Bluetooth & HID functionality (`bleps`)
- **BLE Stack**: Utilize `esp_radio::init()` and `BleConnector::new()` as the transport layer for `bleps`.
- **GATT Attributes**: Configure standard BLE HID attributes:
  - **Device Information Service** (0x180A): Manufacturer, PnP ID.
  - **Battery Service** (0x180F): Optional, but highly recommended for battery devices.
  - **Human Interface Device Service** (0x1812): Host the HID Report Map (defining the device as a typical Keyboard) and characteristic endpoints for Input Reports.
- **Advertising**: Start advertising with "Discoverable" flags and the HID appearance UUID when Pairing Mode is active.
- **Event Handling**: On a valid keypress (in the Connected state), generate an HID Report array mapping the respective key to a standard USB usage code (e.g., `Enter` or `Space`), and notify the GATT characteristic. Send an empty report when the key is released.

### D. Deep Sleep & Battery Management
- Maintain a `last_activity` timestamp. Update this timestamp whenever:
  - A button state changes (press or release).
  - A BLE connection event occurs.
- Monitor `Instant::now().duration_since(last_activity)`. If this exceeds 5 minutes (300 seconds), prepare for sleep.
- **Entering Deep Sleep**:
  1. Gracefully terminate BLE connections.
  2. Turn off the LED.
  3. Configure the specific GPIO pins for the 2 keys as wakeup sources utilizing ESP32-C3's `Ext1WakeupSource` via the RTC controller.
  4. Invoke `Rtc::sleep_deep()`. The MCU will power down and effectively reset upon the next key press.

## 4. Software Architecture Outline
Due to the presence of `bleps` with the `async` feature enabled, the architecture can neatly fit into an async executor loop or a fast concurrent polled loop.

```rust
// Draft Pseudocode Architecture
let mut state = State::Idle;
let mut last_activity = Instant::now();
let mut hold_start: Option<Instant> = None;

loop {
    let now = Instant::now();
    let k1_pressed = key1.is_low();
    let k2_pressed = key2.is_low();

    // 1. Inactivity Tracker
    if k1_pressed || k2_pressed /* or ble activity */ {
        last_activity = now;
    }

    // 2. Deep Sleep Transition
    if now.duration_since(last_activity) > Duration::from_secs(300) {
        // Setup RTC Wakeup for key1_pin and key2_pin
        // Enter deep sleep
    }

    // 3. 5-Second Hold detection
    if k1_pressed && k2_pressed {
        if let Some(start) = hold_start {
            if now.duration_since(start) >= Duration::from_secs(5) {
                state = State::Pairing;
                start_ble_advertising(); 
                hold_start = None; // Reset so it fires only once
            }
        } else {
            hold_start = Some(now);
        }
    } else {
        hold_start = None;
        
        // Handle normal key presses when connected
        if state == State::Connected {
            send_hid_report(k1_pressed, k2_pressed);
        }
    }

    // 4. LED & State Management
    match state {
        State::Idle => led.set_low(),
        State::Pairing => {
            // Blink LED every 500ms
            if (now.as_millis() / 500) % 2 == 0 {
                led.set_high();
            } else {
                led.set_low();
            }
            if ble_is_connected() {
                state = State::Connected;
            }
        },
        State::Connected => {
            led.set_high(); // Solid glow
            if !ble_is_connected() {
                state = State::Idle;
            }
        }
    }
    
    // 5. Yield/Process BLE Background Tasks
    // ble_executor.poll();
}
```

## Next Steps for Development
1. Define the explicit GPIO mappings for the ESP32-C3 in `main.rs`.
2. Construct the BLE HID Report Map for a standard keyboard in `bleps`.
3. Implement the timer-based state machine logic for debouncing and hold detection.
4. Implement the deep sleep logic using `esp_hal::rtc_cntl::Rtc`.
