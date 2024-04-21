#![allow(unused_variables)]

use std::{cell::RefCell, cmp::min};

mod helpers;
use helpers::*;

use usbd_class_tester::prelude::*;

use usb_device::bus::UsbBusAllocator;

use usbd_dfu::class::*;

const TESTMEMSIZE: usize = 64 * 1024;
pub struct TestMem {
    memory: RefCell<[u8; TESTMEMSIZE]>,
    buffer: [u8; 1024],
    overrides: TestMemOverride,
}

struct TestMemOverride {
    read: Option<
        fn(&mut TestMem, address: u32, length: usize) -> core::result::Result<&[u8], DFUMemError>,
    >,
    erase: Option<fn(&mut TestMem, address: u32) -> Result<(), DFUMemError>>,
    program: Option<
        fn(&mut TestMem, address: u32, length: usize) -> core::result::Result<(), DFUMemError>,
    >,
    manifestation: Option<fn(&mut TestMem) -> Result<(), DFUManifestationError>>,
}

impl TestMem {
    fn new(overrides: Option<TestMemOverride>) -> Self {
        let tmo = overrides.unwrap_or(TestMemOverride {
            read: None,
            erase: None,
            program: None,
            manifestation: None,
        });
        Self {
            memory: RefCell::new(Self::init_buf()),
            buffer: [0; 1024],
            overrides: tmo,
        }
    }

    // Initialize buffer as: [0,0, 1,0, 2,0, ... 255,0, 0,1, ...]
    fn init_buf() -> [u8; TESTMEMSIZE] {
        let mut buf = [0u8; TESTMEMSIZE];

        for (i, v) in buf.iter_mut().enumerate() {
            if i & 1 == 1 {
                *v = ((i >> 9) & 0xff) as u8;
            } else {
                *v = ((i >> 1) & 0xff) as u8;
            }
        }
        buf
    }

    fn erase(&mut self, block: usize) {
        let mut buf = self.memory.borrow_mut();
        buf[block..block + 1024].fill(0xff);
    }

    fn read_to_buf(&mut self, block: usize) -> usize {
        let len = min(self.buffer.len(), TESTMEMSIZE - block);
        let mem = self.memory.borrow();
        self.buffer[..len].copy_from_slice(&mem[block..block + len]);
        len
    }
    fn write_from_buf(&mut self, block: usize, srclen: usize) -> usize {
        let len = min(srclen, TESTMEMSIZE - block);
        let mut mem = self.memory.borrow_mut();

        for (i, m) in mem[block..block + len].iter_mut().enumerate() {
            // emulate flash write - set bits to 0 only
            *m &= self.buffer[i];
        }
        len
    }
    fn verify_with_buf(&self, block: usize, srclen: usize) -> bool {
        let len = min(srclen, TESTMEMSIZE - block);
        let mem = self.memory.borrow();

        for (i, m) in mem[block..block + len].iter().enumerate() {
            if *m != self.buffer[i] {
                return false;
            }
        }
        true
    }
}

const TESTMEM_BASE: u32 = 0x0200_0000;

impl DFUMemIO for TestMem {
    const INITIAL_ADDRESS_POINTER: u32 = TESTMEM_BASE;
    const MANIFESTATION_TOLERANT: bool = false;
    const PROGRAM_TIME_MS: u32 = 50;
    const ERASE_TIME_MS: u32 = 0x1ff;
    const FULL_ERASE_TIME_MS: u32 = 0x2_0304;
    const MEM_INFO_STRING: &'static str = "@Flash/0x02000000/16*1Ka,48*1Kg";
    const HAS_DOWNLOAD: bool = true;
    const HAS_UPLOAD: bool = true;
    const DETACH_TIMEOUT: u16 = 0x1122;
    const TRANSFER_SIZE: u16 = 128;

    fn read(&mut self, address: u32, length: usize) -> core::result::Result<&[u8], DFUMemError> {
        if self.overrides.read.is_some() {
            return self.overrides.read.unwrap()(self, address, length);
        }
        if address < TESTMEM_BASE {
            return Err(DFUMemError::Address);
        }

        let from = (address - TESTMEM_BASE) as usize;
        if from >= TESTMEMSIZE {
            return Ok(&[]);
        }

        let len = self.read_to_buf(from);
        Ok(&self.buffer[..min(length, len)])
    }

    fn erase(&mut self, address: u32) -> core::result::Result<(), DFUMemError> {
        if self.overrides.erase.is_some() {
            return self.overrides.erase.unwrap()(self, address);
        }

        if address < TESTMEM_BASE {
            return Err(DFUMemError::Address);
        }

        let from = address - TESTMEM_BASE;

        if from & 0x3ff != 0 {
            // erase aligned blocks only
            return Ok(());
        }
        if from >= TESTMEMSIZE as u32 {
            return Err(DFUMemError::Address);
        }

        self.erase(from as usize);
        Ok(())
    }

    fn erase_all(&mut self) -> Result<(), DFUMemError> {
        for block in (0..TESTMEMSIZE).step_by(1024) {
            self.erase(block);
        }
        Ok(())
    }

    fn store_write_buffer(&mut self, src: &[u8]) -> core::result::Result<(), ()> {
        self.buffer[..src.len()].clone_from_slice(src);
        Ok(())
    }

    fn program(&mut self, address: u32, length: usize) -> core::result::Result<(), DFUMemError> {
        if self.overrides.program.is_some() {
            return self.overrides.program.unwrap()(self, address, length);
        }

        if address < TESTMEM_BASE {
            return Err(DFUMemError::Address);
        }

        let dst = (address - TESTMEM_BASE) as usize;
        if dst >= TESTMEMSIZE {
            return Err(DFUMemError::Address);
        }

        let len = self.write_from_buf(dst, length);
        if len != length {
            return Err(DFUMemError::Prog);
        }

        if !self.verify_with_buf(dst, length) {
            return Err(DFUMemError::Verify);
        }

        Ok(())
    }

    fn manifestation(&mut self) -> Result<(), DFUManifestationError> {
        if self.overrides.manifestation.is_some() {
            return self.overrides.manifestation.unwrap()(self);
        }
        panic!("emulate device reset");
    }
}

/// Default DFU class factory
struct MkDFU {}

impl UsbDeviceCtx for MkDFU {
    type C<'c> = DFUClass<EmulatedUsbBus, TestMem>;
    const EP0_SIZE: u8 = 32;

    fn create_class<'a>(
        &mut self,
        alloc: &'a UsbBusAllocator<EmulatedUsbBus>,
    ) -> AnyResult<DFUClass<EmulatedUsbBus, TestMem>> {
        Ok(DFUClass::new(&alloc, TestMem::new(None)))
    }
}

#[test]
fn test_simple_get_status() {
    MkDFU {}
        .with_usb(|mut dfu, mut dev| {
            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_IDLE));
        })
        .expect("with_usb");
}

#[test]
fn test_get_configuration() {
    MkDFU {}
        .with_usb(|mut dfu, mut dev| {
            // get configuration descriptor
            let vec = dev
                .device_get_descriptor(&mut dfu, 2, 0, 0, 130)
                .expect("vec");
            assert_eq!(vec.len(), 27);

            let device = &vec[..9];
            let interf = &vec[9..18];
            let config = &vec[18..];

            // skip device, first byte should be 9=length
            assert_eq!(device[0], 9);

            // interface descriptor
            assert_eq!(
                interf,
                &[
                    9, 4, 0, 0, 0, 0xfe, // application specific
                    1,    // dfu
                    2,    // dfu mode
                    4
                ]
            );

            // dfu descriptor
            assert_eq!(
                config,
                &[
                    9, 0x21,
                    0b1011, // bitWillDetach, not bitManifestationTolerant, bitCanUpload, bitCanDnload
                    0x22, 0x11, // detach timeout
                    128, 0, // transfer size
                    0x1a, 1, // dfu version = 1.1a
                ]
            );

            // get string descriptor languages
            let vec = dev
                .device_get_descriptor(&mut dfu, 3, 0, 0, 128)
                .expect("vec");
            assert_eq!(vec, [4, 3u8, 9, 4]); // 0x409 = EN_US

            // get string descriptor (EN_US)
            let istr = dev.device_get_string(&mut dfu, 4, 0x409).expect("str");
            assert_eq!(istr, TestMem::MEM_INFO_STRING);

            // get string descriptor (lang_id = 0)
            let istr = dev.device_get_string(&mut dfu, 4, 0).expect("str");
            assert_eq!(istr, TestMem::MEM_INFO_STRING);

            // get string descriptor unsupported lang_id (lang_id = 1)
            dev.device_get_string(&mut dfu, 4, 1).expect_err("stall");
        })
        .expect("with_usb");
}

#[test]
fn test_set_address_pointer() {
    MkDFU {}
        .with_usb(|mut dfu, mut dev| {
            let new_addr: u32 = 0x2000_0000;

            assert_ne!(new_addr, dfu.get_address_pointer());
            assert_eq!(dfu.get_address_pointer(), TestMem::INITIAL_ADDRESS_POINTER);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_IDLE));

            /* Download block 0 (command), address pointer = new_addr */
            let b = new_addr.to_le_bytes();
            let vec = dev
                .download(&mut dfu, 0, &[0x21, b[0], b[1], b[2], b[3]])
                .expect("vec");
            assert_eq!(vec, []);
            assert_eq!(dfu.get_address_pointer(), TestMem::INITIAL_ADDRESS_POINTER); // must change after Get Status

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_DN_BUSY));
            assert_eq!(dfu.get_address_pointer(), new_addr);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_DNLOAD_IDLE));
        })
        .expect("with_usb");
}

#[test]
fn test_upload() {
    MkDFU {}
        .with_usb(|mut dfu, mut dev| {
            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_IDLE));

            /* Upload block 2 (offset 0) */
            let vec = dev.upload(&mut dfu, 2, 128).expect("vec");
            assert_eq!(vec.len(), 128);
            assert_eq!(vec[0..10], [0, 0, 1, 0, 2, 0, 3, 0, 4, 0]);
            assert_eq!(vec[120..128], [60, 0, 61, 0, 62, 0, 63, 0]);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_UPLOAD_IDLE));

            /* Upload block 7 (offset 5*128) */
            let vec = dev.upload(&mut dfu, 7, 128).expect("vec");
            assert_eq!(vec.len(), 128);
            assert_eq!(vec[0..10], [64, 1, 65, 1, 66, 1, 67, 1, 68, 1]);
            assert_eq!(vec[120..128], [124, 1, 125, 1, 126, 1, 127, 1]);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_UPLOAD_IDLE));

            /* Abort */
            let vec = dev.abort(&mut dfu).expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_IDLE));
        })
        .expect("with_usb");
}

#[test]
fn test_erase() {
    MkDFU {}
        .with_usb(|mut dfu, mut dev| {
            let blkaddr: u32 = TestMem::INITIAL_ADDRESS_POINTER + 1024;
            let et = TestMem::ERASE_TIME_MS.to_le_bytes();

            assert_ne!(blkaddr, dfu.get_address_pointer());
            assert_ne!(0, TestMem::ERASE_TIME_MS);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_IDLE));

            /* Download block 0 (command), erase = blkaddr */
            let b = blkaddr.to_le_bytes();
            let vec = dev
                .download(&mut dfu, 0, &[0x41, b[0], b[1], b[2], b[3]])
                .expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, TestMem::ERASE_TIME_MS, DFU_DN_BUSY));

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_DNLOAD_IDLE));

            /* Abort */
            let vec = dev.abort(&mut dfu).expect("vec");
            assert_eq!(vec, []);

            /* Upload block 9 (offset 7) - not erased */
            let vec = dev.upload(&mut dfu, 9, 128).expect("vec");
            assert_eq!(vec.len(), 128);
            assert_eq!(vec[0..10], [192, 1, 193, 1, 194, 1, 195, 1, 196, 1]);
            assert_eq!(vec[120..128], [252, 1, 253, 1, 254, 1, 255, 1]);

            /* Upload block 10 (offset 8) - erased */
            let vec = dev.upload(&mut dfu, 10, 128).expect("vec");
            assert_eq!(vec.len(), 128);
            assert_eq!(vec[0..10], [0xff; 10]);
            assert_eq!(vec[120..128], [0xff; 8]);

            /* Upload block 17 (offset 15) - erased */
            let vec = dev.upload(&mut dfu, 17, 128).expect("vec");
            assert_eq!(vec.len(), 128);
            assert_eq!(vec[0..10], [0xff; 10]);
            assert_eq!(vec[120..128], [0xff; 8]);

            /* Upload block 18 (offset 16) - not erased */
            let vec = dev.upload(&mut dfu, 18, 128).expect("vec");
            assert_eq!(vec.len(), 128);
            assert_eq!(vec[0..10], [0, 4, 1, 4, 2, 4, 3, 4, 4, 4]);
            assert_eq!(vec[120..128], [60, 4, 61, 4, 62, 4, 63, 4]);
        })
        .expect("with_usb");
}

#[test]
fn test_erase_all() {
    MkDFU {}
        .with_usb(|mut dfu, mut dev| {
            let et = TestMem::FULL_ERASE_TIME_MS.to_le_bytes();

            assert_ne!(0, TestMem::FULL_ERASE_TIME_MS);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_IDLE));

            /* Download block 0 (command), erase = full */
            let vec = dev.download(&mut dfu, 0, &[0x41]).expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(
                vec,
                status(STATUS_OK, TestMem::FULL_ERASE_TIME_MS, DFU_DN_BUSY)
            );

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_DNLOAD_IDLE));

            /* Abort */
            let vec = dev.abort(&mut dfu).expect("vec");
            assert_eq!(vec, []);

            let mut blk = 2; // offset 2 is zeroth block
            loop {
                /* Upload block - erased */
                let vec = dev.upload(&mut dfu, blk as u16, 128).expect("vec");

                if vec.len() == 0 {
                    break;
                }

                assert_eq!(vec, [0xff; 128]);

                blk += 1;
                assert!(blk < 0xffff);
            }
            assert_eq!(blk - 2, TESTMEMSIZE / 128);
        })
        .expect("with_usb");
}

#[test]
fn test_upload_last() {
    MkDFU {}
        .with_usb(|mut dfu, mut dev| {
            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_IDLE));

            /* Upload block 2 (offset 0) */
            let vec = dev.upload(&mut dfu, 2, 128).expect("vec");
            assert_eq!(vec.len(), 128);
            assert_eq!(vec[0..10], [0, 0, 1, 0, 2, 0, 3, 0, 4, 0]);
            assert_eq!(vec[120..128], [60, 0, 61, 0, 62, 0, 63, 0]);

            /* Upload block 513 (offset 511*128) - Last block */
            let vec = dev.upload(&mut dfu, 513, 128).expect("vec");
            assert_eq!(vec.len(), 128);
            assert_eq!(
                vec[0..10],
                [192, 127, 193, 127, 194, 127, 195, 127, 196, 127]
            );
            assert_eq!(vec[120..128], [252, 127, 253, 127, 254, 127, 255, 127]);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_UPLOAD_IDLE));

            /* Upload block 514 (offset 512*128), short read */
            let vec = dev.upload(&mut dfu, 514, 128).expect("vec");
            assert_eq!(vec.len(), 0);

            /* Get Status, dfuIdle after short frame */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_IDLE));
        })
        .expect("with_usb");
}

#[test]
fn test_upload_err_bad_address() {
    MkDFU {}
        .with_usb(|mut dfu, mut dev| {
            let invalid_addr: u32 = TestMem::INITIAL_ADDRESS_POINTER - 0x1_0000;

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_IDLE));

            /* Download block 0 (command), address pointer = invalid_addr */
            let b = invalid_addr.to_le_bytes();
            let vec = dev
                .download(&mut dfu, 0, &[0x21, b[0], b[1], b[2], b[3]])
                .expect("vec");
            assert_eq!(vec, []);
            assert_eq!(dfu.get_address_pointer(), TestMem::INITIAL_ADDRESS_POINTER); // must change after Get Status

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_DN_BUSY));
            assert_eq!(dfu.get_address_pointer(), invalid_addr);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_DNLOAD_IDLE));

            /* Abort */
            let vec = dev.abort(&mut dfu).expect("vec");
            assert_eq!(vec, []);

            /* Upload block 2 (offset 0) */
            let e = dev.upload(&mut dfu, 2, 128).expect_err("stall");
            assert_eq!(e, AnyUsbError::EP0Stalled);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_ERR_ADDRESS, 0, DFU_ERROR));
        })
        .expect("with_usb");
}

#[test]
fn test_download_to_upload_err() {
    MkDFU {}
        .with_usb(|mut dfu, mut dev| {
            let xaddr: u32 = TestMem::INITIAL_ADDRESS_POINTER;

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_IDLE));

            /* Download block 0 (command), address pointer */
            let b = xaddr.to_le_bytes();
            let vec = dev
                .download(&mut dfu, 0, &[0x21, b[0], b[1], b[2], b[3]])
                .expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_DN_BUSY));

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_DNLOAD_IDLE));

            /* Can't call Upload from dfuDnloadIdle, expect stall */

            /* Upload block 2 (offset 0) */
            let e = dev.upload(&mut dfu, 2, 128).expect_err("stall");
            assert_eq!(e, AnyUsbError::EP0Stalled);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_ERR_STALLED_PKT, 0, DFU_ERROR));
        })
        .expect("with_usb");
}

#[test]
fn test_download_program0_with_tail() {
    MkDFU {}
    .with_usb(|mut dfu, mut dev| {
        /* Get Status */
        let vec = dev.get_status(&mut dfu).expect("vec");
        assert_eq!(vec, status(STATUS_OK, 0, DFU_IDLE));

        /* Download block 2 (offset 0) */
        let vec = dev.download(&mut dfu, 2, &[0; 128]).expect("vec");
        assert_eq!(vec, []);

        /* Get State */
        let vec = dev.get_state(&mut dfu).expect("vec");
        assert_eq!(vec, [DFU_DNLOAD_SYNC]);

        /* Get Status */
        let vec = dev.get_status(&mut dfu).expect("vec");
        assert_eq!(
            vec,
            status(STATUS_OK, TestMem::PROGRAM_TIME_MS, DFU_DN_BUSY)
        );

        /* Get Status */
        let vec = dev.get_status(&mut dfu).expect("vec");
        assert_eq!(vec, status(STATUS_OK, 0, DFU_DNLOAD_IDLE));

        /* Download block 3 (offset 1), with a wLength of 64 bytes, emulate short write */
        let vec = dev.download(&mut dfu, 3, &[0; 64]).expect("vec");
        assert_eq!(vec, []);

        /* Get State */
        let vec = dev.get_state(&mut dfu).expect("vec");
        assert_eq!(vec, [DFU_DNLOAD_SYNC]);

        /* Get Status */
        let vec = dev.get_status(&mut dfu).expect("vec");
        assert_eq!(
            vec,
            status(STATUS_OK, TestMem::PROGRAM_TIME_MS, DFU_DN_BUSY)
        );

        /* Get Status */
        let vec = dev.get_status(&mut dfu).expect("vec");
        assert_eq!(vec, status(STATUS_OK, 0, DFU_DNLOAD_IDLE));

        /* Abort */
        let vec = dev.abort(&mut dfu).expect("vec");
        assert_eq!(vec, []);

        /* Upload block 2 (offset 0) - must be zeroed */
        let vec = dev.upload(&mut dfu, 2, 128).expect("vec");
        assert_eq!(vec.len(), 128);
        assert_eq!(vec, [0; 128]);

        /* Upload block 3 (offset 1) - must be 0 for the first 64 bytes and intact for the rest */
        let vec = dev.upload(&mut dfu, 3, 128).expect("vec");
        assert_eq!(vec.len(), 128);
        assert_eq!(vec[0..64], [0; 64]);
        assert_eq!(vec[64..72], [96, 0, 97, 0, 98, 0, 99, 0]);
        assert_eq!(vec[120..128], [124, 0, 125, 0, 126, 0, 127, 0]);

        /* Upload block 4 (offset 2) - intact, short read of 64 bytes */
        let vec = dev.upload(&mut dfu, 4, 64).expect("vec");
        assert_eq!(vec.len(), 64);
        assert_eq!(vec[0..10], [128, 0, 129, 0, 130, 0, 131, 0, 132, 0]);
        assert_eq!(vec[56..64], [156, 0, 157, 0, 158, 0, 159, 0]);
    })
    .expect("with_usb");
}

#[test]
fn test_download_program_err_verify_and_to_idle() {
    MkDFU {}
        .with_usb(|mut dfu, mut dev| {
            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_IDLE));

            /* Download block 2 (offset 0) */
            let vec = dev.download(&mut dfu, 2, &[0x55; 128]).expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(
                vec,
                status(STATUS_OK, TestMem::PROGRAM_TIME_MS, DFU_DN_BUSY)
            );

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_ERR_VERIFY, 0, DFU_ERROR));

            /* Clear Status */
            let vec = dev.clear_status(&mut dfu).expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_IDLE));
        })
        .expect("with_usb");
}

#[test]
fn test_erase_and_program() {
    MkDFU {}
        .with_usb(|mut dfu, mut dev| {
            let blkaddr: u32 = TestMem::INITIAL_ADDRESS_POINTER;

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_IDLE));

            /* Download block 0 (command), erase = blkaddr */
            let b = blkaddr.to_le_bytes();
            let vec = dev
                .download(&mut dfu, 0, &[0x41, b[0], b[1], b[2], b[3]])
                .expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, TestMem::ERASE_TIME_MS, DFU_DN_BUSY));

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_DNLOAD_IDLE));

            /* Download block 2 (offset 0) */
            let vec = dev.download(&mut dfu, 2, &[0x55; 128]).expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(
                vec,
                status(STATUS_OK, TestMem::PROGRAM_TIME_MS, DFU_DN_BUSY)
            );

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_DNLOAD_IDLE));

            /* Abort */
            let vec = dev.abort(&mut dfu).expect("vec");
            assert_eq!(vec, []);

            /* Upload block 2 (offset 0) - must be 0x55 */
            let vec = dev.upload(&mut dfu, 2, 128).expect("vec");
            assert_eq!(vec.len(), 128);
            assert_eq!(vec, [0x55; 128]);

            /* Upload block 3 (offset 1) - must be 0xff */
            let vec = dev.upload(&mut dfu, 3, 128).expect("vec");
            assert_eq!(vec.len(), 128);
            assert_eq!(vec[0..128], [0xff; 128]);
        })
        .expect("with_usb");
}

#[test]
#[should_panic(expected = "emulate device reset")]
fn test_manifestation() {
    MkDFU {}
        .with_usb(|mut dfu, mut dev| {
            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_IDLE));

            /* Download block 2 (offset 0) */
            let vec = dev.download(&mut dfu, 2, &[0; 128]).expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(
                vec,
                status(STATUS_OK, TestMem::PROGRAM_TIME_MS, DFU_DN_BUSY)
            );

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_DNLOAD_IDLE));

            /* Download block 3 (offset 1) len 0, trigger manifestation */
            let vec = dev.download(&mut dfu, 3, &[]).expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 1, DFU_MANIFEST));

            unreachable!("device must reset");
        })
        .expect("with_usb");
}

/// DFU class with manifestation call that returns
struct MkDFUMTret {}

impl UsbDeviceCtx for MkDFUMTret {
    type C<'c> = DFUClass<EmulatedUsbBus, TestMem>;
    const EP0_SIZE: u8 = 32;

    fn create_class<'a>(
        &mut self,
        alloc: &'a UsbBusAllocator<EmulatedUsbBus>,
    ) -> AnyResult<DFUClass<EmulatedUsbBus, TestMem>> {
        fn manifestation(tm: &mut TestMem) -> Result<(), DFUManifestationError> {
            Ok(())
        }
        let overrides = TestMemOverride {
            read: None,
            erase: None,
            program: None,
            manifestation: Some(manifestation),
        };
        Ok(DFUClass::new(&alloc, TestMem::new(Some(overrides))))
    }
}

#[test]
fn test_manifestation_no_reset() {
    MkDFUMTret {}
        .with_usb(|mut dfu, mut dev| {
            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_IDLE));

            /* Download block 2 (offset 0) */
            let vec = dev.download(&mut dfu, 2, &[0x0; 128]).expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(
                vec,
                status(STATUS_OK, TestMem::PROGRAM_TIME_MS, DFU_DN_BUSY)
            );

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_DNLOAD_IDLE));

            /* Download block 3 (offset 1) len 0, trigger manifestation */
            let vec = dev.download(&mut dfu, 3, &[]).expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 1, DFU_MANIFEST));

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_MANIFEST_WAIT_RESET));

            /* Abort */
            let e = dev.abort(&mut dfu).expect_err("stall");
            assert_eq!(e, AnyUsbError::EP0Stalled);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_MANIFEST_WAIT_RESET));
        })
        .expect("with_usb");
}

/// DFU class with manifestation call that returns
struct MkDFUMTerr {}

impl UsbDeviceCtx for MkDFUMTerr {
    type C<'c> = DFUClass<EmulatedUsbBus, TestMem>;
    const EP0_SIZE: u8 = 32;

    fn create_class<'a>(
        &mut self,
        alloc: &'a UsbBusAllocator<EmulatedUsbBus>,
    ) -> AnyResult<DFUClass<EmulatedUsbBus, TestMem>> {
        fn manifestation(tm: &mut TestMem) -> Result<(), DFUManifestationError> {
            Err(DFUManifestationError::NotDone)
        }
        let overrides = TestMemOverride {
            read: None,
            erase: None,
            program: None,
            manifestation: Some(manifestation),
        };
        Ok(DFUClass::new(&alloc, TestMem::new(Some(overrides))))
    }
}

#[test]
fn test_manifestation_err_not_done() {
    MkDFUMTerr {}
        .with_usb(|mut dfu, mut dev| {
            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_IDLE));

            /* Download block 2 (offset 0) */
            let vec = dev.download(&mut dfu, 2, &[0; 128]).expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(
                vec,
                status(STATUS_OK, TestMem::PROGRAM_TIME_MS, DFU_DN_BUSY)
            );

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_DNLOAD_IDLE));

            /* Download block 3 (offset 1) len 0, trigger manifestation */
            let vec = dev.download(&mut dfu, 3, &[]).expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 1, DFU_MANIFEST));

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_ERR_NOTDONE, 0, DFU_ERROR));
        })
        .expect("with_usb");
}

struct MkDFUEraseErr {}

impl UsbDeviceCtx for MkDFUEraseErr {
    type C<'c> = DFUClass<EmulatedUsbBus, TestMem>;
    const EP0_SIZE: u8 = 32;

    fn create_class<'a>(
        &mut self,
        alloc: &'a UsbBusAllocator<EmulatedUsbBus>,
    ) -> AnyResult<DFUClass<EmulatedUsbBus, TestMem>> {
        fn erase(tm: &mut TestMem, address: u32) -> core::result::Result<(), DFUMemError> {
            Err(DFUMemError::CheckErased)
        }

        let overrides = TestMemOverride {
            read: None,
            erase: Some(erase),
            program: None,
            manifestation: None,
        };
        Ok(DFUClass::new(&alloc, TestMem::new(Some(overrides))))
    }
}

#[test]
fn test_erase_err_verfail() {
    MkDFUEraseErr {}
        .with_usb(|mut dfu, mut dev| {
            let blkaddr: u32 = TestMem::INITIAL_ADDRESS_POINTER;

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_IDLE));

            /* Download block 0 (command), erase = blkaddr */
            let b = blkaddr.to_le_bytes();
            let vec = dev
                .download(&mut dfu, 0, &[0x41, b[0], b[1], b[2], b[3]])
                .expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, TestMem::ERASE_TIME_MS, DFU_DN_BUSY));

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_ERR_CHECK_ERASED, 0, DFU_ERROR));
        })
        .expect("with_usb");
}

struct MkDFUProgErr {}

impl UsbDeviceCtx for MkDFUProgErr {
    type C<'c> = DFUClass<EmulatedUsbBus, TestMem>;
    const EP0_SIZE: u8 = 32;

    fn create_class<'a>(
        &mut self,
        alloc: &'a UsbBusAllocator<EmulatedUsbBus>,
    ) -> AnyResult<DFUClass<EmulatedUsbBus, TestMem>> {
        fn program(tm: &mut TestMem, address: u32, length: usize) -> Result<(), DFUMemError> {
            if address > TestMem::INITIAL_ADDRESS_POINTER {
                Err(DFUMemError::Write)
            } else {
                Err(DFUMemError::Prog)
            }
        }

        let overrides = TestMemOverride {
            read: None,
            erase: None,
            program: Some(program),
            manifestation: None,
        };
        Ok(DFUClass::new(&alloc, TestMem::new(Some(overrides))))
    }
}

#[test]
fn test_program_err_prog_write() {
    MkDFUProgErr {}
        .with_usb(|mut dfu, mut dev| {
            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_IDLE));

            /* Download block 2 (offset 0) */
            let vec = dev.download(&mut dfu, 2, &[0x55; 128]).expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(
                vec,
                status(STATUS_OK, TestMem::PROGRAM_TIME_MS, DFU_DN_BUSY)
            );

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_ERR_PROG, 0, DFU_ERROR));

            /* Clear Status */
            let vec = dev.clear_status(&mut dfu).expect("vec");
            assert_eq!(vec, []);

            /* Download block 3 (offset 1) */
            let vec = dev.download(&mut dfu, 3, &[0x55; 128]).expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(
                vec,
                status(STATUS_OK, TestMem::PROGRAM_TIME_MS, DFU_DN_BUSY)
            );

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_ERR_WRITE, 0, DFU_ERROR));
        })
        .expect("with_usb");
}

struct MkDFUReadErr {}

impl UsbDeviceCtx for MkDFUReadErr {
    type C<'c> = DFUClass<EmulatedUsbBus, TestMem>;
    const EP0_SIZE: u8 = 32;

    fn create_class<'a>(
        &mut self,
        alloc: &'a UsbBusAllocator<EmulatedUsbBus>,
    ) -> AnyResult<DFUClass<EmulatedUsbBus, TestMem>> {
        fn read(
            tm: &mut TestMem,
            address: u32,
            length: usize,
        ) -> core::result::Result<&[u8], DFUMemError> {
            if address > TestMem::INITIAL_ADDRESS_POINTER {
                Err(DFUMemError::ErrVendor)
            } else {
                Err(DFUMemError::Address)
            }
        }

        let overrides = TestMemOverride {
            read: Some(read),
            erase: None,
            program: None,
            manifestation: None,
        };
        Ok(DFUClass::new(&alloc, TestMem::new(Some(overrides))))
    }
}

#[test]
fn test_read_err_addr_vend() {
    MkDFUReadErr {}
        .with_usb(|mut dfu, mut dev| {
            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_IDLE));

            /* Upload block 2 (offset 0) */
            let e = dev.upload(&mut dfu, 2, 128).expect_err("stall");
            assert_eq!(e, AnyUsbError::EP0Stalled);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_ERR_ADDRESS, 0, DFU_ERROR));

            /* Clear Status */
            let vec = dev.clear_status(&mut dfu).expect("vec");
            assert_eq!(vec, []);

            /* Upload block 3 (offset 1*128) */
            let e = dev.upload(&mut dfu, 3, 128).expect_err("stall");
            assert_eq!(e, AnyUsbError::EP0Stalled);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_ERR_VENDOR, 0, DFU_ERROR));
        })
        .expect("with_usb");
}

#[test]
fn test_download_program_short() {
    MkDFU {}
        .with_usb(|mut dfu, mut dev| {
            assert!(16 < TestMem::TRANSFER_SIZE);

            let mut blkaddr: u32 = TestMem::INITIAL_ADDRESS_POINTER;

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_IDLE));

            /* Download block 0 (command), erase = blkaddr */
            let mut b = blkaddr.to_le_bytes();
            let vec = dev
                .download(&mut dfu, 0, &[0x41, b[0], b[1], b[2], b[3]])
                .expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, TestMem::ERASE_TIME_MS, DFU_DN_BUSY));

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_DNLOAD_IDLE));

            /* Download block 2 (offset 0), full block of 0x55 */
            let vec = dev.download(&mut dfu, 2, &[0x55; 128]).expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(
                vec,
                status(STATUS_OK, TestMem::PROGRAM_TIME_MS, DFU_DN_BUSY)
            );

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_DNLOAD_IDLE));

            blkaddr = TestMem::INITIAL_ADDRESS_POINTER + 128;

            /* Download block 0 (command), address pointer = blkaddr */
            b = blkaddr.to_le_bytes();
            let vec = dev
                .download(&mut dfu, 0, &[0x21, b[0], b[1], b[2], b[3]])
                .expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_DN_BUSY));

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_DNLOAD_IDLE));

            /* Download block 2 (offset 0), new address, short block of 0xaa */
            let vec = dev.download(&mut dfu, 2, &[0xaa; 16]).expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(
                vec,
                status(STATUS_OK, TestMem::PROGRAM_TIME_MS, DFU_DN_BUSY)
            );

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_DNLOAD_IDLE));

            blkaddr = TestMem::INITIAL_ADDRESS_POINTER;

            /* Download block 0 (command), address pointer = blkaddr */
            b = blkaddr.to_le_bytes();
            let vec = dev
                .download(&mut dfu, 0, &[0x21, b[0], b[1], b[2], b[3]])
                .expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_DN_BUSY));

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_DNLOAD_IDLE));

            /* Abort */
            let vec = dev.abort(&mut dfu).expect("vec");
            assert_eq!(vec, []);

            /* Upload block 2 (offset 0) - must be 0x55 */
            let vec = dev.upload(&mut dfu, 2, 128).expect("vec");
            assert_eq!(vec.len(), 128);
            assert_eq!(vec[0..128], [0x55; 128]);

            /* Upload block 3 (offset 1) - must be 0xaa and 0xff */
            let vec = dev.upload(&mut dfu, 3, 128).expect("vec");
            assert_eq!(vec.len(), 128);
            let mut refblock = [0xffu8; 128];
            refblock[0..16].fill(0xaa);
            assert_eq!(vec[0..128], refblock);
        })
        .expect("with_usb");
}

#[test]
fn test_status_err_small_buffer() {
    MkDFU {}
        .with_usb(|mut dfu, mut dev| {
            /* Get Status, buffer is 5 bytes instead of 6 */
            let e = dev.read(&mut dfu, 3, 0, 0, 5).expect_err("stall");
            assert_eq!(e, AnyUsbError::EP0Stalled);
        })
        .expect("with_usb");
}

#[test]
fn test_state_err_small_buffer() {
    MkDFU {}
        .with_usb(|mut dfu, mut dev| {
            /* Get State, buffer is 0 bytes instead of 1 */
            let e = dev.read(&mut dfu, 5, 0, 0, 0).expect_err("stall");
            assert_eq!(e, AnyUsbError::EP0Stalled);
        })
        .expect("with_usb");
}

#[test]
fn test_commands_err_small_buffer() {
    MkDFU {}
        .with_usb(|mut dfu, mut dev| {
            /* Upload block 0 (get commands), 2 byte buffer */
            let e = dev.upload(&mut dfu, 0, 2).expect_err("stall");
            assert_eq!(e, AnyUsbError::EP0Stalled);
        })
        .expect("with_usb");
}

#[test]
fn test_err_addr_overflow() {
    MkDFU {}
        .with_usb(|mut dfu, mut dev| {
            let invalid_addr: u32 = 0xffff_fff0;

            /* Download block 0 (command), address pointer = invalid_addr */
            let b = invalid_addr.to_le_bytes();
            let vec = dev
                .download(&mut dfu, 0, &[0x21, b[0], b[1], b[2], b[3]])
                .expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_DN_BUSY));
            assert_eq!(dfu.get_address_pointer(), invalid_addr);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_DNLOAD_IDLE));

            /* Download block 3 (offset 1), real start address 0x1_0000_0070 */
            let vec = dev.download(&mut dfu, 3, &[0x55; 128]).expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(
                vec,
                status(STATUS_OK, TestMem::PROGRAM_TIME_MS, DFU_DN_BUSY)
            );

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_ERR_ADDRESS, 0, DFU_ERROR));

            /* Clear Status */
            let vec = dev.clear_status(&mut dfu).expect("vec");
            assert_eq!(vec, []);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_OK, 0, DFU_IDLE));

            /* Upload block 3 (offset 1) - real start address 0x1_0000_0070 */
            let e = dev.upload(&mut dfu, 3, 128).expect_err("stall");
            assert_eq!(e, AnyUsbError::EP0Stalled);

            /* Get Status */
            let vec = dev.get_status(&mut dfu).expect("vec");
            assert_eq!(vec, status(STATUS_ERR_ADDRESS, 0, DFU_ERROR));
        })
        .expect("with_usb");
}
