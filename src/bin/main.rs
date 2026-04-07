use anyhow::Result;
use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::gpio::*;
use esp_idf_hal::i2c::*;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::units::*;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp32_nimble::{BLEAdvertisementData, BLEDevice, BLEHIDDevice, NimbleProperties, enums::*};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_6X10},
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Alignment, Text},
};
use sh1106::Builder;
use sh1106::prelude::GraphicsMode;

mod config {
    use std::time::Duration;
    pub const INACTIVITY_TIMEOUT: Duration = Duration::from_secs(1800);
    pub const PAIRING_HOLD_DURATION: Duration = Duration::from_secs(5);
    pub const KEY_A: u8 = 0x29; // ESC
}

mod hid {
    pub const REPORT_DESCRIPTOR: &[u8] = &[
        // Keyboard Report (ID 1)
        0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x85, 0x01, 0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, 0x15, 0x00,
        0x25, 0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0x95, 0x01, 0x75, 0x08, 0x81, 0x01, 0x05, 0x07,
        0x19, 0x00, 0x29, 0x65, 0x15, 0x00, 0x25, 0x65, 0x75, 0x08, 0x95, 0x06, 0x81, 0x00, 0xc0,
        // Consumer Control Report (ID 2)
        0x05, 0x0c, 0x09, 0x01, 0xa1, 0x01, 0x85, 0x02, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x01,
        0x09, 0xcf, 0x81, 0x02, 0x75, 0x01, 0x95, 0x07, 0x81, 0x03, 0xc0,
    ];

    pub fn create_keyboard_report(pressed: bool) -> [u8; 8] {
        let mut report = [0u8; 8];
        if pressed {
            report[2] = crate::config::KEY_A;
        }
        report
    }

    pub fn create_consumer_report(voice_pressed: bool) -> [u8; 1] {
        if voice_pressed {
            [0x01] // Bit 0 is Voice Command (0xCF)
        } else {
            [0x00]
        }
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

    let peripherals = Peripherals::take().unwrap();
    let _sysloop = EspSystemEventLoop::take()?;
    let _nvs = EspDefaultNvsPartition::take()?;

    // GPIO Setup
    let key1 = PinDriver::input(peripherals.pins.gpio1, Pull::Up)?;
    let key2 = PinDriver::input(peripherals.pins.gpio2, Pull::Up)?;

    // I2C & Display Setup
    let sda = peripherals.pins.gpio8;
    let scl = peripherals.pins.gpio9;
    let i2c_config = I2cConfig::new().baudrate(100u32.kHz().into());
    let i2c_driver = I2cDriver::new(peripherals.i2c0, sda, scl, &i2c_config)?;

    let mut display: GraphicsMode<_> = Builder::new().connect_i2c(i2c_driver).into();

    display.init().unwrap_or_else(|e| {
        println!("Display init error: {:?}", e);
    });
    let _ = display.clear();
    let welcome_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    let _ = Text::with_alignment(
        "RUSTY BALCON",
        Point::new(64, 28),
        welcome_style,
        Alignment::Center,
    )
    .draw(&mut display);
    let _ = Text::with_alignment(
        env!("GIT_TAG"),
        Point::new(127, 62),
        welcome_style,
        Alignment::Right,
    )
    .draw(&mut display);
    let _ = display.flush();
    FreeRtos::delay_ms(1000);

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
    let mut hold_start: Option<Instant> = None;
    let mut last_display_state = (MachineState::Idle, true, true, true); // Force initial draw
    let mut last_kb_report = [0u8; 8];
    let mut last_cons_report = [0u8; 1];
    let blink_timer = Instant::now();

    loop {
        let now = Instant::now();
        let k1_p = key1.is_low();
        let k2_p = key2.is_low();

        let new_state = *server_arc.lock().unwrap();
        if new_state != state {
            println!("State changed: {:?} -> {:?}", state, new_state);
            state = new_state;
        }

        if k1_p || k2_p {
            last_activity = now;
        }

        // Deep Sleep Logic (Sleep if no activity for 60s, unless in Pairing mode)
        if state != MachineState::Pairing
            && now.duration_since(last_activity) >= config::INACTIVITY_TIMEOUT
        {
            println!("No activity for 30m. Entering deep sleep...");
            let _ = display.clear();
            let _ = display.flush();
            FreeRtos::delay_ms(100);

            unsafe {
                // Wake up from GPIO1 or GPIO2 (Low level)
                use esp_idf_sys::*;
                const WAKEUP_PIN_MASK: u64 = (1 << 1) | (1 << 2);
                esp_deep_sleep_enable_gpio_wakeup(
                    WAKEUP_PIN_MASK,
                    esp_deepsleep_gpio_wake_up_mode_t_ESP_GPIO_WAKEUP_GPIO_LOW,
                );
                esp_idf_sys::esp_deep_sleep_start();
            }
        }

        // Pairing Toggle (Hold both keys for 5s)
        if k1_p && k2_p {
            if let Some(start) = hold_start {
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
                hold_start = Some(now);
            }
        } else {
            hold_start = None;
        }

        // Update Display
        let is_pairing_blink = state == MachineState::Pairing
            && (now.duration_since(blink_timer).as_millis() % 1000 < 500);
        let current_display_state = (state, k1_p, k2_p, is_pairing_blink);

        if current_display_state != last_display_state {
            let _ = display.clear();

            let header_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
            let _ = Text::with_alignment(
                "RUSTY BALCON",
                Point::new(64, 12),
                header_style,
                Alignment::Center,
            )
            .draw(&mut display);

            let status_text = match state {
                MachineState::Idle => "IDLE",
                MachineState::Pairing => {
                    if is_pairing_blink {
                        ">> PAIRING <<"
                    } else {
                        "   PAIRING   "
                    }
                }
                MachineState::Connected => "CONNECTED",
            };
            let _ = Text::with_alignment(
                status_text,
                Point::new(64, 32),
                header_style,
                Alignment::Center,
            )
            .draw(&mut display);

            let keys_text = format!(
                "{}  {}",
                if k1_p { "[A]" } else { " _ " },
                if k2_p { "[B]" } else { " _ " }
            );
            let _ = Text::with_alignment(
                &keys_text,
                Point::new(64, 52),
                header_style,
                Alignment::Center,
            )
            .draw(&mut display);

            let _ = Text::with_alignment(
                env!("GIT_TAG"),
                Point::new(127, 62),
                header_style,
                Alignment::Right,
            )
            .draw(&mut display);

            if let Err(e) = display.flush() {
                println!("Display flush error: {:?}", e);
            }
            last_display_state = current_display_state;
        }

        match state {
            MachineState::Connected => {
                let current_kb = hid::create_keyboard_report(k1_p);
                if current_kb != last_kb_report {
                    keyboard_report.lock().set_value(&current_kb);
                    keyboard_report.lock().notify();
                    last_kb_report = current_kb;
                }

                let current_cons = hid::create_consumer_report(k2_p);
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
