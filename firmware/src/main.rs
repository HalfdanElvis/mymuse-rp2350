#![no_std]
#![no_main]

extern crate alloc;

mod storage;
mod stubs;
mod library_build;

use core::cell::RefCell;
use core::fmt::Write;
use core::mem::MaybeUninit;

use critical_section::Mutex;
use embedded_alloc::LlffHeap as Heap;

use rp235x_hal::clocks::init_clocks_and_plls;
use rp235x_hal::{self as hal, entry};
use rp235x_hal::{Clock, pac};

use usb_device::{class_prelude::*, prelude::*};
use usbd_serial::{SerialPort, USB_CLASS_CDC};

use alloc::boxed::Box;
use alloc::vec::Vec;

// --- Display Imports ---
use display_interface_spi::SPIInterface;
use embedded_graphics::{
    image::{Image, ImageRaw},
    mono_font::{MonoTextStyle, ascii::FONT_6X10},
    pixelcolor::{Rgb565, Rgb888},
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle, Triangle},
    text::Text,
};
use embedded_hal::spi::MODE_0;
use embedded_hal_bus::spi::ExclusiveDevice;
use fugit::RateExtU32;
use mipidsi::{Builder, models::ST7789, options::ColorInversion};
use rp235x_hal::gpio::FunctionSpi;
use rp235x_hal::spi::Spi;

use mymuse_core::app::App;
use mymuse_core::app::ArtContext;
use mymuse_core::draw::DrawCmd;
use mymuse_core::library::HANDLED_EXTENSIONS;
use mymuse_core::nav::NavState;
use mymuse_core::system::MockSystemInfo;

use crate::storage::{DummyTimeSource, SdStorage};
use embedded_sdmmc::SdCard;

use mymuse_hal::{AudioOutput, Display, InputSource, Storage, TimeSource};

#[global_allocator]
static HEAP: Heap = Heap::empty();

const HEAP_SIZE: usize = 256 * 1024;
static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] =
    [MaybeUninit::uninit(); HEAP_SIZE];

// --- Framebuffer in RAM ---
const WIDTH: usize = 320;
const HEIGHT: usize = 240;
static mut FRAMEBUF: [Rgb565; WIDTH * HEIGHT] =
    [Rgb565::BLACK; WIDTH * HEIGHT];

#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef =
    hal::block::ImageDef::secure_exe();

// --- USB Serial ---
static SERIAL: Mutex<RefCell<Option<SerialPort<'static, hal::usb::UsbBus>>>> =
    Mutex::new(RefCell::new(None));

struct SerialWriter<'a>(
    &'a mut SerialPort<'static, hal::usb::UsbBus>
);

impl core::fmt::Write for SerialWriter<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let _ = self.0.write(s.as_bytes());
        Ok(())
    }
}

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

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    critical_section::with(|cs| {
        let mut serial_opt = SERIAL.borrow(cs).borrow_mut();

        if let Some(serial) = serial_opt.as_mut() {
            let mut writer = SerialWriter(serial);
            let _ = writeln!(writer, "\n\n*** PANIC ***");
            let _ = writeln!(writer, "{}", info);
            let _ = writeln!(writer, "*************\n");
        }
    });

    loop {}
}

// ============================================================================
// Framebuffer Implementation
// ============================================================================

pub struct FrameBuffer<'a> {
    pub buffer: &'a mut [Rgb565],
    pub width: usize,
    pub height: usize,
}

impl<'a> DrawTarget for FrameBuffer<'a> {
    type Color = Rgb565;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(
        &mut self,
        pixels: I,
    ) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(coord, color) in pixels.into_iter() {
            if coord.x >= 0 && coord.y >= 0 {
                let x = coord.x as usize;
                let y = coord.y as usize;

                if x < self.width && y < self.height {
                    self.buffer[y * self.width + x] = color;
                }
            }
        }

        Ok(())
    }
}

impl<'a> OriginDimensions for FrameBuffer<'a> {
    fn size(&self) -> Size {
        Size::new(self.width as u32, self.height as u32)
    }
}

// ============================================================================
// Hardware Display Implementation
// ============================================================================

pub struct HardwareDisplay<D> {
    display: D,
    framebuffer: FrameBuffer<'static>,
}

impl<D> HardwareDisplay<D> {
    pub fn new(
        display: D,
        framebuffer: FrameBuffer<'static>,
    ) -> Self {
        Self {
            display,
            framebuffer,
        }
    }
}

impl<D> Display for HardwareDisplay<D>
where
    D: DrawTarget<Color = Rgb565>,
{
    fn measure_text(&self, text: &str, _size: u16) -> f32 {
        6.0 * text.chars().count() as f32
    }

    fn present(
        &mut self,
        cmds: &[DrawCmd],
        _art: &ArtContext<'_>,
    ) {
        // Clear the RAM buffer
        self.framebuffer.buffer.fill(Rgb565::BLACK);

        // Draw commands into RAM
        for cmd in cmds {
            match cmd {
                DrawCmd::Rect {
                    x, y, w, h, color, ..
                } => {
                    let color = rgb_from_u32(*color);

                    let _ = Rectangle::new(
                        Point::new(*x, *y),
                        Size::new(*w, *h),
                    )
                    .into_styled(PrimitiveStyle::with_fill(color))
                    .draw(&mut self.framebuffer);
                }

                DrawCmd::GradientRect {
                    x,
                    y,
                    w,
                    h,
                    color_start,
                    ..
                } => {
                    let color = rgb_from_u32(*color_start);

                    let _ = Rectangle::new(
                        Point::new(*x, *y),
                        Size::new(*w, *h),
                    )
                    .into_styled(PrimitiveStyle::with_fill(color))
                    .draw(&mut self.framebuffer);
                }

                DrawCmd::Text {
                    x, y, text, color, ..
                } => {
                    let color = rgb_from_u32(*color);
                    let style = MonoTextStyle::new(&FONT_6X10, color);

                    let _ = Text::new(
                        text,
                        Point::new(*x, *y),
                        style,
                    )
                    .draw(&mut self.framebuffer);
                }

                DrawCmd::Triangle {
                    x, y, w, h, color, ..
                } => {
                    let color = rgb_from_u32(*color);

                    let points = [
                        Point::new(*x, *y),
                        Point::new(*x, *y + *h as i32),
                        Point::new(
                            *x + *w as i32,
                            *y + (*h as i32 / 2),
                        ),
                    ];

                    let _ = Triangle::new(
                        points[0],
                        points[1],
                        points[2],
                    )
                    .into_styled(PrimitiveStyle::with_fill(color))
                    .draw(&mut self.framebuffer);
                }

                _ => {}
            }
        }

        // Send the framebuffer to the display
        let bytes = unsafe {
            core::slice::from_raw_parts(
                self.framebuffer.buffer.as_ptr() as *const u8,
                self.framebuffer.buffer.len() * 2,
            )
        };

        let raw_image =
            ImageRaw::<Rgb565>::new(bytes, WIDTH as u32);

        let image = Image::new(&raw_image, Point::zero());

        let _ = image.draw(&mut self.display);
    }
}

fn rgb_from_u32(color: u32) -> Rgb565 {
    let r = ((color >> 16) & 0xFF) as u8;
    let g = ((color >> 8) & 0xFF) as u8;
    let b = (color & 0xFF) as u8;

    Rgb565::from(Rgb888::new(r, g, b))
}

// ============================================================================
// Main Entry Point
// ============================================================================

#[entry]
fn main() -> ! {
    unsafe {
        HEAP.init(
            core::ptr::addr_of_mut!(HEAP_MEM) as usize,
            HEAP_SIZE,
        );
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

    let mut delay = cortex_m::delay::Delay::new(
        core_periph.SYST,
        clocks.system_clock.freq().to_Hz(),
    );

    let pins = hal::gpio::Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );

    // Create SD timer before USB setup
    let sd_timer = rp235x_hal::Timer::new_timer0(
        pac.TIMER0,
        &mut pac.RESETS,
        &clocks,
    );

    // --- 0. Setup USB Serial ---
    let usb_alloc = UsbBusAllocator::new(
        hal::usb::UsbBus::new(
            pac.USB,
            pac.USB_DPRAM,
            clocks.usb_clock,
            true,
            &mut pac.RESETS,
        ),
    );

    let usb_bus: &'static UsbBusAllocator<hal::usb::UsbBus> =
        Box::leak(Box::new(usb_alloc));

    let serial = SerialPort::new(usb_bus);

    critical_section::with(|cs| {
        SERIAL.borrow(cs).replace(Some(serial));
    });

    let mut usb_dev =
        UsbDeviceBuilder::new(
            usb_bus,
            UsbVidPid(0x2e8a, 0x000a),
        )
        .strings(&[
            StringDescriptors::default()
                .manufacturer("MyMuse")
                .product("RP2350 MyMuse")
                .serial_number("123456"),
        ])
        .unwrap()
        .device_class(USB_CLASS_CDC)
        .build();

    for _ in 0..200_000 {
        critical_section::with(|cs| {
            let mut serial_opt =
                SERIAL.borrow(cs).borrow_mut();

            if let Some(serial) = serial_opt.as_mut() {
                usb_dev.poll(&mut [serial]);
            }
        });
    }

    // --- 1. Setup SPI Display ---
    let sck = pins.gpio18.into_function::<FunctionSpi>();
    let mosi = pins.gpio19.into_function::<FunctionSpi>();

    let dc = pins.gpio16.into_push_pull_output();
    let cs = pins.gpio20.into_push_pull_output();
    let rst = pins.gpio17.into_push_pull_output();

    // Setup D-Pad
    let pin_up = pins.gpio0.into_pull_up_input();
    let pin_down = pins.gpio1.into_pull_up_input();
    let pin_left = pins.gpio2.into_pull_up_input();
    let pin_right = pins.gpio3.into_pull_up_input();
    let pin_mid = pins.gpio4.into_pull_up_input();

    let spi: Spi<_, _, _, 8> =
        Spi::new(pac.SPI0, (mosi, sck)).init(
            &mut pac.RESETS,
            clocks.peripheral_clock.freq(),
            32_000_000u32.Hz(),
            MODE_0,
        );

    let spi_device =
        ExclusiveDevice::new_no_delay(spi, cs).unwrap();

    let di = SPIInterface::new(spi_device, dc);

    delay.delay_ms(100);

    let display = Builder::new(ST7789, di)
        .reset_pin(rst)
        .invert_colors(ColorInversion::Normal)
        .orientation(
            mipidsi::options::Orientation::new()
                .rotate(mipidsi::options::Rotation::Deg90),
        )
        .init(&mut delay)
        .unwrap();

    // Create framebuffer from static RAM array
    let framebuf = FrameBuffer {
        buffer: unsafe {
            &mut *core::ptr::addr_of_mut!(FRAMEBUF)
        },
        width: WIDTH,
        height: HEIGHT,
    };

    let mut display =
        HardwareDisplay::new(display, framebuf);

    // --- 2. Setup SD Card (SPI1) ---
    info!("Initializing SD Card...");

    // MISO=12, CS=13, SCK=14, MOSI=15
    let sd_miso = pins.gpio12.into_function::<FunctionSpi>();
    let sd_cs = pins.gpio13.into_push_pull_output();
    let sd_sck = pins.gpio14.into_function::<FunctionSpi>();
    let sd_mosi = pins.gpio15.into_function::<FunctionSpi>();

    let sd_spi: Spi<_, _, _, 8> =
        Spi::new(
            pac.SPI1,
            (sd_mosi, sd_miso, sd_sck),
        )
        .init(
            &mut pac.RESETS,
            clocks.peripheral_clock.freq(),
            1_000_000u32.Hz(),
            MODE_0,
        );

    let sd_spi_device =
        ExclusiveDevice::new_no_delay(sd_spi, sd_cs).unwrap();

    let sd_card = SdCard::new(sd_spi_device, sd_timer);

    let mut storage =
        SdStorage::new(sd_card, DummyTimeSource);

    info!("SD Card initialized.");
    info!("MyMuse booting!");

    // --- 3. App Setup ---
    let mut audio = stubs::StubAudio;

    let mut input = stubs::StubInput::new(
        pin_up,
        pin_down,
        pin_left,
        pin_right,
        pin_mid,
    );

    let mut time = stubs::StubTime::new();
    let _decoder = stubs::StubDecoder;

    // Scan SD card for audio files
    info!("Scanning SD card for files...");

    let files_result =
        storage.list_files(&HANDLED_EXTENSIONS);

    let files = match files_result {
        Ok(f) => {
            info!(
                "SD Card scan successful. Found {} audio files.",
                f.len()
            );

            for (path, _size) in f.iter().take(20) {
                info!(" - {}", path);
            }

            f
        }

        Err(_) => {
            info!(
                "ERROR: SD Card scan failed! Check wiring or format."
            );

            let mut cmds = Vec::new();

            cmds.push(DrawCmd::rect(
                0, 0, 320, 240, 0xFF000000,
            ));

            cmds.push(DrawCmd::text(
                10,
                100,
                "SD CARD ERROR",
                0xFFFF0000,
                20,
                0,
                320,
            ));

            cmds.push(DrawCmd::text(
                10,
                130,
                "Check wiring or format",
                0xFFFFFFFF,
                10,
                0,
                320,
            ));

            let art = ArtContext {
                pane_current: None,
                pane_previous: None,
                now: None,
                pane_progress: 1.0,
            };

            display.present(&cmds, &art);

            delay.delay_ms(5000);

            Vec::new()
        }
    };

    // ------------------------------------------------------------------------
    // Build the artist / album / track hierarchy using SD-card metadata.
    // This replaces build_library(&paths), which only used filenames.
    // ------------------------------------------------------------------------

    let library =
        library_build::build_library_from_storage(
            &mut storage,
            files,
        );

    info!(
        "Library built: {} artists found.",
        library.len()
    );

    let nav = NavState::new(library);

    let mut app = App::new(
        nav,
        Box::new(MockSystemInfo::new()),
        1.16,
        0,
    );

    info!("entering main loop");

    // --- 4. Main Loop ---
    let mut tick = 0u32;
    let mut last_render_ms = 0;

    loop {
        critical_section::with(|cs| {
            let mut serial_opt =
                SERIAL.borrow(cs).borrow_mut();

            if let Some(serial) = serial_opt.as_mut() {
                usb_dev.poll(&mut [serial]);
            }
        });

        let now = time.now_ms();
        let audio_state = audio.state();

        let mut needs_redraw = false;

        while let Some(action) = input.poll() {
            if let Some(intent) =
                app.handle_input(action, now, audio_state)
            {
                audio.execute(intent);
            }

            needs_redraw = true;
        }

        if let Some(intent) = app.tick(now, audio_state) {
            audio.execute(intent);
            needs_redraw = true;
        }

        // Throttled rendering at ~20 FPS
        if now.saturating_sub(last_render_ms) >= 50 {
            needs_redraw = true;
        }

        if needs_redraw {
            let frame = {
                let measure =
                    |text: &str, size: u16| {
                        display.measure_text(text, size)
                    };

                app.render(
                    now,
                    &audio_state,
                    "00:00",
                    &measure,
                )
            };

            display.present(&frame.cmds, &frame.art);

            last_render_ms = now;
        }

        tick += 1;

        if tick % 30 == 0 {
            info!("alive! tick {}", tick);
        }

        delay.delay_ms(10);
    }
}
