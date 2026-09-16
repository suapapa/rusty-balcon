use anyhow::Result;
use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::gpio::*;
use esp_idf_hal::i2c::*;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::units::*;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs};
use esp32_nimble::{BLEAdvertisementData, BLEDevice, BLEHIDDevice, NimbleProperties, enums::*};
use smart_leds::{RGB8, SmartLedsWrite};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ws2812_esp32_rmt_driver::Ws2812Esp32Rmt;

use embedded_graphics::{
    mono_font::{
        MonoTextStyle, MonoTextStyleBuilder,
        ascii::{FONT_6X10, FONT_8X13},
    },
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{Line, PrimitiveStyle, Rectangle},
    text::{Alignment, Text},
};
use sh1106::Builder;
use sh1106::prelude::GraphicsMode;

mod config {
    use std::time::Duration;
    // Deep sleep only when Idle (not Connected). Keep BLE alive longer so keys respond immediately.
    pub const INACTIVITY_TIMEOUT: Duration = Duration::from_secs(1800); // 30 minutes
    pub const DISPLAY_TIMEOUT: Duration = Duration::from_secs(30); // 30 seconds
    pub const PAIRING_HOLD_DURATION: Duration = Duration::from_secs(5);
    pub const MODE_HOLD_DURATION: Duration = Duration::from_secs(5);
    pub const KEY_FLASH_DURATION: Duration = Duration::from_millis(80);
    pub const KEY_ENTER: u8 = 0x28; // Enter / Return
    pub const KEY_ESC: u8 = 0x29; // Escape
    // Keep onboard WS2812 dim — full white is a noticeable battery drain.
    pub const LED_KEY_WHITE: u8 = 48;
    pub const LED_PAIRING_BLUE: u8 = 40;
}

/// Blue heartbeat envelope: double-beat then rest, period 1.2s.
fn pairing_heartbeat_level(elapsed_ms: u128) -> u8 {
    let t = (elapsed_ms % 1200) as u32;
    let peak = config::LED_PAIRING_BLUE as u32;
    let envelope = |phase: u32, width: u32| -> u32 {
        if phase >= width {
            return 0;
        }
        // Triangle pulse
        let half = width / 2;
        if phase < half {
            phase * peak / half
        } else {
            (width - phase) * peak / half
        }
    };
    let level = match t {
        0..=149 => envelope(t, 150),
        150..=249 => 0,
        250..=369 => envelope(t - 250, 120) * 7 / 10, // softer second beat
        _ => 0,
    };
    level as u8
}

fn set_rgb_led(led: &mut Ws2812Esp32Rmt<'_>, color: RGB8) {
    let _ = led.write(std::iter::once(color));
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum Key2Mode {
    Voice,
    Esc,
}

impl Key2Mode {
    pub fn label(&self) -> &'static str {
        match self {
            Key2Mode::Voice => "MIC",
            Key2Mode::Esc => "ESC",
        }
    }

    pub fn toggle(&self) -> Self {
        match self {
            Key2Mode::Voice => Key2Mode::Esc,
            Key2Mode::Esc => Key2Mode::Voice,
        }
    }

    pub fn to_u8(&self) -> u8 {
        match self {
            Key2Mode::Voice => 0,
            Key2Mode::Esc => 1,
        }
    }

    pub fn from_u8(val: u8) -> Self {
        match val {
            1 => Key2Mode::Esc,
            _ => Key2Mode::Voice,
        }
    }
}

mod hid {
    pub const REPORT_DESCRIPTOR: &[u8] = &[
        // Keyboard Report (ID 1)
        0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x85, 0x01, 0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, 0x15,
        0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0x95, 0x01, 0x75, 0x08, 0x81, 0x01,
        0x05, 0x07, 0x19, 0x00, 0x29, 0x65, 0x15, 0x00, 0x25, 0x65, 0x75, 0x08, 0x95, 0x06, 0x81,
        0x00, 0xc0, // End Collection (keyboard)
        // Consumer Control Report (ID 2)
        // bit0: Voice Command (0xCF), bit1: Globe / AC Keyboard Layout Select (0x029D)
        0x05, 0x0c, 0x09, 0x01, 0xa1, 0x01, 0x85, 0x02, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95,
        0x02, 0x09, 0xcf, 0x0a, 0x9d, 0x02, 0x81, 0x02, 0x75, 0x01, 0x95, 0x06, 0x81, 0x03, 0xc0,
    ];

    pub fn create_keyboard_report(enter_pressed: bool, esc_pressed: bool) -> [u8; 8] {
        let mut report = [0u8; 8];
        let mut idx = 2;
        if enter_pressed {
            report[idx] = crate::config::KEY_ENTER;
            idx += 1;
        }
        if esc_pressed {
            report[idx] = crate::config::KEY_ESC;
        }
        report
    }

    pub fn create_consumer_report(globe_pressed: bool, voice_pressed: bool) -> [u8; 1] {
        let mut bits = 0u8;
        if voice_pressed {
            bits |= 1 << 0; // Voice Command (0xCF)
        }
        if globe_pressed {
            bits |= 1 << 1; // Globe (0x029D)
        }
        [bits]
    }
}

#[derive(PartialEq, Clone, Copy, Debug)]
enum MachineState {
    Idle,
    Pairing,
    Connected,
}

fn main() -> Result<()> {
    esp_idf_sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    // Suppress verbose NimBLE/GATT info logs
    unsafe {
        let tag = std::ffi::CString::new("NimBLE").unwrap();
        esp_idf_sys::esp_log_level_set(tag.as_ptr(), esp_idf_sys::esp_log_level_t_ESP_LOG_WARN);
    }

    // ESP32-H2 EXT1 wakeup GPIOs are RTC IOs 7–14. Keys use GPIO13 / GPIO11 / GPIO12.
    const KEY1_GPIO: u32 = 13;
    const KEY2_GPIO: u32 = 11;
    const KEY3_GPIO: u32 = 12;

    // Check wakeup cause before taking peripherals (must be called early)
    let wakeup_gpio_status: u64 = unsafe {
        let cause = esp_idf_sys::esp_sleep_get_wakeup_cause();
        if cause == esp_idf_sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_EXT1 {
            esp_idf_sys::esp_sleep_get_ext1_wakeup_status()
        } else {
            0
        }
    };
    let wakeup_pending: Option<(bool, bool, bool)> = if wakeup_gpio_status != 0 {
        Some((
            (wakeup_gpio_status & (1 << KEY1_GPIO)) != 0,
            (wakeup_gpio_status & (1 << KEY2_GPIO)) != 0,
            (wakeup_gpio_status & (1 << KEY3_GPIO)) != 0,
        ))
    } else {
        None
    };

    let peripherals = Peripherals::take().unwrap();
    let _sysloop = EspSystemEventLoop::take()?;
    let nvs_default = EspDefaultNvsPartition::take()?;
    let mut nvs = match EspNvs::new(nvs_default, "config", true) {
        Ok(nvs) => Some(nvs),
        Err(e) => {
            println!("Failed to initialize NVS: {:?}", e);
            None
        }
    };

    let mut key2_mode = if let Some(nvs) = &nvs {
        match nvs.get_u8("k2_mode") {
            Ok(Some(val)) => Key2Mode::from_u8(val),
            Ok(None) => Key2Mode::Voice,
            Err(e) => {
                println!("Failed to read k2_mode from NVS: {:?}", e);
                Key2Mode::Voice
            }
        }
    } else {
        Key2Mode::Voice
    };
    println!("Initial Key 2 Mode: {:?}", key2_mode);

    // GPIO Setup (RTC-capable pins required for deep-sleep EXT1 wakeup)
    let key1 = PinDriver::input(peripherals.pins.gpio13, Pull::Up)?;
    let key2 = PinDriver::input(peripherals.pins.gpio11, Pull::Up)?;
    let key3 = PinDriver::input(peripherals.pins.gpio12, Pull::Up)?;

    // Onboard WS2812 RGB LED (ESP32-H2 SuperMini → GPIO8)
    // ws2812-esp32-rmt-driver 0.14 still uses the legacy RMT channel API.
    #[allow(deprecated)]
    let mut rgb_led = Ws2812Esp32Rmt::new(peripherals.rmt.channel0, peripherals.pins.gpio8)?;
    set_rgb_led(&mut rgb_led, RGB8::new(0, 0, 0));

    // I2C & Display Setup
    let sda = peripherals.pins.gpio4;
    let scl = peripherals.pins.gpio5;
    let i2c_config = I2cConfig::new().baudrate(100u32.kHz().into());
    let i2c_driver = I2cDriver::new(peripherals.i2c0, sda, scl, &i2c_config)?;

    let mut display: GraphicsMode<_> = Builder::new().connect_i2c(i2c_driver).into();

    display.init().unwrap_or_else(|e| {
        println!("Display init error: {:?}", e);
    });
    let _ = display.set_contrast(0x10); // Dim the display to save power
    let _ = display.clear();
    let welcome_style = MonoTextStyle::new(&FONT_8X13, BinaryColor::On);
    let tag_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

    let _ = Text::with_alignment(
        "RUSTY BALCON",
        Point::new(64, 30),
        welcome_style,
        Alignment::Center,
    )
    .draw(&mut display);
    let _ = Text::with_alignment(
        env!("GIT_TAG"),
        Point::new(64, 45),
        tag_style,
        Alignment::Center,
    )
    .draw(&mut display);
    let _ = display.flush();
    FreeRtos::delay_ms(1500);

    // BLE Setup
    let _ = BLEDevice::set_device_name("Rusty-Balcon");
    let device = BLEDevice::take();
    device
        .security()
        .set_auth(AuthReq::all())
        .set_io_cap(SecurityIOCap::NoInputNoOutput)
        .resolve_rpa();

    let server = device.get_server();

    // Mandatory for macOS/iOS HID: Device Info Service
    let info_service = server.create_service(esp32_nimble::utilities::BleUuid::from_uuid16(0x180A));
    info_service
        .lock()
        .create_characteristic(
            esp32_nimble::utilities::BleUuid::from_uuid16(0x2A29),
            NimbleProperties::READ,
        )
        .lock()
        .set_value(b"Rusty-Balcon Team");
    info_service
        .lock()
        .create_characteristic(
            esp32_nimble::utilities::BleUuid::from_uuid16(0x2A24),
            NimbleProperties::READ,
        )
        .lock()
        .set_value(b"RB-01");

    // Battery Service (Ensures macOS sees it as a proper peripheral)
    let battery_service =
        server.create_service(esp32_nimble::utilities::BleUuid::from_uuid16(0x180F));
    battery_service
        .lock()
        .create_characteristic(
            esp32_nimble::utilities::BleUuid::from_uuid16(0x2A19),
            NimbleProperties::READ | NimbleProperties::READ_ENC | NimbleProperties::NOTIFY,
        )
        .lock()
        .set_value(&[100]);

    let mut hid = BLEHIDDevice::new(server);
    hid.report_map(hid::REPORT_DESCRIPTOR);
    hid.pnp(0x02, 0x05ac, 0x820a, 0x0210); // Apple Magic Keyboard mock
    hid.set_battery_level(100);

    let keyboard_report = hid.input_report(1);
    let consumer_report = hid.input_report(2);

    let advertising = device.get_advertising();
    let mut ad_data = BLEAdvertisementData::new();
    ad_data
        .name("Rusty-Balcon")
        .appearance(0x03C1) // Keyboard
        .add_service_uuid(esp32_nimble::utilities::BleUuid::from_uuid16(0x1812));
    advertising.lock().set_data(&mut ad_data)?;
    println!("Starting advertising on boot...");
    if let Err(e) = advertising.lock().start() {
        println!("Failed to start advertising: {:?}", e);
    }

    let mut state = MachineState::Idle;
    let server_arc = Arc::new(Mutex::new(MachineState::Idle));
    let server_arc_clone = server_arc.clone();

    server.on_connect(move |server, desc| {
        println!("BLE Connected: {:?}", desc);
        let mut s = server_arc_clone.lock().unwrap();
        *s = MachineState::Connected;

        // macOS prefers specific connection parameters for HID
        if let Err(e) = server.update_conn_params(desc.conn_handle(), 12, 12, 0, 400) {
            println!("Failed to update conn params: {:?}", e);
        }
    });

    let server_arc_clone2 = server_arc.clone();
    server.on_disconnect(move |desc, reason| {
        println!("BLE Disconnected: {:?}, reason: {:?}", desc, reason);
        let mut s = server_arc_clone2.lock().unwrap();
        *s = MachineState::Idle;
    });

    let mut last_activity = Instant::now();
    let mut pairing_hold_start: Option<Instant> = None;
    let mut mode_hold_start: Option<Instant> = None;
    let mut mode_hold_triggered = false;
    let mut mode_toast_until: Option<Instant> = None;
    let mut last_display_state = (MachineState::Idle, true, true, true, Key2Mode::Voice, true, false); // Force initial draw
    let mut last_kb_report = [0u8; 8];
    let mut last_cons_report = [0u8; 1];
    let mut display_is_on = true;
    let blink_timer = Instant::now();
    let mut wakeup_key_to_send = wakeup_pending; // pending key from deep sleep wakeup
    let mut connect_time: Option<Instant> = None; // time of last BLE connection
    let mut prev_any_key = false;
    let mut key_flash_until: Option<Instant> = None;
    let mut last_led_color = RGB8::new(0, 0, 0);

    loop {
        let now = Instant::now();
        let k1_p = key1.is_low();
        let k2_p = key2.is_low();
        let k3_p = key3.is_low();

        let new_state = *server_arc.lock().unwrap();
        if new_state != state {
            println!("State changed: {:?} -> {:?}", state, new_state);
            if new_state == MachineState::Connected {
                connect_time = Some(now);
            }
            state = new_state;
        }

        if k1_p || k2_p || k3_p {
            last_activity = now;
        }

        // White flash on any key press edge (same for all keys)
        let any_key = k1_p || k2_p || k3_p;
        if any_key && !prev_any_key {
            key_flash_until = Some(now + config::KEY_FLASH_DURATION);
        }
        prev_any_key = any_key;

        // Status LED: key flash > pairing heartbeat > off
        let led_color = if key_flash_until.is_some_and(|until| now < until) {
            let w = config::LED_KEY_WHITE;
            RGB8::new(w, w, w)
        } else {
            key_flash_until = None;
            if state == MachineState::Pairing {
                let level = pairing_heartbeat_level(now.duration_since(blink_timer).as_millis());
                RGB8::new(0, 0, level)
            } else {
                RGB8::new(0, 0, 0)
            }
        };
        // Heartbeat needs frequent writes; otherwise only update on change.
        let force_led_update = state == MachineState::Pairing && key_flash_until.is_none();
        if force_led_update || led_color != last_led_color {
            set_rgb_led(&mut rgb_led, led_color);
            last_led_color = led_color;
        }

        // Deep sleep only when Idle + inactive. Stay awake while Connected so BLE stays hot.
        if state == MachineState::Idle
            && now.duration_since(last_activity) >= config::INACTIVITY_TIMEOUT
        {
            println!("No activity for 30m while Idle. Entering deep sleep...");
            set_rgb_led(&mut rgb_led, RGB8::new(0, 0, 0));
            let _ = display.clear();
            let _ = display.flush();
            FreeRtos::delay_ms(100);

            unsafe {
                // Wake on GPIO13 / GPIO11 / GPIO12 low (ESP32-H2 EXT1 / RTC IO 7–14)
                const WAKEUP_PIN_MASK: u64 =
                    (1 << KEY1_GPIO) | (1 << KEY2_GPIO) | (1 << KEY3_GPIO);
                esp_idf_sys::esp_sleep_enable_ext1_wakeup(
                    WAKEUP_PIN_MASK,
                    esp_idf_sys::esp_sleep_ext1_wakeup_mode_t_ESP_EXT1_WAKEUP_ANY_LOW,
                );
                esp_idf_sys::esp_deep_sleep_start();
            }
        }

        // Key 2 Mode Toggle (Hold Key 1 + Key 2 for 5s)
        if k1_p && k2_p && !k3_p {
            if !mode_hold_triggered {
                if let Some(start) = mode_hold_start {
                    if now.duration_since(start) >= config::MODE_HOLD_DURATION {
                        key2_mode = key2_mode.toggle();
                        println!("Key 2 Mode changed to: {:?}", key2_mode);
                        if let Some(nvs) = &mut nvs {
                            if let Err(e) = nvs.set_u8("k2_mode", key2_mode.to_u8()) {
                                println!("Failed to save k2_mode to NVS: {:?}", e);
                            }
                        }
                        mode_hold_triggered = true;
                        mode_toast_until = Some(now + Duration::from_secs(2));
                        key_flash_until = Some(now + Duration::from_millis(300));

                        // Release any currently pressed key reports immediately
                        if state == MachineState::Connected {
                            keyboard_report.lock().set_value(&[0u8; 8]);
                            keyboard_report.lock().notify();
                            consumer_report.lock().set_value(&[0u8; 1]);
                            consumer_report.lock().notify();
                            last_kb_report = [0u8; 8];
                            last_cons_report = [0u8; 1];
                        }
                    }
                } else {
                    mode_hold_start = Some(now);
                }
            }
        } else {
            mode_hold_start = None;
            mode_hold_triggered = false;
        }

        // Pairing Toggle (Hold Key 1 + Key 3 for 5s)
        if k1_p && k3_p && !k2_p {
            if let Some(start) = pairing_hold_start {
                if now.duration_since(start) >= config::PAIRING_HOLD_DURATION {
                    if state != MachineState::Pairing {
                        println!("Manual Pairing Start (Clearing all bonds)...");
                        unsafe {
                            esp_idf_sys::ble_store_clear();
                        }
                        if let Err(e) = advertising.lock().start() {
                            println!("Failed to start pairing advertising: {:?}", e);
                        }
                        let mut s = server_arc.lock().unwrap();
                        *s = MachineState::Pairing;
                    }
                }
            } else {
                pairing_hold_start = Some(now);
            }
        } else {
            pairing_hold_start = None;
        }

        // Display Power Management
        let inactivity_duration = now.duration_since(last_activity);
        let is_mode_toast = mode_toast_until.is_some_and(|until| now < until);
        let target_display_on = state == MachineState::Pairing
            || inactivity_duration < config::DISPLAY_TIMEOUT
            || is_mode_toast;

        if target_display_on != display_is_on {
            if target_display_on {
                let _ = display.set_contrast(0x10);
            } else {
                let _ = display.set_contrast(0);
                let _ = display.clear();
                let _ = display.flush();
            }
            display_is_on = target_display_on;
            if display_is_on {
                // Force redraw when turning back on
                last_display_state = (MachineState::Idle, true, true, true, Key2Mode::Voice, true, false);
            }
        }

        // Update Display
        let is_pairing_blink = state == MachineState::Pairing
            && (now.duration_since(blink_timer).as_millis() % 1000 < 500);
        let is_reconnecting = wakeup_key_to_send.is_some();
        let current_display_state = (
            state,
            k1_p,
            k2_p,
            k3_p,
            key2_mode,
            is_pairing_blink || is_reconnecting,
            is_mode_toast,
        );

        if display_is_on && current_display_state != last_display_state {
            let _ = display.clear();

            let header_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
            let status_style = MonoTextStyle::new(&FONT_8X13, BinaryColor::On);

            // App Title
            let _ = Text::with_alignment(
                "RUSTY BALCON",
                Point::new(64, 10),
                header_style,
                Alignment::Center,
            )
            .draw(&mut display);

            // Separator Line
            let _ = Line::new(Point::new(0, 14), Point::new(127, 14))
                .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
                .draw(&mut display);

            // Connection Status
            let status_text = if is_mode_toast {
                match key2_mode {
                    Key2Mode::Voice => "MODE: VOICE",
                    Key2Mode::Esc => "MODE: ESC",
                }
            } else {
                match state {
                    MachineState::Idle => {
                        if wakeup_key_to_send.is_some() {
                            "RECONNECTING..."
                        } else {
                            "IDLE"
                        }
                    }
                    MachineState::Pairing => {
                        if is_pairing_blink {
                            ">> PAIRING <<"
                        } else {
                            "-- PAIRING --"
                        }
                    }
                    MachineState::Connected => "CONNECTED",
                }
            };
            let _ = Text::with_alignment(
                status_text,
                Point::new(64, 33),
                status_style,
                Alignment::Center,
            )
            .draw(&mut display);

            // Key Visualization Boxes
            let key_width = 24u32;
            let key_height = 14u32;
            let key_y = 42i32;
            let key_xs = [14i32, 52, 90];
            let key_labels = ["GLB", key2_mode.label(), "ENT"];
            let key_pressed = [k1_p, k2_p, k3_p];

            for i in 0..3 {
                let x = key_xs[i];
                let pressed = key_pressed[i];
                let rect = Rectangle::new(Point::new(x, key_y), Size::new(key_width, key_height));
                let style = if pressed {
                    PrimitiveStyle::with_fill(BinaryColor::On)
                } else {
                    PrimitiveStyle::with_stroke(BinaryColor::On, 1)
                };
                let _ = rect.into_styled(style).draw(&mut display);

                let text_style = if pressed {
                    MonoTextStyleBuilder::new()
                        .font(&FONT_6X10)
                        .text_color(BinaryColor::Off)
                        .build()
                } else {
                    header_style
                };
                let _ = Text::with_alignment(
                    key_labels[i],
                    Point::new(x + key_width as i32 / 2, key_y + 11),
                    text_style,
                    Alignment::Center,
                )
                .draw(&mut display);
            }

            if let Err(e) = display.flush() {
                println!("Display flush error: {:?}", e);
            }
            last_display_state = current_display_state;
        }

        match state {
            MachineState::Connected => {
                // Replay the key that woke us from deep sleep, once BLE stabilizes
                if let Some(ct) = connect_time {
                    if now.duration_since(ct) >= Duration::from_millis(500) {
                        if let Some((k1_wake, k2_wake, k3_wake)) = wakeup_key_to_send.take() {
                            println!(
                                "Replaying wakeup key: k1={} k2={} k3={}",
                                k1_wake, k2_wake, k3_wake
                            );
                            let esc_wake = k2_wake && key2_mode == Key2Mode::Esc;
                            let voice_wake = k2_wake && key2_mode == Key2Mode::Voice;

                            if k3_wake || esc_wake {
                                keyboard_report
                                    .lock()
                                    .set_value(&hid::create_keyboard_report(k3_wake, esc_wake));
                                keyboard_report.lock().notify();
                                FreeRtos::delay_ms(50);
                                keyboard_report
                                    .lock()
                                    .set_value(&hid::create_keyboard_report(false, false));
                                keyboard_report.lock().notify();
                                last_kb_report = [0u8; 8];
                            }
                            if k1_wake || voice_wake {
                                consumer_report
                                    .lock()
                                    .set_value(&hid::create_consumer_report(k1_wake, voice_wake));
                                consumer_report.lock().notify();
                                FreeRtos::delay_ms(50);
                                consumer_report
                                    .lock()
                                    .set_value(&hid::create_consumer_report(false, false));
                                consumer_report.lock().notify();
                                last_cons_report = [0u8; 1];
                            }
                        }
                    }
                }

                // Filter out keys if user is holding them for mode toggle
                let send_k1 = k1_p && !mode_hold_triggered;
                let send_k2 = k2_p && !mode_hold_triggered;
                let send_k3 = k3_p;

                let esc_pressed = send_k2 && key2_mode == Key2Mode::Esc;
                let voice_pressed = send_k2 && key2_mode == Key2Mode::Voice;

                let current_kb = hid::create_keyboard_report(send_k3, esc_pressed);
                if current_kb != last_kb_report {
                    keyboard_report.lock().set_value(&current_kb);
                    keyboard_report.lock().notify();
                    last_kb_report = current_kb;
                }

                let current_cons = hid::create_consumer_report(send_k1, voice_pressed);
                if current_cons != last_cons_report {
                    consumer_report.lock().set_value(&current_cons);
                    consumer_report.lock().notify();
                    last_cons_report = current_cons;
                }
            }
            MachineState::Idle => {}
            MachineState::Pairing => {}
        }

        FreeRtos::delay_ms(10);
    }
}
