#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

mod adalogger;

use core::fmt::Write;
use embassy_executor::Spawner;
use embassy_rp::gpio;
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
use futures::pin_mut;

use heapless::String;
use sh1107::{prelude::*, Builder};

use {defmt_rtt as _, panic_probe as _};

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let i2c = embassy_rp::i2c::I2c::new_blocking(
        p.I2C1,
        p.PIN_3,
        p.PIN_2,
        embassy_rp::i2c::Config::default(),
    );

    let bus = shared_bus::BusManagerCortexM::new(i2c);
    let spi = embassy_rp::spi::Spi::new_blocking(
        p.SPI0,
        p.PIN_18,
        p.PIN_19,
        p.PIN_20,
        embassy_rp::spi::Config::default(),
    );
    let mut display: GraphicsMode<_> = Builder::new()
        .with_size(DisplaySize::Display64x128)
        .with_rotation(DisplayRotation::Rotate90)
        .connect_i2c(bus.acquire_i2c())
        .into();
    display.init().unwrap();
    display.clear();
    display.flush().unwrap();

    let thin_stroke = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let thick_stroke = PrimitiveStyle::with_stroke(BinaryColor::On, 3);
    let border_stroke = PrimitiveStyleBuilder::new()
        .stroke_color(BinaryColor::On)
        .stroke_width(3)
        .stroke_alignment(StrokeAlignment::Inside)
        .build();
    let fill = PrimitiveStyle::with_fill(BinaryColor::On);
    let character_style = MonoTextStyle::new(&FONT_5X7, BinaryColor::On);
    let yoffset = 14;
    // Draw a 3px wide outline around the display.
    display
        .bounding_box()
        .into_styled(border_stroke)
        .draw(&mut display)
        .unwrap();
    // Draw a triangle.
    Triangle::new(
        Point::new(16, 16 + yoffset),
        Point::new(16 + 16, 16 + yoffset),
        Point::new(16 + 8, yoffset),
    )
    .into_styled(thin_stroke)
    .draw(&mut display)
    .unwrap();

    // Draw a filled square
    Rectangle::new(Point::new(52, yoffset), Size::new(16, 16))
        .into_styled(fill)
        .draw(&mut display)
        .unwrap();

    // Draw a circle with a 3px wide stroke.
    Circle::new(Point::new(88, yoffset), 17)
        .into_styled(thick_stroke)
        .draw(&mut display)
        .unwrap();

    Text::with_alignment(
        "Hi Rust",
        display.bounding_box().center() + Point::new(0, 15),
        character_style,
        Alignment::Center,
    )
    .draw(&mut display)
    .unwrap();
    display.flush().unwrap();
    Timer::after(Duration::from_secs(1)).await;
    display.clear();
    display.flush().unwrap();
    let mut measurement_text: String<64> = String::new();
    let mut button_b = gpio::Input::new(p.PIN_8, gpio::Pull::Up);
    let mut scd40 = scd4x::scd4x::Scd4x::new(bus.acquire_i2c(), Delay);
    let mut _adalogger = adalogger::Adalogger::new(
        bus.acquire_i2c(),
        spi,
        gpio::Output::new(p.PIN_10, gpio::Level::High),
    );
    if let Err(_) = scd40.stop_periodic_measurement() {
        Text::with_alignment(
            "Error stopping measurements",
            display.bounding_box().center(),
            character_style,
            Alignment::Center,
        )
        .draw(&mut display)
        .unwrap();
        display.flush().unwrap();
        Timer::after(Duration::from_secs(10)).await;
    }
    if let Ok(serial_num) = scd40.serial_number() {
        let mut serial_num_str: heapless::String<64> = heapless::String::new();
        core::write!(&mut serial_num_str, "S#: {}", serial_num).unwrap();
        Text::with_alignment(
            &serial_num_str,
            display.bounding_box().center(),
            character_style,
            Alignment::Center,
        )
        .draw(&mut display)
        .unwrap();
    } else {
        Text::with_alignment(
            "Error reading serial number",
            display.bounding_box().center(),
            character_style,
            Alignment::Center,
        )
        .draw(&mut display)
        .unwrap();
    }
    if let Err(_) = scd40.start_periodic_measurement() {
        Text::with_alignment(
            "Error starting measurements",
            display.bounding_box().center(),
            character_style,
            Alignment::Center,
        )
        .draw(&mut display)
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
                .draw(&mut display)
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
        .draw(&mut display)
        .unwrap();
        display.flush().unwrap();
        Timer::after(Duration::from_millis(500)).await;
    }
    display.clear();
    display.flush().unwrap();
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
            core::write!(
                &mut measurement_text,
                "CO2: {}\nHumidity: {:.2}\nTemp: {:.2}",
                measurement.co2,
                measurement.humidity,
                measurement.temperature
            )
            .unwrap();
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
    let mut should_clear = true;
    let mut button_pushed = false;
    loop {
        if button_pushed {
            should_clear = false;
            button_pushed = false;
            match scd40.measurement() {
                Ok(measurement) => {
                    measurement_text.clear();
                    core::write!(
                        &mut measurement_text,
                        "CO2: {}\nHumidity: {:.2}\nTemp: {:.2}",
                        measurement.co2,
                        measurement.humidity,
                        measurement.temperature
                    )
                    .unwrap();
                    Text::with_alignment(
                        &measurement_text,
                        display.bounding_box().top_left + Point::new(10, 10),
                        character_style,
                        Alignment::Left,
                    )
                    .draw(&mut display)
                    .unwrap();
                }
                Err(err_data) => {
                    let mut display_text: heapless::String<64> = heapless::String::new();
                    core::write!(&mut display_text, "{:?}", err_data).unwrap();
                    Text::with_alignment(
                        &display_text,
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
        } else if should_clear {
            display.clear();
            display.flush().unwrap();
            button_b.wait_for_low().await;
            button_pushed = true;
            continue;
        }
        let display_sensor_data_future = async {
            button_b.wait_for_low().await;
            button_pushed = true;
        };
        let time_out_future = async {
            Timer::after(Duration::from_secs(5)).await;
            should_clear = true;
        };
        pin_mut!(display_sensor_data_future);
        pin_mut!(time_out_future);

        futures::future::select(display_sensor_data_future, time_out_future).await;
        display.clear();
        display.flush().unwrap();
    }
}
