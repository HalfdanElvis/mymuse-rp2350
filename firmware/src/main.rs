#![no_std]
#![no_main]

extern crate alloc;

mod storage;
mod stubs;

use core::mem::MaybeUninit;

use defmt::info;
use defmt_rtt as _;
use embedded_alloc::LlffHeap as Heap;
use panic_probe as _;

use rp235x_hal::clocks::init_clocks_and_plls;
use rp235x_hal::{self as hal, entry};
use rp235x_hal::{Clock, pac};

use alloc::boxed::Box;
use alloc::vec::Vec;

use mymuse_core::app::App;
use mymuse_core::library::{HANDLED_EXTENSIONS, build_library};
use mymuse_core::nav::NavState;
use mymuse_core::system::MockSystemInfo;

use mymuse_hal::ArtDecoder;
use mymuse_hal::AudioOutput;
use mymuse_hal::Display;
use mymuse_hal::InputSource;
use mymuse_hal::Storage;
use mymuse_hal::TimeSource;

// ---------------------------------------------------------------------------
// Global allocator
// ---------------------------------------------------------------------------

#[global_allocator]
static HEAP: Heap = Heap::empty();

const HEAP_SIZE: usize = 128 * 1024;
static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];

// ---------------------------------------------------------------------------
// Boot ROM image definition
// ---------------------------------------------------------------------------

#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[entry]
fn main() -> ! {
    unsafe {
        HEAP.init(core::ptr::addr_of_mut!(HEAP_MEM) as usize, HEAP_SIZE);
    }

    info!("MyMuse booting");

    // Clock + GPIO setup
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

    let mut delay =
        cortex_m::delay::Delay::new(core_periph.SYST, clocks.system_clock.freq().to_Hz());

    let _pins = hal::gpio::Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );

    // -----------------------------------------------------------------------
    // Instantiate hardware (currently stubs).
    // -----------------------------------------------------------------------

    let mut display = stubs::StubDisplay::new();
    let mut audio = stubs::StubAudio;
    let mut input = stubs::StubInput;
    let mut time = stubs::StubTime::new();
    let mut decoder = stubs::StubDecoder;
    // ↓ swap to the real one when the board arrives:
    // let mut storage = storage::SdStorage::new();
    // Use the storage to scan for files (currently returns empty vec)
    let mut storage = stubs::StubStorage;
    let files = storage
        .list_files(&HANDLED_EXTENSIONS)
        .unwrap_or_else(|_| Vec::new());
    info!("found {} audio files", files.len());

    // -----------------------------------------------------------------------
    // Boot-time library scan.
    // -----------------------------------------------------------------------

    info!("scanning library...");
    let files = storage
        .list_files(&HANDLED_EXTENSIONS)
        .unwrap_or_else(|_| Vec::new());
    info!("found {} audio files", files.len());

    let paths: Vec<_> = files.into_iter().map(|(p, _size)| p).collect();
    let library = build_library(&paths);
    info!("library: {} artists", library.len());

    let nav = NavState::new(library);
    let mut app = App::new(
        nav,
        Box::new(MockSystemInfo::new()),
        1.16, // line_height_ratio
        0,    // now_ms
    );

    info!("entering main loop");

    // -----------------------------------------------------------------------
    // Main loop.
    // -----------------------------------------------------------------------

    loop {
        let now = time.now_ms();
        let audio_state = audio.state();

        // 1. Drain input
        while let Some(action) = input.poll() {
            if let Some(intent) = app.handle_input(action, now, audio_state) {
                audio.execute(intent);
            }
        }

        // 2. Tick
        if let Some(intent) = app.tick(now, audio_state) {
            audio.execute(intent);
        }

        // 3. Lazy album-art decode
        if let Some(key) = app.current_art_key().cloned() {
            if !app.art_cache.contains(&key) {
                if let Some(bytes) = app.nav.get_album_art_bytes(&key.0, &key.1) {
                    if let Ok(art) = decoder.decode(&bytes) {
                        app.cache_art(key, art);
                    }
                }
            }
        }

        // 4. Render
        let frame = {
            let measure = |text: &str, size: u16| display.measure_text(text, size);
            // TODO: read a real clock (RTC or GPS) when hardware lands.
            app.render(now, &audio_state, "00:00", &measure)
        };
        display.present(&frame.cmds, &frame.art);

        // 5. Pace the loop at ~30 fps.
        delay.delay_ms(33);
    }
}

// ---------------------------------------------------------------------------
// picotool metadata
// ---------------------------------------------------------------------------

#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [rp235x_hal::binary_info::EntryAddr; 5] = [
    rp235x_hal::binary_info::rp_cargo_bin_name!(),
    rp235x_hal::binary_info::rp_cargo_version!(),
    rp235x_hal::binary_info::rp_program_description!(c"MyMuse RP2350"),
    rp235x_hal::binary_info::rp_cargo_homepage_url!(),
    rp235x_hal::binary_info::rp_program_build_attribute!(),
];
