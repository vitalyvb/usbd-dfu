#![allow(unused_variables)]

use std::{cell::RefCell, cmp::min};
use usb_device::bus::UsbBusAllocator;

use usbd_dfu::class::*;

const TESTMEMSIZE: usize = 64 * 1024;
pub struct TestMem {
    memory: RefCell<[u8; TESTMEMSIZE]>,
    buffer: [u8; 1024],
    overrides: TestMemOverride,
}

struct TestMemOverride {
    read_block: Option<
        fn(&mut TestMem, address: u32, length: usize) -> core::result::Result<&[u8], DFUMemError>,
    >,
    erase_block: Option<fn(&mut TestMem, address: u32) -> Result<(), DFUMemError>>,
    program_block: Option<
        fn(&mut TestMem, address: u32, length: usize) -> core::result::Result<(), DFUMemError>,
    >,
    manifestation: Option<fn(&mut TestMem) -> Result<(), DFUManifestationError>>,
}

impl TestMem {
    fn new(overrides: Option<TestMemOverride>) -> Self {
        let tmo = overrides.unwrap_or(TestMemOverride {
            read_block: None,
            erase_block: None,
            program_block: None,
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
    const PAGE_PROGRAM_TIME_MS: u32 = 50;
    const PAGE_ERASE_TIME_MS: u32 = 0x1ff;
    const FULL_ERASE_TIME_MS: u32 = 0x2_0304;
    const MEM_INFO_STRING: &'static str = "@Flash/0x02000000/16*1Ka,48*1Kg";
    const HAS_DOWNLOAD: bool = true;
    const HAS_UPLOAD: bool = true;
    const DETACH_TIMEOUT: u16 = 0x1122;
    const TRANSFER_SIZE: u16 = 128;

    fn read_block(
        &mut self,
        address: u32,
        length: usize,
    ) -> core::result::Result<&[u8], DFUMemError> {
        if self.overrides.read_block.is_some() {
            return self.overrides.read_block.unwrap()(self, address, length);
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

    fn erase_block(&mut self, address: u32) -> core::result::Result<(), DFUMemError> {
        if self.overrides.erase_block.is_some() {
            return self.overrides.erase_block.unwrap()(self, address);
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

    fn erase_all_blocks(&mut self) -> Result<(), DFUMemError> {
        for block in (0..TESTMEMSIZE).step_by(1024) {
            self.erase(block);
        }
        Ok(())
    }

    fn store_write_buffer(&mut self, src: &[u8]) -> core::result::Result<(), ()> {
        self.buffer[..src.len()].clone_from_slice(src);
        Ok(())
    }

    fn program_block(
        &mut self,
        address: u32,
        length: usize,
    ) -> core::result::Result<(), DFUMemError> {
        if self.overrides.program_block.is_some() {
            return self.overrides.program_block.unwrap()(self, address, length);
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

mod mockusb;
use mockusb::*;

/// Default DFU class factory
struct MkDFU {}

impl ClsMaker<TestBus, DFUClass<TestBus, TestMem>> for MkDFU {
    fn create<'a>(&mut self, alloc: &'a UsbBusAllocator<TestBus>) -> DFUClass<TestBus, TestMem> {
        DFUClass::new(&alloc, TestMem::new(None))
    }
}

#[test]
fn test_get_configuration() {
    with_usb(&mut MkDFU {}, |mut dfu, transact| {
        let mut buf = [0u8; 256];
        let mut len;

        // get configuration descriptor
        len = transact(&mut dfu, &[0x80, 0x6, 0, 2, 0, 0, 0x80, 0], None, &mut buf).expect("len");
        assert_eq!(len, 27);

        let device = &buf[..9];
        let interf = &buf[9..18];
        let config = &buf[18..len];

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

        // get string descriptor
        len = transact(&mut dfu, &[0x80, 0x6, 4, 3, 0, 0, 0x80, 0], None, &mut buf).expect("len");
        assert_eq!(len, 2 + TestMem::MEM_INFO_STRING.len() * 2);
        assert_eq!(&buf[0..2], &[len as u8, 3u8]);
        let u16v: Vec<_> = buf[2..len]
            .chunks(2)
            .map(|v| (v[0] as u16) | ((v[1] as u16) << 8))
            .collect();
        let istr = String::from_utf16(&u16v).unwrap();
        assert_eq!(istr, TestMem::MEM_INFO_STRING);
    });
}

#[test]
fn test_set_address_pointer() {
    with_usb(&mut MkDFU {}, |mut dfu, transact| {
        let mut buf = [0u8; 256];
        let mut len;

        let new_addr: u32 = 0x2000_0000;

        assert_ne!(new_addr, dfu.get_address_pointer());
        assert_eq!(dfu.get_address_pointer(), TestMem::INITIAL_ADDRESS_POINTER);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 2, 0]); // dfuIdle

        /* Download block 0 (command), address pointer = new_addr */
        let b = new_addr.to_le_bytes();
        len = transact(
            &mut dfu,
            &[0x21, 0x1, 0, 0, 0, 0, 5, 0],
            Some(&[0x21, b[0], b[1], b[2], b[3]]),
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 0);
        assert_eq!(dfu.get_address_pointer(), TestMem::INITIAL_ADDRESS_POINTER); // must change after Get Status

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 4, 0]); // dfuDnBusy
        assert_eq!(dfu.get_address_pointer(), new_addr);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 5, 0]); // dfuDnloadIdle
    });
}

#[test]
fn test_block_upload() {
    with_usb(&mut MkDFU {}, |mut dfu, transact| {
        let mut buf = [0u8; 256];
        let mut len;

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 2, 0]); // dfuIdle

        /* Upload block 2 (offset 0) */
        len = transact(&mut dfu, &[0xa1, 0x2, 2, 0, 0, 0, 128, 0], None, &mut buf).expect("len");
        assert_eq!(len, 128);
        assert_eq!(&buf[0..10], &[0, 0, 1, 0, 2, 0, 3, 0, 4, 0]);
        assert_eq!(&buf[120..128], &[60, 0, 61, 0, 62, 0, 63, 0]);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 9, 0]); // DfuUploadIdle

        /* Upload block 7 (offset 5*128) */
        len = transact(&mut dfu, &[0xa1, 0x2, 7, 0, 0, 0, 128, 0], None, &mut buf).expect("len");
        assert_eq!(len, 128);
        assert_eq!(&buf[0..10], &[64, 1, 65, 1, 66, 1, 67, 1, 68, 1]);
        assert_eq!(&buf[120..128], &[124, 1, 125, 1, 126, 1, 127, 1]);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 9, 0]); // DfuUploadIdle

        /* Abort */
        len = transact(&mut dfu, &[0x21, 0x6, 0, 0, 0, 0, 0, 0], None, &mut buf).expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 2, 0]); // dfuIdle
    });
}

#[test]
fn test_block_erase() {
    with_usb(&mut MkDFU {}, |mut dfu, transact| {
        let mut buf = [0u8; 256];
        let mut len;

        let blkaddr: u32 = TestMem::INITIAL_ADDRESS_POINTER + 1024;
        let et = TestMem::PAGE_ERASE_TIME_MS.to_le_bytes();

        assert_ne!(blkaddr, dfu.get_address_pointer());
        assert_ne!(0, TestMem::PAGE_ERASE_TIME_MS);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 2, 0]); // dfuIdle

        /* Download block 0 (command), erase = blkaddr */
        let b = blkaddr.to_le_bytes();
        len = transact(
            &mut dfu,
            &[0x21, 0x1, 0, 0, 0, 0, 5, 0],
            Some(&[0x41, b[0], b[1], b[2], b[3]]),
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, et[0], et[1], et[2], 4, 0]); // dfuDnBusy

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 5, 0]); // dfuDnloadIdle

        /* Abort */
        len = transact(&mut dfu, &[0x21, 0x6, 0, 0, 0, 0, 0, 0], None, &mut buf).expect("len");
        assert_eq!(len, 0);

        /* Upload block 9 (offset 7) - not erased */
        len = transact(&mut dfu, &[0xa1, 0x2, 9, 0, 0, 0, 128, 0], None, &mut buf).expect("len");
        assert_eq!(len, 128);
        assert_eq!(&buf[0..10], &[192, 1, 193, 1, 194, 1, 195, 1, 196, 1]);
        assert_eq!(&buf[120..128], &[252, 1, 253, 1, 254, 1, 255, 1]);

        /* Upload block 10 (offset 8) - erased */
        len = transact(&mut dfu, &[0xa1, 0x2, 10, 0, 0, 0, 128, 0], None, &mut buf).expect("len");
        assert_eq!(len, 128);
        assert_eq!(&buf[0..10], &[0xff; 10]);
        assert_eq!(&buf[120..128], &[0xff; 8]);

        /* Upload block 17 (offset 15) - erased */
        len = transact(&mut dfu, &[0xa1, 0x2, 15, 0, 0, 0, 128, 0], None, &mut buf).expect("len");
        assert_eq!(len, 128);
        assert_eq!(&buf[0..10], &[0xff; 10]);
        assert_eq!(&buf[120..128], &[0xff; 8]);

        /* Upload block 18 (offset 16) - not erased */
        len = transact(&mut dfu, &[0xa1, 0x2, 18, 0, 0, 0, 128, 0], None, &mut buf).expect("len");
        assert_eq!(len, 128);
        assert_eq!(&buf[0..10], &[0, 4, 1, 4, 2, 4, 3, 4, 4, 4]);
        assert_eq!(&buf[120..128], &[60, 4, 61, 4, 62, 4, 63, 4]);
    });
}

#[test]
fn test_block_erase_all() {
    with_usb(&mut MkDFU {}, |mut dfu, transact| {
        let mut buf = [0u8; 256];
        let mut len;

        let et = TestMem::FULL_ERASE_TIME_MS.to_le_bytes();

        assert_ne!(0, TestMem::FULL_ERASE_TIME_MS);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 2, 0]); // dfuIdle

        /* Download block 0 (command), erase = full */
        len = transact(
            &mut dfu,
            &[0x21, 0x1, 0, 0, 0, 0, 1, 0],
            Some(&[0x41]),
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, et[0], et[1], et[2], 4, 0]); // dfuDnBusy

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 5, 0]); // dfuDnloadIdle

        /* Abort */
        len = transact(&mut dfu, &[0x21, 0x6, 0, 0, 0, 0, 0, 0], None, &mut buf).expect("len");
        assert_eq!(len, 0);

        let mut blk = 2; // offset 2 is zeroth block
        loop {
            /* Upload block - erased */
            len = transact(
                &mut dfu,
                &[
                    0xa1,
                    0x2,
                    (blk & 0xff) as u8,
                    (blk >> 8) as u8,
                    0,
                    0,
                    128,
                    0,
                ],
                None,
                &mut buf,
            )
            .expect("len");

            if len == 0 {
                break;
            }

            dbg!(len);
            dbg!(&buf[0..len]);

            assert_eq!(len, 128);
            assert_eq!(&buf[0..len], &[0xffu8; 128]);

            blk += 1;
            assert!(blk < 0xffff);
        }
        assert_eq!(blk - 2, TESTMEMSIZE / 128);
    });
}

#[test]
fn test_block_upload_last() {
    with_usb(&mut MkDFU {}, |mut dfu, transact| {
        let mut buf = [0u8; 256];
        let mut len;

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 2, 0]); // dfuIdle

        /* Upload block 2 (offset 0) */
        len = transact(&mut dfu, &[0xa1, 0x2, 2, 0, 0, 0, 128, 0], None, &mut buf).expect("len");
        assert_eq!(len, 128);
        assert_eq!(&buf[0..10], &[0, 0, 1, 0, 2, 0, 3, 0, 4, 0]);
        assert_eq!(&buf[120..128], &[60, 0, 61, 0, 62, 0, 63, 0]);

        /* Upload block 513 (offset 511*128) - Last block */
        len = transact(
            &mut dfu,
            &[0xa1, 0x2, 0x01, 0x2, 0, 0, 128, 0],
            None,
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 128);
        assert_eq!(
            &buf[0..10],
            &[192, 127, 193, 127, 194, 127, 195, 127, 196, 127]
        );
        assert_eq!(&buf[120..128], &[252, 127, 253, 127, 254, 127, 255, 127]);

        /* Get Status,  */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 9, 0]); // DfuUploadIdle

        /* Upload block 514 (offset 512*128), short read */
        len = transact(
            &mut dfu,
            &[0xa1, 0x2, 0x02, 0x2, 0, 0, 128, 0],
            None,
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 0);

        /* Get Status, dfuIdle after short frame */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 2, 0]); // dfuIdle
    });
}

#[test]
fn test_block_upload_err_bad_address() {
    with_usb(&mut MkDFU {}, |mut dfu, transact| {
        let mut buf = [0u8; 256];
        let mut len;

        let invalid_addr: u32 = TestMem::INITIAL_ADDRESS_POINTER - 0x1_0000;

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 2, 0]); // dfuIdle

        /* Download block 0 (command), address pointer = invalid_addr */
        let b = invalid_addr.to_le_bytes();
        len = transact(
            &mut dfu,
            &[0x21, 0x1, 0, 0, 0, 0, 5, 0],
            Some(&[0x21, b[0], b[1], b[2], b[3]]),
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 0);
        assert_eq!(dfu.get_address_pointer(), TestMem::INITIAL_ADDRESS_POINTER); // must change after Get Status

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 4, 0]); // dfuDnBusy
        assert_eq!(dfu.get_address_pointer(), invalid_addr);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 5, 0]); // dfuDnloadIdle

        /* Abort */
        len = transact(&mut dfu, &[0x21, 0x6, 0, 0, 0, 0, 0, 0], None, &mut buf).expect("len");
        assert_eq!(len, 0);

        /* Upload block 2 (offset 0) */
        let e = transact(&mut dfu, &[0xa1, 0x2, 2, 0, 0, 0, 128, 0], None, &mut buf)
            .expect_err("stall");
        assert_eq!(e, EPErr::Stalled);

        /* Get Status,  */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[8, 0, 0, 0, 10, 0]); // DfuError: Address
    });
}

#[test]
fn test_block_download_to_upload_err() {
    with_usb(&mut MkDFU {}, |mut dfu, transact| {
        let mut buf = [0u8; 256];
        let mut len;

        let xaddr: u32 = TestMem::INITIAL_ADDRESS_POINTER;

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 2, 0]); // dfuIdle

        /* Download block 0 (command), address pointer */
        let b = xaddr.to_le_bytes();
        len = transact(
            &mut dfu,
            &[0x21, 0x1, 0, 0, 0, 0, 5, 0],
            Some(&[0x21, b[0], b[1], b[2], b[3]]),
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 4, 0]); // dfuDnBusy

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 5, 0]); // dfuDnloadIdle

        /* Can't call Upload from dfuDnloadIdle, expect stall */

        /* Upload block 2 (offset 0) */
        let e = transact(&mut dfu, &[0xa1, 0x2, 2, 0, 0, 0, 128, 0], None, &mut buf)
            .expect_err("stall");
        assert_eq!(e, EPErr::Stalled);

        /* Get Status,  */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[15, 0, 0, 0, 10, 0]); // DfuError: ErrStalledPkt
    });
}

#[test]
fn test_block_download_program0() {
    with_usb(&mut MkDFU {}, |mut dfu, transact| {
        let mut buf = [0u8; 256];
        let mut len;

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 2, 0]); // dfuIdle

        /* Download block 2 (offset 0) */
        len = transact(
            &mut dfu,
            &[0x21, 0x1, 2, 0, 0, 0, 128, 0],
            Some(&[0; 128]),
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 0);

        /* Get State */
        len = transact(&mut dfu, &[0xa1, 0x5, 0, 0, 0, 0, 1, 0], None, &mut buf).expect("len");
        assert_eq!(len, 1);
        assert_eq!(buf[0], 3); // dfuDnloadSync

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 50, 0, 0, 4, 0]); // dfuDnBusy

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 5, 0]); // dfuDnloadIdle

        /* Abort */
        len = transact(&mut dfu, &[0x21, 0x6, 0, 0, 0, 0, 0, 0], None, &mut buf).expect("len");
        assert_eq!(len, 0);

        /* Upload block 2 (offset 0) - must be zeroed */
        len = transact(&mut dfu, &[0xa1, 0x2, 2, 0, 0, 0, 128, 0], None, &mut buf).expect("len");
        assert_eq!(len, 128);
        assert_eq!(&buf[0..128], &[0; 128]);

        /* Upload block 3 (offset 1) - intact */
        len = transact(&mut dfu, &[0xa1, 0x2, 3, 0, 0, 0, 128, 0], None, &mut buf).expect("len");
        assert_eq!(len, 128);
        assert_eq!(&buf[0..10], &[64, 0, 65, 0, 66, 0, 67, 0, 68, 0]);
        assert_eq!(&buf[120..128], &[124, 0, 125, 0, 126, 0, 127, 0]);
    });
}

#[test]
fn test_block_download_program_err_verify_and_to_idle() {
    with_usb(&mut MkDFU {}, |mut dfu, transact| {
        let mut buf = [0u8; 256];
        let mut len;

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 2, 0]); // dfuIdle

        /* Download block 2 (offset 0) */
        len = transact(
            &mut dfu,
            &[0x21, 0x1, 2, 0, 0, 0, 128, 0],
            Some(&[0x55; 128]),
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 50, 0, 0, 4, 0]); // dfuDnBusy

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[7, 0, 0, 0, 10, 0]); // DfuError: ErrVerify

        /* Clear Status */
        len = transact(&mut dfu, &[0x21, 0x4, 0, 0, 0, 0, 0, 0], None, &mut buf).expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 2, 0]); // dfuIdle
    });
}

#[test]
fn test_block_erase_and_program() {
    with_usb(&mut MkDFU {}, |mut dfu, transact| {
        let mut buf = [0u8; 256];
        let mut len;

        let blkaddr: u32 = TestMem::INITIAL_ADDRESS_POINTER;

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 2, 0]); // dfuIdle

        /* Download block 0 (command), erase = blkaddr */
        let b = blkaddr.to_le_bytes();
        len = transact(
            &mut dfu,
            &[0x21, 0x1, 0, 0, 0, 0, 5, 0],
            Some(&[0x41, b[0], b[1], b[2], b[3]]),
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 255, 1, 0, 4, 0]); // dfuDnBusy

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 5, 0]); // dfuDnloadIdle

        /* Download block 2 (offset 0) */
        len = transact(
            &mut dfu,
            &[0x21, 0x1, 2, 0, 0, 0, 128, 0],
            Some(&[0x55; 128]),
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 50, 0, 0, 4, 0]); // dfuDnBusy

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 5, 0]); // dfuDnloadIdle

        /* Abort */
        len = transact(&mut dfu, &[0x21, 0x6, 0, 0, 0, 0, 0, 0], None, &mut buf).expect("len");
        assert_eq!(len, 0);

        /* Upload block 2 (offset 0) - must be 0x55 */
        len = transact(&mut dfu, &[0xa1, 0x2, 2, 0, 0, 0, 128, 0], None, &mut buf).expect("len");
        assert_eq!(len, 128);
        assert_eq!(&buf[0..128], &[0x55; 128]);

        /* Upload block 3 (offset 1) - must be 0xff */
        len = transact(&mut dfu, &[0xa1, 0x2, 3, 0, 0, 0, 128, 0], None, &mut buf).expect("len");
        assert_eq!(len, 128);
        assert_eq!(&buf[0..128], &[0xff; 128]);
    });
}

#[test]
#[should_panic(expected = "emulate device reset")]
fn test_manifestation() {
    with_usb(&mut MkDFU {}, |mut dfu, transact| {
        let mut buf = [0u8; 256];
        let mut len;

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 2, 0]); // dfuIdle

        /* Download block 2 (offset 0) */
        len = transact(
            &mut dfu,
            &[0x21, 0x1, 2, 0, 0, 0, 128, 0],
            Some(&[0; 128]),
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 50, 0, 0, 4, 0]); // dfuDnBusy

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 5, 0]); // dfuDnloadIdle

        /* Download block 3 (offset 1) len 0, trigger manifestation */
        len = transact(&mut dfu, &[0x21, 0x1, 3, 0, 0, 0, 0, 0], None, &mut buf).expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 1, 0, 0, 7, 0]); // dfuManifest

        unreachable!("device must reset");
    });
}

/// DFU class with manifestation call that returns
struct MkDFUMTret {}

impl ClsMaker<TestBus, DFUClass<TestBus, TestMem>> for MkDFUMTret {
    fn create<'a>(&mut self, alloc: &'a UsbBusAllocator<TestBus>) -> DFUClass<TestBus, TestMem> {
        fn manifestation(tm: &mut TestMem) -> Result<(), DFUManifestationError> {
            Ok(())
        }
        let overrides = TestMemOverride {
            read_block: None,
            erase_block: None,
            program_block: None,
            manifestation: Some(manifestation),
        };
        DFUClass::new(&alloc, TestMem::new(Some(overrides)))
    }
}

#[test]
fn test_manifestation_no_reset() {
    with_usb(&mut MkDFUMTret {}, |mut dfu, transact| {
        let mut buf = [0u8; 256];
        let mut len;

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 2, 0]); // dfuIdle

        /* Download block 2 (offset 0) */
        len = transact(
            &mut dfu,
            &[0x21, 0x1, 2, 0, 0, 0, 128, 0],
            Some(&[0; 128]),
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 50, 0, 0, 4, 0]); // dfuDnBusy

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 5, 0]); // dfuDnloadIdle

        /* Download block 3 (offset 1) len 0, trigger manifestation */
        len = transact(&mut dfu, &[0x21, 0x1, 3, 0, 0, 0, 0, 0], None, &mut buf).expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 1, 0, 0, 7, 0]); // dfuManifest

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 8, 0]); // DfuManifestWaitReset

        /* Abort */
        let e =
            transact(&mut dfu, &[0x21, 0x6, 0, 0, 0, 0, 0, 0], None, &mut buf).expect_err("stall");
        assert_eq!(e, EPErr::Stalled);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 8, 0]); // DfuManifestWaitReset
    });
}

/// DFU class with manifestation call that returns
struct MkDFUMTerr {}

impl ClsMaker<TestBus, DFUClass<TestBus, TestMem>> for MkDFUMTerr {
    fn create<'a>(&mut self, alloc: &'a UsbBusAllocator<TestBus>) -> DFUClass<TestBus, TestMem> {
        fn manifestation(tm: &mut TestMem) -> Result<(), DFUManifestationError> {
            Err(DFUManifestationError::NotDone)
        }
        let overrides = TestMemOverride {
            read_block: None,
            erase_block: None,
            program_block: None,
            manifestation: Some(manifestation),
        };
        DFUClass::new(&alloc, TestMem::new(Some(overrides)))
    }
}

#[test]
fn test_manifestation_err_not_done() {
    with_usb(&mut MkDFUMTerr {}, |mut dfu, transact| {
        let mut buf = [0u8; 256];
        let mut len;

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 2, 0]); // dfuIdle

        /* Download block 2 (offset 0) */
        len = transact(
            &mut dfu,
            &[0x21, 0x1, 2, 0, 0, 0, 128, 0],
            Some(&[0; 128]),
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 50, 0, 0, 4, 0]); // dfuDnBusy

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 5, 0]); // dfuDnloadIdle

        /* Download block 3 (offset 1) len 0, trigger manifestation */
        len = transact(&mut dfu, &[0x21, 0x1, 3, 0, 0, 0, 0, 0], None, &mut buf).expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 1, 0, 0, 7, 0]); // dfuManifest

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[9, 0, 0, 0, 10, 0]); // DfuError: ErrNotdone
    });
}

struct MkDFUEraseerr {}

impl ClsMaker<TestBus, DFUClass<TestBus, TestMem>> for MkDFUEraseerr {
    fn create<'a>(&mut self, alloc: &'a UsbBusAllocator<TestBus>) -> DFUClass<TestBus, TestMem> {
        fn erase_block(tm: &mut TestMem, address: u32) -> core::result::Result<(), DFUMemError> {
            Err(DFUMemError::CheckErased)
        }

        let overrides = TestMemOverride {
            read_block: None,
            erase_block: Some(erase_block),
            program_block: None,
            manifestation: None,
        };
        DFUClass::new(&alloc, TestMem::new(Some(overrides)))
    }
}

#[test]
fn test_erase_err_verfail() {
    with_usb(&mut MkDFUEraseerr {}, |mut dfu, transact| {
        let mut buf = [0u8; 256];
        let mut len;

        let blkaddr: u32 = TestMem::INITIAL_ADDRESS_POINTER;

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 2, 0]); // dfuIdle

        /* Download block 0 (command), erase = blkaddr */
        let b = blkaddr.to_le_bytes();
        len = transact(
            &mut dfu,
            &[0x21, 0x1, 0, 0, 0, 0, 5, 0],
            Some(&[0x41, b[0], b[1], b[2], b[3]]),
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 255, 1, 0, 4, 0]); // dfuDnBusy

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[5, 0, 0, 0, 10, 0]); // DfuError: errCheckErased
    });
}

struct MkDFUProgerr {}

impl ClsMaker<TestBus, DFUClass<TestBus, TestMem>> for MkDFUProgerr {
    fn create<'a>(&mut self, alloc: &'a UsbBusAllocator<TestBus>) -> DFUClass<TestBus, TestMem> {
        fn program_block(tm: &mut TestMem, address: u32, length: usize) -> Result<(), DFUMemError> {
            if address > TestMem::INITIAL_ADDRESS_POINTER {
                Err(DFUMemError::Write)
            } else {
                Err(DFUMemError::Prog)
            }
        }

        let overrides = TestMemOverride {
            read_block: None,
            erase_block: None,
            program_block: Some(program_block),
            manifestation: None,
        };
        DFUClass::new(&alloc, TestMem::new(Some(overrides)))
    }
}

#[test]
fn test_program_err_prog_write() {
    with_usb(&mut MkDFUProgerr {}, |mut dfu, transact| {
        let mut buf = [0u8; 256];
        let mut len;

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 2, 0]); // dfuIdle

        /* Download block 2 (offset 0) */
        len = transact(
            &mut dfu,
            &[0x21, 0x1, 2, 0, 0, 0, 128, 0],
            Some(&[0x55; 128]),
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 50, 0, 0, 4, 0]); // dfuDnBusy

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[6, 0, 0, 0, 10, 0]); // DfuError: errProg

        /* Clear Status */
        len = transact(&mut dfu, &[0x21, 0x4, 0, 0, 0, 0, 0, 0], None, &mut buf).expect("len");
        assert_eq!(len, 0);

        /* Download block 3 (offset 1) */
        len = transact(
            &mut dfu,
            &[0x21, 0x1, 3, 0, 0, 0, 128, 0],
            Some(&[0x55; 128]),
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 50, 0, 0, 4, 0]); // dfuDnBusy

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[3, 0, 0, 0, 10, 0]); // DfuError: errWrite
    });
}

struct MkDFUReadErr {}

impl ClsMaker<TestBus, DFUClass<TestBus, TestMem>> for MkDFUReadErr {
    fn create<'a>(&mut self, alloc: &'a UsbBusAllocator<TestBus>) -> DFUClass<TestBus, TestMem> {
        fn read_block(
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
            read_block: Some(read_block),
            erase_block: None,
            program_block: None,
            manifestation: None,
        };
        DFUClass::new(&alloc, TestMem::new(Some(overrides)))
    }
}

#[test]
fn test_read_err_addr_vend() {
    with_usb(&mut MkDFUReadErr {}, |mut dfu, transact| {
        let mut buf = [0u8; 256];
        let mut len;
        let mut e;

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 2, 0]); // dfuIdle

        /* Upload block 2 (offset 0) */
        e = transact(&mut dfu, &[0xa1, 0x2, 2, 0, 0, 0, 128, 0], None, &mut buf)
            .expect_err("stall");
        assert_eq!(e, EPErr::Stalled);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[8, 0, 0, 0, 10, 0]); // DfuError: errAddress

        /* Clear Status */
        len = transact(&mut dfu, &[0x21, 0x4, 0, 0, 0, 0, 0, 0], None, &mut buf).expect("len");
        assert_eq!(len, 0);

        /* Upload block 3 (offset 1*128) */
        e = transact(&mut dfu, &[0xa1, 0x2, 3, 0, 0, 0, 128, 0], None, &mut buf)
            .expect_err("stall");
        assert_eq!(e, EPErr::Stalled);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[11, 0, 0, 0, 10, 0]); // DfuError: ErrVendor
    });
}

#[test]
fn test_block_download_program_short() {
    with_usb(&mut MkDFU {}, |mut dfu, transact| {
        let mut buf = [0u8; 256];
        let mut len;

        assert!(16 < TestMem::TRANSFER_SIZE);

        let mut blkaddr: u32 = TestMem::INITIAL_ADDRESS_POINTER;

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 2, 0]); // dfuIdle

        /* Download block 0 (command), erase = blkaddr */
        let mut b = blkaddr.to_le_bytes();
        len = transact(
            &mut dfu,
            &[0x21, 0x1, 0, 0, 0, 0, 5, 0],
            Some(&[0x41, b[0], b[1], b[2], b[3]]),
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 255, 1, 0, 4, 0]); // dfuDnBusy

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 5, 0]); // dfuDnloadIdle

        /* Download block 2 (offset 0), full block of 0x55 */
        len = transact(
            &mut dfu,
            &[0x21, 0x1, 2, 0, 0, 0, 128, 0],
            Some(&[0x55; 128]),
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 50, 0, 0, 4, 0]); // dfuDnBusy

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 5, 0]); // dfuDnloadIdle

        blkaddr = TestMem::INITIAL_ADDRESS_POINTER + 128;

        /* Download block 0 (command), address pointer = blkaddr */
        b = blkaddr.to_le_bytes();
        len = transact(
            &mut dfu,
            &[0x21, 0x1, 0, 0, 0, 0, 5, 0],
            Some(&[0x21, b[0], b[1], b[2], b[3]]),
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 4, 0]); // dfuDnBusy

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 5, 0]); // dfuDnloadIdle

        /* Download block 2 (offset 0), new address, short block of 0xaa */
        len = transact(
            &mut dfu,
            &[0x21, 0x1, 2, 0, 0, 0, 16, 0],
            Some(&[0xaa; 16]),
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 50, 0, 0, 4, 0]); // dfuDnBusy

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 5, 0]); // dfuDnloadIdle

        blkaddr = TestMem::INITIAL_ADDRESS_POINTER;

        /* Download block 0 (command), address pointer = blkaddr */
        b = blkaddr.to_le_bytes();
        len = transact(
            &mut dfu,
            &[0x21, 0x1, 0, 0, 0, 0, 5, 0],
            Some(&[0x21, b[0], b[1], b[2], b[3]]),
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 4, 0]); // dfuDnBusy

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 5, 0]); // dfuDnloadIdle

        /* Abort */
        len = transact(&mut dfu, &[0x21, 0x6, 0, 0, 0, 0, 0, 0], None, &mut buf).expect("len");
        assert_eq!(len, 0);

        /* Upload block 2 (offset 0) - must be 0x55 */
        len = transact(&mut dfu, &[0xa1, 0x2, 2, 0, 0, 0, 128, 0], None, &mut buf).expect("len");
        assert_eq!(len, 128);
        assert_eq!(&buf[0..128], &[0x55; 128]);

        /* Upload block 3 (offset 1) - must be 0xaa and 0xff */
        len = transact(&mut dfu, &[0xa1, 0x2, 3, 0, 0, 0, 128, 0], None, &mut buf).expect("len");
        assert_eq!(len, 128);
        let mut refblock = [0xffu8; 128];
        refblock[0..16].fill(0xaa);
        assert_eq!(&buf[0..128], &refblock);
    });
}

#[test]
fn test_status_err_small_buffer() {
    with_usb(&mut MkDFU {}, |mut dfu, transact| {
        let mut buf = [0u8; 256];

        /* Get Status, buffer is 5 bytes instead of 6 */
        let e =
            transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 5, 0], None, &mut buf).expect_err("stall");
        assert_eq!(e, EPErr::Stalled);
    });
}

#[test]
fn test_state_err_small_buffer() {
    with_usb(&mut MkDFU {}, |mut dfu, transact| {
        let mut buf = [0u8; 256];

        /* Get State, buffer is 0 bytes instead of 1 */
        let e =
            transact(&mut dfu, &[0xa1, 0x5, 0, 0, 0, 0, 0, 0], None, &mut buf).expect_err("stall");
        assert_eq!(e, EPErr::Stalled);
    });
}

#[test]
fn test_commands_err_small_buffer() {
    with_usb(&mut MkDFU {}, |mut dfu, transact| {
        let mut buf = [0u8; 256];

        /* Upload block 0 (get commands), 2 byte buffer */
        let e =
            transact(&mut dfu, &[0xa1, 0x2, 0, 0, 0, 0, 2, 0], None, &mut buf).expect_err("stall");
        assert_eq!(e, EPErr::Stalled);
    });
}

#[test]
fn test_err_addr_overflow() {
    with_usb(&mut MkDFU {}, |mut dfu, transact| {
        let mut buf = [0u8; 256];
        let mut len;

        let invalid_addr: u32 = 0xffff_fff0;

        /* Download block 0 (command), address pointer = invalid_addr */
        let b = invalid_addr.to_le_bytes();
        len = transact(
            &mut dfu,
            &[0x21, 0x1, 0, 0, 0, 0, 5, 0],
            Some(&[0x21, b[0], b[1], b[2], b[3]]),
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 4, 0]); // dfuDnBusy
        assert_eq!(dfu.get_address_pointer(), invalid_addr);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 5, 0]); // dfuDnloadIdle

        /* Download block 3 (offset 1), real start address 0x1_0000_0070 */
        len = transact(
            &mut dfu,
            &[0x21, 0x1, 3, 0, 0, 0, 128, 0],
            Some(&[0x55; 128]),
            &mut buf,
        )
        .expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 50, 0, 0, 4, 0]); // dfuDnBusy

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[8, 0, 0, 0, 10, 0]); // DfuError: ErrAddress

        /* Clear Status */
        len = transact(&mut dfu, &[0x21, 0x4, 0, 0, 0, 0, 0, 0], None, &mut buf).expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 2, 0]); // dfuIdle

        /* Upload block 3 (offset 1) - real start address 0x1_0000_0070 */
        let e = transact(&mut dfu, &[0xa1, 0x2, 3, 0, 0, 0, 128, 0], None, &mut buf)
            .expect_err("stall");
        assert_eq!(e, EPErr::Stalled);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[8, 0, 0, 0, 10, 0]); // DfuError: ErrAddress
    });
}
