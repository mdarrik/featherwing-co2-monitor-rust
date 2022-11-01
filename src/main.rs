#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

mod adalogger;

use core::{
    cell::{Cell, RefCell},
    fmt::Write,
};
use cortex_m::interrupt::Mutex;
use embassy_executor::Spawner;
use embassy_rp::{
    gpio,
    i2c::{Blocking, I2c},
    peripherals::I2C1,
};

use embassy_sync::blocking_mutex::ThreadModeMutex;
use embassy_time::{Delay, Duration, Timer};
use embedded_graphics::{
    mono_font::{ascii::FONT_5X7, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{
        Circle, PrimitiveStyle, PrimitiveStyleBuilder, Rectangle, StrokeAlignment, Triangle,
    },
    text::{Alignment, Text},
};
use embedded_graphics_core::geometry::Dimensions;

use embedded_sdmmc::Timestamp;

use heapless::String;
use scd4x::scd4x::Scd4x;
use sh1107::{interface::DisplayInterface, prelude::*, Builder};
use shared_bus::{BusManager, I2cProxy};
use static_cell::StaticCell;

use {defmt_rtt as _, panic_probe as _};

static THIN_STROKE: PrimitiveStyle<BinaryColor> = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
static THICK_STROKE: PrimitiveStyle<BinaryColor> = PrimitiveStyle::with_stroke(BinaryColor::On, 3);
static BORDER_STROKE: PrimitiveStyle<BinaryColor> = PrimitiveStyleBuilder::new()
    .stroke_color(BinaryColor::On)
    .stroke_width(3)
    .stroke_alignment(StrokeAlignment::Inside)
    .build();

static FILL: PrimitiveStyle<BinaryColor> = PrimitiveStyle::with_fill(BinaryColor::On);

static YOFFSET: i32 = 14;

static LAST_MEASUREMENT: ThreadModeMutex<Cell<MeasurementData>> =
    ThreadModeMutex::new(Cell::new(MeasurementData::default()));
static DISPLAY: ThreadModeMutex<
    Cell<Option<GraphicsMode<I2cInterface<I2cProxy<Mutex<RefCell<I2c<I2C1, Blocking>>>>>>>>,
> = ThreadModeMutex::new(Cell::new(None));

static I2C_BUS: StaticCell<BusManager<Mutex<RefCell<I2c<I2C1, Blocking>>>>> = StaticCell::new();

#[embassy_executor::task]
async fn display_measurement_data(
    mut button: gpio::Input<'static, embassy_rp::peripherals::PIN_8>,
) {
    let character_style = MonoTextStyle::new(&FONT_5X7, BinaryColor::On);
    DISPLAY.lock(|display_option| {
        if let Some(mut display) = display_option.take() {
            display.clear();
            Text::with_alignment(
                "Starting to listen for buttons",
                display.bounding_box().top_left + Point::new(10, 10),
                character_style,
                Alignment::Left,
            )
            .draw(&mut display)
            .unwrap();
            display.flush().unwrap_or_default();
            display_option.set(Some(display));
        }
    });
    Timer::after(Duration::from_secs(5)).await;
    DISPLAY.lock(|display_option| {
        if let Some(mut display) = display_option.take() {
            display.clear();
            display.flush().unwrap_or_default();
            display_option.set(Some(display));
        }
    });
    loop {
        button.wait_for_low().await;
        let measurement_cell = LAST_MEASUREMENT.borrow();

        let measurement = measurement_cell.get();
        let mut measurement_text: String<64> = String::new();
        core::write!(
            &mut measurement_text,
            "CO2: {}\nHumidity: {:.2}\nTemp: {:.2}\n{}",
            measurement.co2,
            measurement.humidity,
            measurement.temperature,
            measurement.timestamp
        )
        .unwrap_or_default();
        DISPLAY.lock(|display_option| {
            if let Some(mut display) = display_option.take() {
                display.clear();
                Text::with_alignment(
                    &measurement_text,
                    display.bounding_box().top_left + Point::new(10, 10),
                    character_style,
                    Alignment::Left,
                )
                .draw(&mut display)
                .unwrap();
                display.flush().unwrap_or_default();
                display_option.set(Some(display));
            }
        });
        Timer::after(Duration::from_secs(5)).await;
        DISPLAY.lock(|display_option| {
            if let Some(mut display) = display_option.take() {
                display.clear();
                display.flush().unwrap_or_default();
                display_option.set(Some(display));
            }
        });
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let i2c = embassy_rp::i2c::I2c::new_blocking(
        p.I2C1,
        p.PIN_3,
        p.PIN_2,
        embassy_rp::i2c::Config::default(),
    );

    let bus = I2C_BUS.init_with(|| shared_bus::BusManagerCortexM::new(i2c));
    let mut spi_config = embassy_rp::spi::Config::default();

    spi_config.frequency = 400_000;
    let spi = embassy_rp::spi::Spi::new_blocking(p.SPI0, p.PIN_18, p.PIN_19, p.PIN_20, spi_config);

    let mut display: GraphicsMode<_> = Builder::new()
        .with_size(DisplaySize::Display64x128)
        .with_rotation(DisplayRotation::Rotate90)
        .connect_i2c(bus.acquire_i2c())
        .into();
    display.init().unwrap();
    display.clear();
    display.flush().unwrap();

    let character_style = MonoTextStyle::new(&FONT_5X7, BinaryColor::On);

    let mut measurement_text: String<64> = String::new();
    let button_b = gpio::Input::new(p.PIN_8, gpio::Pull::Up);
    let mut scd40 = scd4x::scd4x::Scd4x::new(bus.acquire_i2c(), Delay);
    initialization(&mut display, &mut scd40).await;
    let mut _adalogger = adalogger::Adalogger::new(
        bus.acquire_i2c(),
        spi,
        gpio::Output::new(p.PIN_10, gpio::Level::High),
    );

    Text::with_alignment(
        "Taking first measurement",
        display.bounding_box().center() + Point::new(0, 15),
        character_style,
        Alignment::Center,
    )
    .draw(&mut display)
    .unwrap();
    display.flush().unwrap();
    Timer::after(Duration::from_secs(1)).await;
    match scd40.measurement() {
        Ok(measurement) => {
            let timestamp = _adalogger.get_timestamp();
            core::write!(
                &mut measurement_text,
                "CO2: {}\nHumidity: {:.2}\nTemp: {:.2}\n{}",
                measurement.co2,
                measurement.humidity,
                measurement.temperature,
                timestamp
            )
            .unwrap();
            if let Err(write_error) = _adalogger.write_co2_data(&measurement, bus.acquire_i2c()) {
                display.clear();
                Text::with_alignment(
                    "Error writing CO2 data to SD",
                    display.bounding_box().center() + Point::new(0, 15),
                    character_style,
                    Alignment::Center,
                )
                .draw(&mut display)
                .unwrap();
                display.flush().unwrap();
                Timer::after(Duration::from_secs(1)).await;
                let mut err_str: String<512> = String::new();
                display.clear();
                core::write!(&mut err_str, "{:#?}", write_error).unwrap();
                Text::with_alignment(
                    &err_str,
                    Point::new(10, YOFFSET),
                    character_style,
                    Alignment::Left,
                )
                .draw(&mut display)
                .unwrap();
                display.flush().unwrap();
                Timer::after(Duration::from_secs(5)).await;
            }
            LAST_MEASUREMENT.lock(|data_cell| {
                data_cell.replace(MeasurementData {
                    co2: measurement.co2,
                    humidity: measurement.humidity,
                    temperature: measurement.temperature,
                    timestamp,
                })
            });
            display.clear();
            Text::with_alignment(
                &measurement_text,
                display.bounding_box().top_left + Point::new(10, 10),
                character_style,
                Alignment::Left,
            )
            .draw(&mut display)
            .unwrap();
        }
        Err(_) => {
            Text::with_alignment(
                "Error reading data",
                display.bounding_box().center() + Point::new(-3, -3),
                character_style,
                Alignment::Left,
            )
            .draw(&mut display)
            .unwrap();
        }
    }
    display.flush().unwrap();
    Timer::after(Duration::from_secs(5)).await;
    display.clear();
    display.flush().unwrap();
    scd40.stop_periodic_measurement().unwrap_or_default();
    scd40
        .start_low_power_periodic_measurements()
        .unwrap_or_default();
    DISPLAY.lock(|d| {
        d.replace(Some(display));
    });
    _spawner
        .spawn(display_measurement_data(button_b))
        .unwrap_or_default();
    let mut indicator_led = gpio::Output::new(p.PIN_13, gpio::Level::Low);
    loop {
        let measurement_wait_future = async {
            while !scd40.data_ready_status().unwrap_or_default() {
                Timer::after(Duration::from_secs_floor(30)).await;
            }
        };
        measurement_wait_future.await;
        match scd40.measurement() {
            Ok(measurement) => {
                let timestamp = _adalogger.get_timestamp();
                // set the indicator led high to signal that an SD card write is happening to reduce the chances of corrupting the data
                indicator_led.set_high();
                if let Err(write_error) = _adalogger.write_co2_data(&measurement, bus.acquire_i2c())
                {
                    DISPLAY.lock(|display_option| {
                        if let Some(mut display) = display_option.take() {
                            display.clear();
                            let mut err_str: String<512> = String::new();
                            display.clear();
                            core::write!(&mut err_str, "{:#?}", write_error).unwrap();
                            Text::with_alignment(
                                &err_str,
                                Point::new(10, YOFFSET),
                                character_style,
                                Alignment::Left,
                            )
                            .draw(&mut display)
                            .unwrap();
                            display.flush().unwrap_or_default();
                            display_option.set(Some(display));
                        }
                    });

                    Timer::after(Duration::from_secs(5)).await;
                    DISPLAY.lock(|display_option| {
                        if let Some(mut display) = display_option.take() {
                            display.clear();
                            display.flush().unwrap_or_default();
                            display_option.set(Some(display));
                        }
                    });
                }
                LAST_MEASUREMENT.lock(|data_cell| {
                    data_cell.replace(MeasurementData {
                        co2: measurement.co2,
                        humidity: measurement.humidity,
                        temperature: measurement.temperature,
                        timestamp,
                    })
                });
            }
            Err(_) => {}
        }
        indicator_led.set_low();
    }
    // let mut should_clear = true;
    // let mut button_pushed = false;
    // loop {
    //     if button_pushed {
    //         should_clear = false;
    //         button_pushed = false;
    //         match scd40.measurement() {
    //             Ok(measurement) => {
    //                 measurement_text.clear();
    //                 let timestamp = _adalogger.get_timestamp();
    //                 core::write!(
    //                     &mut measurement_text,
    //                     "CO2: {}\nHumidity: {:.2}\nTemp: {:.2}\n{}",
    //                     measurement.co2,
    //                     measurement.humidity,
    //                     measurement.temperature,
    //                     timestamp
    //                 )
    //                 .unwrap();
    //                 _adalogger
    //                     .write_co2_data(&measurement, bus.acquire_i2c())
    //                     .unwrap();
    //                 Text::with_alignment(
    //                     &measurement_text,
    //                     display.bounding_box().top_left + Point::new(10, 10),
    //                     character_style,
    //                     Alignment::Left,
    //                 )
    //                 .draw(&mut display)
    //                 .unwrap();
    //                 LAST_MEASUREMENT.lock(|d| {
    //                     d.replace(MeasurementData {measurement, timestamp});
    //                 })
    //             }
    //             Err(err_data) => {
    //                 let mut display_text: heapless::String<64> = heapless::String::new();
    //                 core::write!(&mut display_text, "{:?}", err_data).unwrap();
    //                 Text::with_alignment(
    //                     &display_text,
    //                     display.bounding_box().center() + Point::new(-3, -3),
    //                     character_style,
    //                     Alignment::Left,
    //                 )
    //                 .draw(&mut display)
    //                 .unwrap();
    //             }
    //         }
    //         display.flush().unwrap();
    //         Timer::after(Duration::from_secs(5)).await;
    //     } else if should_clear {
    //         display.clear();
    //         display.flush().unwrap();
    //         button_b.wait_for_low().await;
    //         button_pushed = true;
    //         continue;
    //     }
    //     let display_sensor_data_future = async {
    //         button_b.wait_for_low().await;
    //         button_pushed = true;
    //     };
    //     let time_out_future = async {
    //         Timer::after(Duration::from_secs(5)).await;
    //         should_clear = true;
    //     };
    //     pin_mut!(display_sensor_data_future);
    //     pin_mut!(time_out_future);

    //     futures::future::select(display_sensor_data_future, time_out_future).await;
    //     display.clear();
    //     display.flush().unwrap();
    // }
}

async fn initialization<Interface, I2C, E>(
    display: &mut GraphicsMode<Interface>,
    scd40: &mut Scd4x<I2C, Delay>,
) where
    <Interface as DisplayInterface>::Error: core::fmt::Debug,
    Interface: DisplayInterface,
    I2C: embedded_hal::blocking::i2c::Read<Error = E>
        + embedded_hal::blocking::i2c::Write<Error = E>
        + embedded_hal::blocking::i2c::WriteRead<Error = E>,
{
    let character_style = MonoTextStyle::new(&FONT_5X7, BinaryColor::On);
    // Draw a 3px wide outline around the display.
    display
        .bounding_box()
        .into_styled(BORDER_STROKE)
        .draw(display)
        .unwrap();
    // Draw a triangle.
    Triangle::new(
        Point::new(16, 16 + YOFFSET),
        Point::new(16 + 16, 16 + YOFFSET),
        Point::new(16 + 8, YOFFSET),
    )
    .into_styled(THIN_STROKE)
    .draw(display)
    .unwrap();

    // Draw a filled square
    Rectangle::new(Point::new(52, YOFFSET), Size::new(16, 16))
        .into_styled(FILL)
        .draw(display)
        .unwrap();

    // Draw a circle with a 3px wide stroke.
    Circle::new(Point::new(88, YOFFSET), 17)
        .into_styled(THICK_STROKE)
        .draw(display)
        .unwrap();

    Text::with_alignment(
        "Hi Rust",
        display.bounding_box().center() + Point::new(0, 15),
        character_style,
        Alignment::Center,
    )
    .draw(display)
    .unwrap();
    display.flush().unwrap();
    Timer::after(Duration::from_secs(1)).await;
    display.clear();
    display.flush().unwrap();
    if let Err(_) = scd40.stop_periodic_measurement() {
        Text::with_alignment(
            "Error stopping measurements",
            display.bounding_box().center(),
            character_style,
            Alignment::Center,
        )
        .draw(display)
        .unwrap();
        display.flush().unwrap();
        Timer::after(Duration::from_secs(2)).await;
    }
    #[cfg(feature = "factory-reset")]
    async {
        display.clear();
        if let Err(e) = scd40.factory_reset() {
            Text::with_alignment(
                "scd40:\nError issuing factory reset",
                display.bounding_box().center(),
                character_style,
                Alignment::Center,
            )
            .draw(display)
            .unwrap_or_default();
        } else {
            Text::with_alignment(
                "scd40:\n factory reset complete",
                display.bounding_box().center(),
                character_style,
                Alignment::Center,
            )
            .draw(display)
            .unwrap_or_default();
        }
        display.flush().unwrap_or_default();
        Timer::after(Duration::from_secs(4)).await;
        display.clear();
        display.flush().unwrap_or_default();
    }
    .await;
    if let Ok(serial_num) = scd40.serial_number() {
        let mut serial_num_str: heapless::String<64> = heapless::String::new();
        core::write!(&mut serial_num_str, "S#: {}", serial_num).unwrap();
        Text::with_alignment(
            &serial_num_str,
            display.bounding_box().center(),
            character_style,
            Alignment::Center,
        )
        .draw(display)
        .unwrap();
    } else {
        Text::with_alignment(
            "Error reading serial number",
            display.bounding_box().center(),
            character_style,
            Alignment::Center,
        )
        .draw(display)
        .unwrap();
    }

    match scd40.self_test_is_ok() {
        Ok(is_ok) => {
            if is_ok {
                Text::with_alignment(
                    "self test is ok",
                    Point {
                        x: display.bounding_box().center().x,
                        y: display.bounding_box().center().y + YOFFSET,
                    },
                    character_style,
                    Alignment::Center,
                )
                .draw(display)
                .unwrap();
            } else {
                Text::with_alignment(
                    "self test is not ok",
                    Point {
                        x: display.bounding_box().center().x,
                        y: display.bounding_box().center().y + YOFFSET,
                    },
                    character_style,
                    Alignment::Center,
                )
                .draw(display)
                .unwrap();
            }
        }
        Err(_) => {
            Text::with_alignment(
                "Error during self test",
                display.bounding_box().center(),
                character_style,
                Alignment::Center,
            )
            .draw(display)
            .unwrap();
        }
    }
    display.flush().unwrap();
    Timer::after(Duration::from_secs(5)).await;
    display.clear();
    scd40.reinit().unwrap_or_default();
    if let Ok(self_calibration_settings) = scd40.automatic_self_calibration() {
        if self_calibration_settings {
            Text::with_alignment(
                "Self calibration set",
                Point {
                    x: display.bounding_box().center().x,
                    y: YOFFSET,
                },
                character_style,
                Alignment::Center,
            )
            .draw(display)
            .unwrap();
        } else {
            Text::with_alignment(
                "self calibration is not set",
                Point {
                    x: display.bounding_box().center().x,
                    y: YOFFSET,
                },
                character_style,
                Alignment::Center,
            )
            .draw(display)
            .unwrap();
        }
    }
    #[cfg(feature = "set-altitude")]
    if let Err(_) = scd40.set_altitude(64) {
        Text::with_alignment(
            "Error setting altitude",
            Point {
                x: display.bounding_box().center().x,
                y: YOFFSET * 2,
            },
            character_style,
            Alignment::Center,
        )
        .draw(display)
        .unwrap();
    }
    if let Ok(altitude) = scd40.altitude() {
        let mut altitude_string: String<12> = String::new();
        core::write!(&mut altitude_string, "alt: {}", altitude).unwrap();
        Text::with_alignment(
            &altitude_string,
            Point {
                x: display.bounding_box().center().x,
                y: YOFFSET * 3,
            },
            character_style,
            Alignment::Center,
        )
        .draw(display)
        .unwrap();
    }
    display.flush().unwrap();

    Timer::after(Duration::from_secs(5)).await;
    display.clear();
    #[cfg(feature = "set-altitude")]
    scd40.persist_settings().unwrap_or_default();
    if let Err(_) = scd40.start_periodic_measurement() {
        Text::with_alignment(
            "Error starting measurements",
            display.bounding_box().center(),
            character_style,
            Alignment::Center,
        )
        .draw(display)
        .unwrap();
        display.flush().unwrap();
        Timer::after(Duration::from_secs(10)).await;
    }
    let mut data_ready = false;
    while !data_ready {
        match scd40.data_ready_status() {
            Ok(is_ready) => {
                data_ready = is_ready;
            }
            Err(_) => {
                Text::with_alignment(
                    "Error getting data ready status",
                    display.bounding_box().center() + Point::new(0, 15),
                    character_style,
                    Alignment::Center,
                )
                .draw(display)
                .unwrap();
                Timer::after(Duration::from_secs(10)).await;
                display.clear();
            }
        }
        Text::with_alignment(
            "Waiting for SCD data ready",
            display.bounding_box().center() + Point::new(0, 15),
            character_style,
            Alignment::Center,
        )
        .draw(display)
        .unwrap();
        display.flush().unwrap();
        Timer::after(Duration::from_millis(500)).await;
    }
    display.clear();
    display.flush().unwrap();
}

#[derive(Debug, Clone, Copy)]
struct MeasurementData {
    co2: u16,
    humidity: f32,
    temperature: f32,
    timestamp: Timestamp,
}

impl MeasurementData {
    const fn default() -> Self {
        MeasurementData {
            co2: 0,
            humidity: 0.0,
            temperature: 0.0,
            timestamp: Timestamp {
                year_since_1970: 0,
                zero_indexed_month: 0,
                zero_indexed_day: 0,
                hours: 0,
                minutes: 0,
                seconds: 0,
            },
        }
    }
}
