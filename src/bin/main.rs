use anyhow::Result;
use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::gpio::*;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp32_nimble::{enums::*, BLEAdvertisementData, BLEDevice, BLEHIDDevice};
use std::sync::{Arc, Mutex};
use std::time::Instant;

mod config {
    use std::time::Duration;
    pub const INACTIVITY_TIMEOUT: Duration = Duration::from_secs(60);
    pub const PAIRING_HOLD_DURATION: Duration = Duration::from_secs(5);
    pub const BLINK_INTERVAL: Duration = Duration::from_millis(500);
    pub const KEY_A: u8 = 0x29; 
    pub const KEY_B: u8 = 0x46; 
}

mod hid {
    pub const REPORT_DESCRIPTOR: &[u8] = &[
        0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, 0x15, 0x00, 0x25,
        0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0x95, 0x01, 0x75, 0x08, 0x81, 0x01, 0x05, 0x07,
        0x19, 0x00, 0x29, 0x65, 0x15, 0x00, 0x25, 0x65, 0x75, 0x08, 0x95, 0x06, 0x81, 0x00, 0xc0,
    ];

    pub fn create_report(k1: bool, k2: bool) -> [u8; 8] {
        let mut report = [0u8; 8];
        if k1 { report[2] = crate::config::KEY_A; }
        if k2 { report[3] = crate::config::KEY_B; }
        report
    }
}

#[derive(PartialEq, Clone, Copy)]
enum MachineState { Idle, Pairing, Connected }

fn main() -> Result<()> {
    esp_idf_sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take().unwrap();
    let _sysloop = EspSystemEventLoop::take()?;
    let _nvs = EspDefaultNvsPartition::take()?;

    // GPIO Setup
    let key1 = PinDriver::input(peripherals.pins.gpio1, Pull::Up)?;
    let key2 = PinDriver::input(peripherals.pins.gpio2, Pull::Up)?;
    let mut led = PinDriver::output(peripherals.pins.gpio8)?;
    led.set_low()?;

    // BLE Setup
    let device = BLEDevice::take();
    device.security()
        .set_auth(AuthReq::Bond)
        .set_io_cap(SecurityIOCap::NoInputNoOutput)
        .resolve_rpa();

    let server = device.get_server();
    let mut hid = BLEHIDDevice::new(server);
    hid.report_map(hid::REPORT_DESCRIPTOR);
    hid.pnp(0x02, 0x05ac, 0x820a, 0x0210); // Apple Magic Keyboard mock
    hid.set_battery_level(100);

    let input_report = hid.input_report(1);

    let advertising = device.get_advertising();
    let mut ad_data = BLEAdvertisementData::new();
    ad_data.name("Rusty-Balcon-Std")
        .appearance(0x03C1) // Keyboard
        .add_service_uuid(esp32_nimble::utilities::BleUuid::from_uuid16(0x1812));
    advertising.lock().set_data(&mut ad_data)?;

    let mut state;
    let server_arc = Arc::new(Mutex::new(MachineState::Idle));
    let server_arc_clone = server_arc.clone();

    server.on_connect(move |_, _| {
        println!("Connected");
        let mut s = server_arc_clone.lock().unwrap();
        *s = MachineState::Connected;
    });

    let server_arc_clone2 = server_arc.clone();
    server.on_disconnect(move |_, _| {
        println!("Disconnected");
        let mut s = server_arc_clone2.lock().unwrap();
        *s = MachineState::Idle;
    });

    let mut last_activity = Instant::now();
    let mut hold_start: Option<Instant> = None;
    let mut blink_timer = Instant::now();
    let mut keys_were_pressed = false;

    println!("--- Rusty Balcon Std Core Ready ---");

    loop {
        let now = Instant::now();
        let k1_p = key1.is_low();
        let k2_p = key2.is_low();

        state = *server_arc.lock().unwrap();

        if k1_p || k2_p { 
            last_activity = now;
        }

        // Deep Sleep Logic
        if state == MachineState::Idle && now.duration_since(last_activity) >= config::INACTIVITY_TIMEOUT {
            println!("Sleep...");
            FreeRtos::delay_ms(100);
            unsafe {
                esp_idf_sys::esp_deep_sleep_start();
            }
        }

        // Pairing Toggle (Hold both keys for 5s)
        if k1_p && k2_p {
            if let Some(start) = hold_start {
                if now.duration_since(start) >= config::PAIRING_HOLD_DURATION {
                    if state == MachineState::Idle {
                        println!("Pairing Mode Start...");
                        advertising.lock().start()?;
                        let mut s = server_arc.lock().unwrap();
                        *s = MachineState::Pairing;
                    }
                }
            } else { hold_start = Some(now); }
        } else { hold_start = None; }

        match state {
            MachineState::Connected => {
                led.set_high()?;
                if k1_p || k2_p {
                    keys_were_pressed = true;
                    input_report.lock().set_value(&hid::create_report(k1_p, k2_p));
                    input_report.lock().notify();
                } else if keys_were_pressed {
                    keys_were_pressed = false;
                    input_report.lock().set_value(&[0u8; 8]);
                    input_report.lock().notify();
                }
            }
            MachineState::Idle => {
                led.set_low()?;
            }
            MachineState::Pairing => {
                if now.duration_since(blink_timer) >= config::BLINK_INTERVAL {
                    led.toggle()?;
                    blink_timer = now;
                }
            }
        }

        FreeRtos::delay_ms(10);
    }
}
