#![no_std]
#![no_main]

extern crate alloc;
use alloc::boxed::Box;

use bleps::{
    ad_structure::{create_advertising_data, AdStructure},
    attribute_server::{AttributeServer, NotificationData, WorkResult},
    gatt,
    no_rng::NoRng,
    Ble, HciConnector,
};
use esp_hal::{
    clock::CpuClock,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull, RtcPinWithResistors},
    interrupt::software::SoftwareInterruptControl,
    rtc_cntl::{
        sleep::{RtcioWakeupSource, TimerWakeupSource, WakeupLevel},
        Rtc,
    },
    time::Instant,
    timer::timg::TimerGroup,
};
use esp_radio::ble::controller::BleConnector;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

// --- Constants & Configuration ---
mod config {
    use esp_hal::time::Duration;

    pub const INACTIVITY_TIMEOUT: Duration = Duration::from_secs(300); // 5 mins
    pub const PAIRING_HOLD_DURATION: Duration = Duration::from_secs(5); // 5 secs
    pub const BLINK_INTERVAL: Duration = Duration::from_millis(500); // 500 ms
    pub const DEEP_SLEEP_WAKEUP_SEC: u64 = 10; // 10 secs

    pub const KEY_A: u8 = 0x29; // ESC
    pub const KEY_B: u8 = 0x46; // Print Screen (PrtSc) - Mac recognizes this as F13
}

// --- HID Descriptors ---
mod hid {
    pub const REPORT_DESCRIPTOR: &[u8] = &[
        0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x85, 0x01, 0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, 0x15,
        0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0x95, 0x01, 0x75, 0x08, 0x81, 0x01,
        0x05, 0x07, 0x19, 0x00, 0x29, 0x65, 0x15, 0x00, 0x25, 0x65, 0x75, 0x08, 0x95, 0x06, 0x81,
        0x00, 0xc0,
    ];

    pub fn create_report(k1: bool, k2: bool) -> [u8; 8] {
        let mut report = [0u8; 8];
        if k1 {
            report[2] = crate::config::KEY_A;
        }
        if k2 {
            report[3] = crate::config::KEY_B;
        }
        report
    }
}

#[derive(PartialEq, Clone, Copy, Debug)]
enum MachineState {
    Idle,
    Pairing,
    Connected,
}

fn current_millis() -> u64 {
    Instant::now().duration_since_epoch().as_millis()
}

#[allow(clippy::large_stack_frames)]
#[esp_hal::main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // Give some memory to the alloc runtime internally used by bleps/esp-radio
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 66320);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    let mut rtc = Rtc::new(peripherals.LPWR);
    esp_println::println!("--- 2-Key Bluetooth Keyboard Booting ---");

    // GPIO Setup
    let mut gpio2 = peripherals.GPIO2;
    let mut gpio3 = peripherals.GPIO3;
    let key1 = Input::new(gpio2.reborrow(), InputConfig::default().with_pull(Pull::Up));
    let key2 = Input::new(gpio3.reborrow(), InputConfig::default().with_pull(Pull::Up));
    let mut led = Output::new(peripherals.GPIO8, Level::Low, OutputConfig::default());

    let radio_init = Box::leak(Box::new(
        esp_radio::init().expect("Failed to initialize Wi-Fi/BLE controller"),
    ));

    let connector = Box::leak(Box::new(
        BleConnector::new(radio_init, peripherals.BT, Default::default())
            .expect("Failed to create BLE connector"),
    ));
    
    // hci is essentially a 'static mut reference allowing us to initialize the Ble instance repeatedly.
    let hci = Box::leak(Box::new(HciConnector::new(connector, current_millis)));
    esp_println::println!("BLE Stack initialized successfully");

    let mut state = MachineState::Idle;
    let mut last_activity = Instant::now();
    let mut hold_start: Option<Instant> = None;
    let mut blink_timer = Instant::now();
    
    // Essential for HID! Tracks whether we must send an empty release report once we release all keys.
    let mut keys_were_pressed = false;

    loop {
        let now = Instant::now();
        let k1_pressed = key1.is_low();
        let k2_pressed = key2.is_low();
        let keys_pressed = k1_pressed || k2_pressed;

        // 1. Inactivity Tracker
        if keys_pressed {
            last_activity = now;
        }

        if now.duration_since_epoch() - last_activity.duration_since_epoch() >= config::INACTIVITY_TIMEOUT {
            esp_println::println!("Inactivity timeout, entering deep sleep...");
            led.set_low();

            // Must drop handles to release borrows on GPIOs before assigning them as Rtc Wakeup pins.
            core::mem::drop(key1);
            core::mem::drop(key2);

            let timer_wakeup = TimerWakeupSource::new(core::time::Duration::from_secs(config::DEEP_SLEEP_WAKEUP_SEC));

            let wakeup_pins: &mut [(&mut dyn RtcPinWithResistors, WakeupLevel)] = &mut [
                (&mut gpio2, WakeupLevel::Low),
                (&mut gpio3, WakeupLevel::Low),
            ];
            let rtcio_wakeup = RtcioWakeupSource::new(wakeup_pins);

            rtc.sleep_deep(&[&timer_wakeup, &rtcio_wakeup]);
        }

        // 2. State Transitions
        // Pairing: Hold BOTH keys for PAIRING_HOLD_DURATION
        if k1_pressed && k2_pressed {
            if let Some(start) = hold_start {
                if now.duration_since_epoch() - start.duration_since_epoch() >= config::PAIRING_HOLD_DURATION {
                    if state != MachineState::Pairing {
                        esp_println::println!("Entering Pairing Mode...");
                        state = MachineState::Pairing;

                        // Advertising Setup - Note the usage of explicit reborrow on `hci`
                        let mut ble = Ble::new(&mut *hci);
                        ble.init().unwrap();
                        ble.cmd_set_le_advertising_parameters().unwrap();
                        ble.cmd_set_le_advertising_data(
                            create_advertising_data(&[
                                AdStructure::Flags(0x06),
                                AdStructure::ServiceUuids16(&[bleps::att::Uuid::Uuid16(0x1812)]),
                                AdStructure::CompleteLocalName("2-Key-Kbrd"),
                            ])
                            .unwrap(),
                        )
                        .unwrap();
                        ble.cmd_set_le_advertise_enable(true).unwrap();

                        hold_start = None;
                    }
                }
            } else {
                hold_start = Some(now);
            }
        } else {
            hold_start = None;
        }

        // 3. Main BLE/Application Logic
        {
            let mut rf_protocol_mode = |_offset: usize, data: &mut [u8]| {
                let val = [0x01, 0x01, 0x00, 0x01];
                data[..val.len()].copy_from_slice(&val);
                Ok(val.len())
            };
            let mut rf_descriptor = |_offset: usize, data: &mut [u8]| {
                data[..hid::REPORT_DESCRIPTOR.len()].copy_from_slice(hid::REPORT_DESCRIPTOR);
                Ok(hid::REPORT_DESCRIPTOR.len())
            };
            let mut rf_report = |_offset: usize, _data: &mut [u8]| Ok(0usize);

            gatt!([service {
                uuid: "1812",
                characteristics: [
                    characteristic {
                        uuid: "2a4a",
                        read: rf_protocol_mode,
                    },
                    characteristic {
                        uuid: "2a4b",
                        read: rf_descriptor,
                    },
                    characteristic {
                        name: "hid_report",
                        uuid: "2a4d",
                        notify: true,
                        read: rf_report,
                    }
                ],
            },]);

            let mut ble = Ble::new(&mut *hci);
            let mut rng = NoRng;
            let mut srv = AttributeServer::new(&mut ble, &mut gatt_attributes, &mut rng);

            match state {
                MachineState::Idle => {
                    led.set_low();
                    let _ = srv.do_work();
                }
                MachineState::Pairing => {
                    if now.duration_since_epoch() - blink_timer.duration_since_epoch() >= config::BLINK_INTERVAL {
                        led.toggle();
                        blink_timer = now;
                    }

                    if let Ok(WorkResult::DidWork) = srv.do_work() {
                        esp_println::println!("Connected to host!");
                        state = MachineState::Connected;
                    }
                }
                MachineState::Connected => {
                    led.set_high();

                    let notification = if keys_pressed {
                        keys_were_pressed = true;
                        let report = hid::create_report(k1_pressed, k2_pressed);
                        Some(NotificationData::new(hid_report_handle, &report))
                    } else if keys_were_pressed {
                        // Key released edge-trigger! Send empty report so OS stops typing
                        keys_were_pressed = false;
                        let release_report = [0u8; 8];
                        Some(NotificationData::new(hid_report_handle, &release_report))
                    } else {
                        // Neither pressed, and we already sent the release report
                        None
                    };

                    if let Ok(WorkResult::GotDisconnected) = srv.do_work_with_notification(notification) {
                        esp_println::println!("Disconnected by host.");
                        keys_were_pressed = false; // Reset to avoid errant reports across reconnection
                        state = MachineState::Idle;
                    }
                }
            }
        }
    }
}
