// SPDX-FileCopyrightText: 2018-2022 Joonas Javanainen <joonas.javanainen@gmail.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

mod cmds;
mod types;
mod utils;

pub use crate::cmds::*;
pub use crate::types::*;

use hidapi::{DeviceInfo, HidApi, HidDevice, HidError, HidResult};
use std::cmp::min;
use std::error::Error;
use std::ffi::{CString, NulError};
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum Mcp2210Error {
    Hid(HidError),
    NonUtf8Path(PathBuf),
    NulCharInPath(NulError),
    CommandCode { expected: u8, actual: u8 },
    SubCommandCode { expected: u8, actual: u8 },
    InvalidResponse(String),
    UnknownErrorCode(u8),
    StringSize(usize),
    PayloadSize(usize),
    TransferStatus(SpiTransferStatus),

    // MCP2210 error codes
    EepromWrite,            // 0xFA
    AccessDenied,           // 0xFB
    AccessRejected,         // 0xFC
    AccessDeniedRetry,      // 0xFD
    Unavailable,            // 0xF7
    Busy,                   // 0xF8
    UnknownCommandCode(u8), // 0xF9
}

impl fmt::Display for Mcp2210Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use crate::Mcp2210Error::*;
        match self {
            Hid(err) => fmt::Display::fmt(err, f),
            NonUtf8Path(path) => write!(f, "Non UTF-8 path: {:?}", path),
            NulCharInPath(err) => write!(f, "Nul character found in path: {}", err),
            CommandCode { expected, actual } => write!(
                f,
                "Invalid command code (expected {:2x}, got {:2x})",
                expected, actual
            ),
            SubCommandCode { expected, actual } => write!(
                f,
                "Invalid sub-command code (expected {:2x}, got {:2x})",
                expected, actual
            ),
            InvalidResponse(response) => write!(f, "Invalid response ({})", response),
            UnknownErrorCode(code) => write!(f, "Unknown error code {:2x}", code),
            StringSize(size) => write!(
                f,
                "String is too long (expected at most 29 UTF-16 encoded u16 values, got {})",
                size
            ),
            PayloadSize(size) => write!(
                f,
                "Payload is too big (expected at most 60 bytes, got {})",
                size
            ),
            TransferStatus(status) => write!(f, "Unexpected SPI transfer status {:?}", status),
            EepromWrite => write!(f, "EEPROM write failure"),
            AccessDenied => write!(f, "Access denied"),
            AccessRejected => write!(f, "Access rejected"),
            AccessDeniedRetry => write!(f, "Access denied, retrying allowed"),
            Unavailable => write!(f, "SPI bus unavailable"),
            Busy => write!(f, "SPI bus busy"),
            UnknownCommandCode(code) => write!(f, "Unknown command code {:2x}", code),
        }
    }
}

impl Error for Mcp2210Error {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        use crate::Mcp2210Error::*;
        match self {
            Hid(err) => Some(err),
            _ => None,
        }
    }
}

pub type Buffer = [u8; 64];

pub const MAX_BIT_RATE: u32 = 12_000_000;

pub struct Mcp2210 {
    device: HidDevice,
}

impl CommandResponse for Mcp2210 {
    fn command_response(&mut self, cmd: &Buffer, res: &mut Buffer) -> HidResult<()> {
        // TODO: What do write() and read() return when they succeed? Is it important?
        self.device
            .write(&[[0x00].to_vec(), cmd.to_vec()].concat())?;
        self.device.read(res)?;
        Ok(())
    }
}

impl Mcp2210 {
    /// Opens a MCP2210 by &&PathBuf or &&DeviceInfo
    ///
    /// # Deprecated
    ///
    /// Use `open_path()` or `open_device()` instead.
    ///
    /// # Panics
    ///
    /// Under the hood this calls the `hidapi::HidApi::new()` function which panics if hidapi is already
    /// initialized in "without enumerate" mode (i.e. if `HidApi::new_without_enumerate()` has been called before).
    /// This would also cause a later call to `HidApi::new_without_enumberate()` to panic.
    #[deprecated]
    pub fn open<D: Mcp2210Identity>(device_id: D) -> Result<Mcp2210, Mcp2210Error> {
        device_id.open()
    }

    /// Opens a MCP2210 by path
    ///
    /// # Panics
    ///
    /// Under the hood this calls the `hidapi::HidApi::new()` function which panics if hidapi is already
    /// initialized in "without enumerate" mode (i.e. if `HidApi::new_without_enumerate()` has been called before).
    /// This would also cause a later call to `HidApi::new_without_enumberate()` to panic.
    pub fn open_path<P: AsRef<Path>>(path: P) -> Result<Mcp2210, Mcp2210Error> {
        // Path to CString
        let path_cstr = CString::new(
            path.as_ref()
                .to_str()
                .ok_or_else(|| Mcp2210Error::NonUtf8Path(path.as_ref().to_owned()))?,
        )
        .map_err(Mcp2210Error::NulCharInPath)?;

        let context = HidApi::new().map_err(Mcp2210Error::Hid)?;
        let device = context.open_path(&path_cstr).map_err(Mcp2210Error::Hid)?;
        Ok(Mcp2210 { device })
    }

    /// Opens a MCP2210 using a Vendor ID (VID), Product ID (PID) and a serial number.
    ///
    /// # Panics
    ///
    /// Under the hood this calls the `hidapi::HidApi::new()` function which panics if hidapi is already
    /// initialized in "without enumerate" mode (i.e. if `HidApi::new_without_enumerate()` has been called before).
    /// This would also cause a later call to `HidApi::new_without_enumberate()` to panic.
    pub fn open_serial(vid: u16, pid: u16, sn: &str) -> Result<Mcp2210, Mcp2210Error> {
        let context = HidApi::new().map_err(Mcp2210Error::Hid)?;
        let device = context
            .open_serial(vid, pid, sn)
            .map_err(Mcp2210Error::Hid)?;
        Ok(Mcp2210 { device })
    }

    /// Opens a MCP2210 using a &DeviceInfo whice you may optain with the `scan_devices_with_filter()` or
    /// `scan_devices()` functions.
    ///
    /// # Panics
    ///
    /// Under the hood this calls the `hidapi::HidApi::new()` function which panics if hidapi is already
    /// initialized in "without enumerate" mode (i.e. if `HidApi::new_without_enumerate()` has been called before).
    /// This would also cause a later call to `HidApi::new_without_enumberate()` to panic.
    pub fn open_device(device_info: &DeviceInfo) -> Result<Mcp2210, Mcp2210Error> {
        let context = HidApi::new().map_err(Mcp2210Error::Hid)?;
        let device = device_info
            .open_device(&context)
            .map_err(Mcp2210Error::Hid)?;
        Ok(Mcp2210 { device })
    }

    pub fn spi_transfer_to_end(
        &mut self,
        mut data: &[u8],
        buf: &mut Vec<u8>,
    ) -> Result<(), Mcp2210Error> {
        let mut res: Buffer = [0; 64];
        {
            let len = min(data.len(), 60);
            let res = self.spi_transfer(&data[..len], &mut res)?;
            data = &data[len..];
            if res.status != SpiTransferStatus::Started {
                return Err(Mcp2210Error::TransferStatus(res.status));
            }
        }
        loop {
            let len = min(data.len(), 60);
            match self.spi_transfer(&data[..len], &mut res) {
                Ok(res) => {
                    data = &data[len..];
                    buf.extend(res.data);
                    if res.status == SpiTransferStatus::Finished {
                        break;
                    }
                }
                Err(Mcp2210Error::Busy) => (),
                Err(err) => return Err(err),
            }
        }
        Ok(())
    }
}

pub trait Mcp2210Identity {
    fn open(self) -> Result<Mcp2210, Mcp2210Error>;
}

impl Mcp2210Identity for &&PathBuf {
    fn open(self) -> Result<Mcp2210, Mcp2210Error> {
        Mcp2210::open_path(self)
    }
}

impl Mcp2210Identity for &&DeviceInfo {
    fn open(self) -> Result<Mcp2210, Mcp2210Error> {
        Mcp2210::open_device(self)
    }
}

/// Scans devices for the default vendor ID and product ID that the MCP2210 comes with
///
/// # Panics
///
/// Under the hood this calls the `hidapi::HidApi::new()` function which panics if hidapi is already
/// initialized in "without enumerate" mode (i.e. if `HidApi::new_without_enumerate()` has been called before).
/// This would also cause a later call to `HidApi::new_without_enumberate()` to panic.
pub fn scan_devices() -> Result<Vec<DeviceInfo>, Mcp2210Error> {
    scan_devices_with_filter(|d| d.vendor_id() == 0x04d8 && d.product_id() == 0x00de)
}

/// Scans devices with a provided filter
///
/// # Panics
///
/// Under the hood this calls the `hidapi::HidApi::new()` function which panics if hidapi is already
/// initialized in "without enumerate" mode (i.e. if `HidApi::new_without_enumerate()` has been called before).
/// This would also cause a later call to `HidApi::new_without_enumberate()` to panic.
pub fn scan_devices_with_filter<F: FnMut(&DeviceInfo) -> bool>(
    mut f: F,
) -> Result<Vec<DeviceInfo>, Mcp2210Error> {
    let mut results = Vec::new();
    let context = HidApi::new().map_err(Mcp2210Error::Hid)?;
    let devices = context.device_list();
    for d in devices {
        if f(d) {
            results.push(d.to_owned());
        }
    }
    Ok(results)
}
