#![no_std]
#![no_main]

use esp_hal::gpio::{Input, InputConfig, Pull, RtcPinWithResistors};
use esp_hal::rtc_cntl::sleep::{RtcioWakeupSource, WakeupLevel, TimerWakeupSource};
use esp_hal::rtc_cntl::Rtc;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }

#[esp_hal::main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut rtc = Rtc::new(peripherals.LPWR);
    let mut gpio2 = peripherals.GPIO2;
    let mut gpio3 = peripherals.GPIO3;

    let key1 = Input::new(gpio2.reborrow(), InputConfig::default().with_pull(Pull::Up));
    let key2 = Input::new(gpio3.reborrow(), InputConfig::default().with_pull(Pull::Up));

    let timer = TimerWakeupSource::new(core::time::Duration::from_secs(10));

    core::mem::drop(key1);
    core::mem::drop(key2);

    let wakeup_pins: &mut [(&mut dyn RtcPinWithResistors, WakeupLevel)] = &mut [
        (&mut gpio2, WakeupLevel::Low),
        (&mut gpio3, WakeupLevel::Low),
    ];
    let rtcio = RtcioWakeupSource::new(wakeup_pins);

    rtc.sleep_deep(&[&timer, &rtcio]);
}
