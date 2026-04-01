#![no_std]
#![no_main]

extern crate alloc;
use alloc::boxed::Box;

use bleps::{
    Ble, HciConnector,
    ad_structure::*,
    attribute_server::{AttributeServer, NotificationData, WorkResult},
    gatt,
};
use esp_hal::{
    clock::CpuClock,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull, RtcPinWithResistors},
    interrupt::software::SoftwareInterruptControl,
    rtc_cntl::{
        Rtc,
        sleep::{RtcioWakeupSource, TimerWakeupSource, WakeupLevel},
    },
    time::Instant,
    timer::timg::TimerGroup,
};
use esp_radio::ble::controller::BleConnector;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    esp_println::println!("!!! PANIC: {:?} !!!", info);
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

mod config {
    use esp_hal::time::Duration;
    pub const INACTIVITY_TIMEOUT: Duration = Duration::from_millis(60_000);
    pub const PAIRING_HOLD_DURATION: Duration = Duration::from_secs(5);
    pub const BLINK_INTERVAL: Duration = Duration::from_millis(500);
    pub const DEEP_SLEEP_WAKEUP_SEC: u64 = 10;
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

pub struct EspRng {
    trng: *const u32,
}
impl EspRng {
    pub fn new() -> Self { Self { trng: 0x600260B0 as *const u32 } }
}
impl rand_core::RngCore for EspRng {
    fn next_u32(&mut self) -> u32 { unsafe { core::ptr::read_volatile(self.trng) } }
    fn next_u64(&mut self) -> u64 { (self.next_u32() as u64) << 32 | (self.next_u32() as u64) }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        for chunk in dest.chunks_mut(4) {
            let val = self.next_u32().to_le_bytes();
            chunk.copy_from_slice(&val[..chunk.len()]);
        }
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> { self.fill_bytes(dest); Ok(()) }
}
impl rand_core::CryptoRng for EspRng {}

pub struct SafeConnector<'a, T: bleps::HciConnection>(&'a T);
impl<'a, T: bleps::HciConnection> bleps::HciConnection for SafeConnector<'a, T> {
    fn read(&self) -> Option<u8> {
        let mut b = self.0.read();
        let mut retry = 0;
        while b.is_none() && retry < 5000 {
            b = self.0.read();
            retry += 1;
        }
        b
    }
    fn write(&self, data: u8) { self.0.write(data); }
}

fn current_millis() -> u64 { Instant::now().duration_since_epoch().as_millis() }
static mut G_ACTIVITY: bool = false;

#[derive(PartialEq, Clone, Copy)]
enum MachineState { Idle, Pairing, Connected }

#[esp_hal::main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 66320); // Recommended size for BLE HID

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    let mut rtc = Rtc::new(peripherals.LPWR);
    let mut gpio1 = peripherals.GPIO1;
    let mut gpio2 = peripherals.GPIO2;
    let key1 = Input::new(gpio1.reborrow(), InputConfig::default().with_pull(Pull::Up));
    let key2 = Input::new(gpio2.reborrow(), InputConfig::default().with_pull(Pull::Up));
    let mut led = Output::new(peripherals.GPIO8, Level::Low, OutputConfig::default());

    let radio_init = Box::leak(Box::new(esp_radio::init().expect("Radio init failed")));
    let connector = Box::leak(Box::new(BleConnector::new(radio_init, peripherals.BT, Default::default()).expect("Connector failed")));
    let safe_connector = Box::leak(Box::new(SafeConnector(connector)));
    let hci = Box::leak(Box::new(HciConnector::new(safe_connector, current_millis)));

    // GATT Attributes Definition
    let mut rf_name = |offset: usize, data: &mut [u8]| {
        let name = b"Balcon-Final";
        if offset >= name.len() { return 0; }
        let len = (name.len() - offset).min(data.len());
        data[..len].copy_from_slice(&name[offset..offset+len]);
        len
    };
    let mut rf_app = |offset: usize, data: &mut [u8]| {
        let val = [0xC1, 0x03];
        if offset >= val.len() { return 0; }
        let len = (val.len() - offset).min(data.len());
        data[..len].copy_from_slice(&val[offset..offset+len]);
        len
    };
    let mut rf_info = |offset: usize, data: &mut [u8]| {
        let val = [0x11, 0x01, 0x00, 0x02];
        if offset >= val.len() { return 0; }
        let len = (val.len() - offset).min(data.len());
        data[..len].copy_from_slice(&val[offset..offset+len]);
        len
    };
    let mut rf_desc = |offset: usize, data: &mut [u8]| {
        unsafe { G_ACTIVITY = true; }
        let d = hid::REPORT_DESCRIPTOR;
        if offset >= d.len() { return 0; }
        let len = (d.len() - offset).min(data.len());
        data[..len].copy_from_slice(&d[offset..offset+len]);
        len
    };
    let mut rf_rep = |_o: usize, _d: &mut [u8]| 0usize;
    let mut wf_ctrl = |_o: usize, _d: &[u8]| {};
    let mut rf_ref = |offset: usize, data: &mut [u8]| {
        let val = [0x00, 0x01];
        if offset >= val.len() { return 0; }
        let len = (val.len() - offset).min(data.len());
        data[..len].copy_from_slice(&val[offset..offset+len]);
        len
    };

    gatt!([
        service { uuid: "1800", characteristics: [
            characteristic { uuid: "2a00", read: rf_name },
            characteristic { uuid: "2a01", read: rf_app },
        ]},
        service { uuid: "1812", characteristics: [
            characteristic { uuid: "2a4a", read: rf_info },
            characteristic { uuid: "2a4b", read: rf_desc },
            characteristic { name: "hid_report", uuid: "2a4d", notify: true, read: rf_rep, descriptors: [
                descriptor { uuid: "2908", read: rf_ref },
            ]},
            characteristic { uuid: "2a4c", write: wf_ctrl },
        ]},
    ]);

    let ble = Box::leak(Box::new(Ble::new(&mut *hci)));
    let ble_raw = ble as *mut Ble;

    ble.init().unwrap();
    ble.cmd_set_le_advertising_parameters().unwrap();
    ble.cmd_set_le_advertising_data(create_advertising_data(&[
        AdStructure::Flags(0x06),
        AdStructure::Unknown { ty: 0x19, data: &[0xC1, 0x03] },
        AdStructure::ServiceUuids16(&[bleps::att::Uuid::Uuid16(0x1812)]),
        AdStructure::CompleteLocalName("Balcon-Final"),
    ]).unwrap()).unwrap();

    let mut rng = EspRng::new();
    let mut srv = AttributeServer::new(ble, &mut gatt_attributes, &mut rng);

    let mut state = MachineState::Idle;
    let mut last_activity = Instant::now();
    let mut hold_start: Option<Instant> = None;
    let mut blink_timer = Instant::now();
    let mut keys_were_pressed = false;

    esp_println::println!("--- Rusty Balcon Stable ---");

    loop {
        let now = Instant::now();
        let k1_p = key1.is_low();
        let k2_p = key2.is_low();

        if k1_p || k2_p { last_activity = now; }

        if state == MachineState::Idle && now.duration_since_epoch() - last_activity.duration_since_epoch() >= config::INACTIVITY_TIMEOUT {
            esp_println::println!("Sleep...");
            core::mem::drop(key1); core::mem::drop(key2);
            let tws = TimerWakeupSource::new(core::time::Duration::from_secs(config::DEEP_SLEEP_WAKEUP_SEC));
            let mut wp: [(&mut dyn RtcPinWithResistors, WakeupLevel); 2] = [(&mut gpio1, WakeupLevel::Low), (&mut gpio2, WakeupLevel::Low)];
            let rows = RtcioWakeupSource::new(&mut wp);
            rtc.sleep_deep(&[&tws, &rows]);
        }

        if k1_p && k2_p {
            if let Some(start) = hold_start {
                if now.duration_since_epoch() - start.duration_since_epoch() >= config::PAIRING_HOLD_DURATION {
                    if state == MachineState::Idle {
                        esp_println::println!("Pairing Mode Wait...");
                        state = MachineState::Pairing;
                        unsafe {
                            G_ACTIVITY = false;
                            // START ADVERTISING ONLY WHEN srv IS FULLY READY
                            (*ble_raw).cmd_set_le_advertise_enable(true).unwrap();
                        }
                    }
                }
            } else { hold_start = Some(now); }
        } else { hold_start = None; }

        match state {
            MachineState::Pairing => {
                if now.duration_since_epoch() - blink_timer.duration_since_epoch() >= config::BLINK_INTERVAL {
                    led.toggle(); blink_timer = now;
                }
                let _ = srv.do_work();
                if unsafe { G_ACTIVITY } {
                    esp_println::println!("Bonding/Connected...");
                    state = MachineState::Connected; led.set_high();
                }
            }
            MachineState::Connected => {
                let note = if k1_p || k2_p {
                    keys_were_pressed = true; Some(NotificationData::new(hid_report_handle, &hid::create_report(k1_p, k2_p)))
                } else if keys_were_pressed {
                    keys_were_pressed = false; Some(NotificationData::new(hid_report_handle, &[0u8; 8]))
                } else { None };

                if let Ok(WorkResult::GotDisconnected) = srv.do_work_with_notification(note) {
                    esp_println::println!("Disconnected.");
                    state = MachineState::Idle;
                    led.set_low();
                    unsafe {
                        (*ble_raw).cmd_set_le_advertise_enable(false).unwrap();
                        G_ACTIVITY = false;
                    }
                }
            }
            MachineState::Idle => led.set_low(),
        }
    }
}
