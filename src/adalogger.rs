use core::{
    cell::RefCell,
    fmt::{Debug, Write},
};
use cortex_m::interrupt;
use embedded_hal::blocking::i2c::WriteRead;
use embedded_sdmmc::{Controller, Mode, SdMmcSpi, TimeSource, Timestamp};
use heapless::String;
use scd4x::types::SensorData;

static PCF8253_ADDRESS: u8 = 0x68;
static CO2_DATA_FILE_NAME: &str = "co2-data.csv";
pub struct Adalogger<I2C, B, C>
where
    B: embedded_hal::blocking::spi::Transfer<u8>,
    <B as embedded_hal::blocking::spi::Transfer<u8>>::Error: Debug,
    I2C: WriteRead,
    <I2C as WriteRead>::Error: Debug,
    C: embedded_hal::digital::v2::OutputPin,
{
    block_device: SdMmcSpi<B, C>,
    // controller: Controller<BlockSpi<'a, B, C>, Pcf8253<I2C>>,
    rtc: Pcf8253<I2C>,
}

impl<I2C, B, C> Adalogger<I2C, B, C>
where
    B: embedded_hal::blocking::spi::Transfer<u8>,
    <B as embedded_hal::blocking::spi::Transfer<u8>>::Error: Debug,
    I2C: WriteRead,
    <I2C as WriteRead>::Error: Debug,
    C: embedded_hal::digital::v2::OutputPin,
{
    pub fn new(i2c_1: I2C, spi: B, cs: C) -> Self {
        let pcf8253 = Pcf8253 {
            i2c: RefCell::new(i2c_1),
        };
        let spi_device = embedded_sdmmc::SdMmcSpi::new(spi, cs);
        Adalogger {
            block_device: spi_device,
            rtc: pcf8253,
        }
    }

    pub fn write_co2_data(
        &mut self,
        data: SensorData,
        i2c: I2C,
    ) -> Result<(), embedded_sdmmc::Error<embedded_sdmmc::SdMmcError>> {
        let mut enabled_device = self.block_device.acquire().unwrap();
        let rtc = Pcf8253 {
            i2c: RefCell::new(i2c),
        };
        let mut controller: Controller<_, Pcf8253<I2C>, 4, 4> =
            embedded_sdmmc::Controller::new(enabled_device, rtc);
        let mut volume = controller.get_volume(embedded_sdmmc::VolumeIdx(0))?;
        let root_directory = controller.open_root_dir(&volume)?;
        let mut file =
            match controller.find_directory_entry(&volume, &root_directory, CO2_DATA_FILE_NAME) {
                Err(_) => {
                    let mut file = controller.open_file_in_dir(
                        &mut volume,
                        &root_directory,
                        CO2_DATA_FILE_NAME,
                        Mode::ReadWriteCreate,
                    )?;
                    if let Err(err) = controller.write(
                        &mut volume,
                        &mut file,
                        b"CO2, Temperature (C), Humidity, Time\n",
                    ) {
                        controller.close_file(&volume, file)?;
                        controller.close_dir(&volume, root_directory);
                        return Err(err);
                    } else {
                        file
                    }
                }
                Ok(_) => controller.open_file_in_dir(
                    &mut volume,
                    &root_directory,
                    CO2_DATA_FILE_NAME,
                    Mode::ReadWriteAppend,
                )?,
            };

        let mut data_string: String<46> = String::new();
        let time = self.rtc.get_timestamp();
        core::write!(
            &mut data_string,
            "{},{},{},{}\n",
            data.co2,
            data.temperature,
            data.humidity,
            time
        )
        .unwrap();
        controller
            .write(&mut volume, &mut file, data_string.as_bytes())
            .unwrap_or_default();

        controller.close_file(&volume, file).unwrap_or_default();
        controller.close_dir(&volume, root_directory);
        controller.free();
        Ok(())
    }
    pub fn get_timestamp(&self) -> Timestamp {
        self.rtc.get_timestamp()
    }
}

struct Pcf8253<I2C> {
    i2c: RefCell<I2C>,
}

impl<'a, I2C> Pcf8253<I2C>
where
    I2C: WriteRead,
{
    fn bcd2bin(&self, bcd_value: u8) -> u8 {
        bcd_value - 6 * (bcd_value >> 4)
    }
}
impl<I2C> TimeSource for Pcf8253<I2C>
where
    I2C: WriteRead,
    <I2C as WriteRead>::Error: Debug,
{
    fn get_timestamp(&self) -> Timestamp {
        let mut time_data: [u8; 7] = [0; 7];
        interrupt::free(|_| {
            self.i2c
                .borrow_mut()
                .write_read(PCF8253_ADDRESS, &[3], &mut time_data)
                .unwrap();
        });

        Timestamp::from_calendar(
            Into::<u16>::into(self.bcd2bin(time_data[6])) + 2000,
            self.bcd2bin(time_data[5]),
            self.bcd2bin(time_data[3]),
            self.bcd2bin(time_data[2]),
            self.bcd2bin(time_data[1]),
            self.bcd2bin(time_data[0] & 0x7F),
        )
        .unwrap()
    }
}
