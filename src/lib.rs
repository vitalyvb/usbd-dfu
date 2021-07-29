#![cfg_attr(not(test), no_std)]
#![warn(missing_docs)]
//!
//! Implements DFU protocol version 1.1a for a `usb-device` device.
//!
//! ## About
//!
//! DFU protocol aims to provide a standard how USB device's firmware
//! can be upgraded. Often, in this case firmware of the device
//! consists of two parts: a large main firmware, and a smaller
//! bootloader. When device is powered on, bootloader starts
//! and either runs main firmware, or enters "firmware update"
//! mode.
//!
//! Protocol implementation tries to follows DFU 1.1a protocol as
//! specified by AN3156 by STMicroelectronics and
//! USB Device Firmware Upgrade Specification, Revision 1.1.
//!
//! This library is a protocol implementation only, actual code
//! that programs, erases, or reads memory or flash in not a
//! of the library and is expected to be provided by library
//! user.
//!
//! ### Supported operations
//!
//! * Read (device to host) - upload command
//! * Write (host to device) - download command
//! * Erase
//! * Erase All
//!
//! ### Not supported operations
//!
//! * Read Unprotect - erase everything and remove read protection.
//!
//! ### Limitations
//!
//! * Maximum USB transfer size is limited to what `usb-device` supports
//! for control enpoint transfers, which is `128` bytes by default.
//!
//! * iString field in `DFU_GETSTATUS` is always `0`. Vendor-specific string
//! error descriptions are not supported.
//!
//! ## DFU utilities
//!
//! There are many implementations of tools to flash USB device
//! supporting DFU protocol, for example:
//!
//! * [dfu](https://crates.io/crates/dfu) and [dfu-flasher](https://crates.io/crates/dfu-flasher)
//! * [dfu-programmer](https://dfu-programmer.github.io/)
//! * [dfu-util](http://dfu-util.sourceforge.net/)
//! * others
//!
//!
//! ## Example
//!
//! The example below tries to focus on [`DFUClass`], parts related to a target
//! controller initialization and configuration (USB, interrupts, GPIO, etc.)
//! are not in the scope of the example.
//!
//! Check examples for more information.
//!
//! Also see documentation for `usb-device` crate, crates that supports
//! target microcontroller and provide a corresponding HAL.
//!
//! ```no_run
//! use usb_device::prelude::*;
//! use usbd_dfu::*;
//! #
//! # use usb_device::prelude::*;
//! # use usb_device::bus::UsbBusAllocator;
//! # use stm32f1xx_hal::usb::{Peripheral, UsbBus, UsbBusType};
//! #
//! # let usb_bus_alloc: UsbBusAllocator<UsbBus<Peripheral>> = unsafe { core::mem::MaybeUninit::<UsbBusAllocator<UsbBus<Peripheral>>>::uninit().assume_init() };
//! # let mut usb_dev = UsbDeviceBuilder::new(&usb_bus_alloc, UsbVidPid(0, 0)).build();
//!
//! // DFUClass will use MyMem to actually read, erase or program the memory.
//! // Here, a set of constant parameters must be set. These parameters
//! // either change how DFUClass behaves, or define host's expectations.
//!
//! struct MyMem {
//!     buffer: [u8; 64],
//!     flash_memory: [u8; 1024],
//! }
//!
//! impl DFUMemIO for MyMem {
//!     const MEM_INFO_STRING: &'static str = "@Flash/0x00000000/1*1Kg";
//!     const INITIAL_ADDRESS_POINTER: u32 = 0x0;
//!     const PROGRAM_TIME_MS: u32 = 8;
//!     const ERASE_TIME_MS: u32 = 50;
//!     const FULL_ERASE_TIME_MS: u32 = 50;
//!     const TRANSFER_SIZE: u16 = 64;
//!
//!     fn read(&mut self, address: u32, length: usize) -> Result<&[u8], DFUMemError> {
//!         // TODO: check address value
//!         let offset = address as usize;
//!         Ok(&self.flash_memory[offset..offset+length])
//!     }
//!
//!     fn erase(&mut self, address: u32) -> Result<(), DFUMemError> {
//!         // TODO: check address value
//!         self.flash_memory.fill(0xff);
//!         // TODO: verify that block is erased successfully
//!         Ok(())
//!     }
//!
//!     fn erase_all(&mut self) -> Result<(), DFUMemError> {
//!         // There is only one block, erase it.
//!         self.erase(0)
//!     }
//!
//!     fn store_write_buffer(&mut self, src:&[u8]) -> Result<(), ()>{
//!         self.buffer[..src.len()].copy_from_slice(src);
//!         Ok(())
//!     }
//!
//!     fn program(&mut self, address: u32, length: usize) -> Result<(), DFUMemError>{
//!         // TODO: check address value
//!         let offset = address as usize;
//!
//!         // Write buffer to a memory
//!         self.flash_memory[offset..offset+length].copy_from_slice(&self.buffer[..length]);
//!
//!         // TODO: verify that memory is programmed correctly
//!         Ok(())
//!     }
//!
//!     fn manifestation(&mut self) -> Result<(), DFUManifestationError> {
//!         // Nothing to do to activate FW
//!         Ok(())
//!     }
//! }
//!
//! let mut my_mem = MyMem {
//!     buffer: [0u8; 64],
//!     flash_memory: [0u8; 1024],
//! };
//!
//! // Create USB device for a target device:
//! // let usb_bus_alloc = UsbBus::new(peripheral);
//! // let usb_dev = UsbDeviceBuilder::new().build();
//!
//! // Create DFUClass
//! let mut dfu = DFUClass::new(&usb_bus_alloc, my_mem);
//!
//! // usb_dev.poll() must be called periodically, usually from USB interrupt handlers.
//! // When USB input/output is done, handlers in MyMem may be called.
//! usb_dev.poll(&mut [&mut dfu]);
//! ```
//!
//! ### Example bootloader implementation
//!
//! See [usbd-dfu-example](https://github.com/vitalyvb/usbd-dfu-example) for a functioning example.
//!

/// DFU protocol module
pub mod class;

#[doc(inline)]
pub use crate::class::{DFUClass, DFUManifestationError, DFUMemError, DFUMemIO};
