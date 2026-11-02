#![no_std]
#![no_main]

extern crate alloc;

mod stubs;
mod storage;

use core::mem::MaybeUninit;
use core::cell::RefCell;
use core::fmt::Write;

use critical_section::Mutex;
use embedded_alloc::LlffHeap as Heap;
use panic_halt as _;

use rp235x_hal::clocks::init_clocks_and_plls;
use rp235x_hal::{self as hal, entry};
use rp235x_hal::{Clock, pac};

use usb_device::{class_prelude::*, prelude::*};
use usbd_serial::{SerialPort, USB_CLASS_CDC};

use alloc::boxed::Box;
use alloc::vec::Vec;

use mymuse_core::app::App;
use mymuse_core::library::{build_library, HANDLED_EXTENSIONS};
use mymuse_core::nav::NavState;
use mymuse_core::system::MockSystemInfo;

use mymuse_hal::{AudioOutput, Display, InputSource, Storage, TimeSource};

#[global_allocator]
static HEAP: Heap = Heap::empty();

const HEAP_SIZE: usize = 128 * 1024;
static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];

#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();

// --- The safe static for the serial port ---
static SERIAL: Mutex<RefCell<Option<SerialPort<'static, hal::usb::UsbBus>>>> =
    Mutex::new(RefCell::new(None));

struct SerialWriter<'a>(&'a mut SerialPort<'static, hal::usb::UsbBus>);

impl core::fmt::Write for SerialWriter<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let _ = self.0.write(s.as_bytes());
        Ok(())
    }
}

// Logging macro that grabs the serial port safely from the mutex
macro_rules! info {
    ($($arg:tt)*) => {
        critical_section::with(|cs| {
            let mut serial_opt = SERIAL.borrow(cs).borrow_mut();
            if let Some(serial) = serial_opt.as_mut() {
                let mut writer = SerialWriter(serial);
                let _ = writeln!(writer, $($arg)*);
            }
        });
    };
}

#[entry]
fn main() -> ! {
    unsafe {
        HEAP.init(core::ptr::addr_of_mut!(HEAP_MEM) as usize, HEAP_SIZE);
    }

    let mut pac = pac::Peripherals::take().unwrap();
    let core_periph = cortex_m::Peripherals::take().unwrap();
    let mut watchdog = hal::Watchdog::new(pac.WATCHDOG);
    let sio = hal::Sio::new(pac.SIO);

    let clocks = init_clocks_and_plls(
        12_000_000u32,
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .ok()
    .unwrap();

    let mut delay = cortex_m::delay::Delay::new(core_periph.SYST, clocks.system_clock.freq().to_Hz());
    let _pins = hal::gpio::Pins::new(pac.IO_BANK0, pac.PADS_BANK0, sio.gpio_bank0, &mut pac.RESETS);

    // --- Setup USB Serial ---
    let usb_alloc = UsbBusAllocator::new(hal::usb::UsbBus::new(
        pac.USB,
        pac.USB_DPRAM,
        clocks.usb_clock,
        true,
        &mut pac.RESETS,
    ));
    
    // Force the USB bus to live for 'static
    let usb_bus: &'static UsbBusAllocator<hal::usb::UsbBus> = Box::leak(Box::new(usb_alloc));
    
    let serial = SerialPort::new(usb_bus);
    critical_section::with(|cs| {
        SERIAL.borrow(cs).replace(Some(serial));
    });

    let mut usb_dev = UsbDeviceBuilder::new(usb_bus, UsbVidPid(0x2e8a, 0x000a))
        .strings(&[StringDescriptors::default()
            .manufacturer("MyMuse")
            .product("RP2350 MyMuse")
            .serial_number("123456")])
        .unwrap()
        .device_class(USB_CLASS_CDC)
        .build();

    // Give the host a moment to recognize the device
    for _ in 0..200_000 {
        critical_section::with(|cs| {
            let mut serial_opt = SERIAL.borrow(cs).borrow_mut();
            if let Some(serial) = serial_opt.as_mut() {
                usb_dev.poll(&mut [serial]);
            }
        });
    }

    info!("MyMuse booting!");

    // --- App Setup ---
    let mut storage = stubs::StubStorage;
    let mut display = stubs::StubDisplay::new();
    let mut audio = stubs::StubAudio;
    let mut input = stubs::StubInput;
    let mut time = stubs::StubTime::new();
    let _decoder = stubs::StubDecoder;

    info!("scanning library...");
    let files = storage.list_files(&HANDLED_EXTENSIONS).unwrap_or_else(|_| Vec::new());
    info!("found {} audio files", files.len());

    let paths: Vec<_> = files.into_iter().map(|(p, _size)| p).collect();
    let library = build_library(&paths);
    info!("library: {} artists", library.len());

    let nav = NavState::new(library);
    let mut app = App::new(nav, Box::new(MockSystemInfo::new()), 1.16, 0);

    info!("entering main loop");

    // --- Main Loop ---
    let mut tick = 0u32;
    loop {
        // Poll USB
        critical_section::with(|cs| {
            let mut serial_opt = SERIAL.borrow(cs).borrow_mut();
            if let Some(serial) = serial_opt.as_mut() {
                usb_dev.poll(&mut [serial]);
            }
        });

        let now = time.now_ms();
        let audio_state = audio.state();

        while let Some(action) = input.poll() {
            if let Some(intent) = app.handle_input(action, now, audio_state) {
                audio.execute(intent);
            }
        }

        if let Some(intent) = app.tick(now, audio_state) {
            audio.execute(intent);
        }

        let frame = {
            let measure = |text: &str, size: u16| display.measure_text(text, size);
            app.render(now, &audio_state, "00:00", &measure)
        };
        display.present(&frame.cmds, &frame.art);

        // Log every ~second so you can see it's alive
        tick += 1;
        if tick % 30 == 0 {
            info!("alive! tick {}", tick);
        }

        delay.delay_ms(33);
    }
}
