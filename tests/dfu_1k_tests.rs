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
    const TRANSFER_SIZE: u16 = 1024;

    fn read_block(
        &mut self,
        address: u32,
        length: usize,
    ) -> core::result::Result<&[u8], DFUMemError> {
        println!("Read {} {}", address, length);
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
fn test_1k_get_configuration() {
    with_usb(&mut MkDFU {}, |mut dfu, transact| {
        let mut buf = [0u8; 8192];
        let mut len;

        // get configuration descriptor
        len = transact(&mut dfu, &[0x80, 0x6, 0, 2, 0, 0, 0, 0x20], None, &mut buf).expect("len");
        assert!(len >= 27);

        let device = &buf[..9];
        let interf = &buf[9..18];
        let config = &buf[18..27];
        let tail = &buf[27..len];

        assert_eq!(tail.len(), 4096-27);
        dbg!(tail);
        // tail is [0x55; 101] ++ [x; 4096-27-101], where x = index/32

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
                0, 4, // transfer size
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
fn test_large_blocks_upload() {
    with_usb(&mut MkDFU {}, |mut dfu, transact| {
        let mut buf = [0u8; 2048];
        let mut len;

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 2, 0]); // dfuIdle

        /* Upload block 2 (offset 0) - 1k transfer == max_transfer -> 1k */
        len = transact(&mut dfu, &[0xa1, 0x2, 2, 0, 0, 0, 0, 4], None, &mut buf).expect("len");
        assert_eq!(len, 1024);
        assert_eq!(&buf[0..10], &[0, 0, 1, 0, 2, 0, 3, 0, 4, 0]);
        assert_eq!(&buf[120..128], &[60, 0, 61, 0, 62, 0, 63, 0]);
        assert_eq!(&buf[1016..1024], &[252, 1, 253, 1, 254, 1, 255, 1]);

        /* Upload block 2 (offset 0) - 512b transfer < max_transfer -> 512b */
        len = transact(&mut dfu, &[0xa1, 0x2, 2, 0, 0, 0, 0, 2], None, &mut buf).expect("len");
        assert_eq!(len, 512);
        assert_eq!(&buf[0..10], &[0, 0, 1, 0, 2, 0, 3, 0, 4, 0]);
        assert_eq!(&buf[120..128], &[60, 0, 61, 0, 62, 0, 63, 0]);
        assert_eq!(&buf[504..512], &[252, 0, 253, 0, 254, 0, 255, 0]);

        /* Upload block 2 (offset 0) - 2k transfer > max_transfer -> 1k */
        len = transact(&mut dfu, &[0xa1, 0x2, 2, 0, 0, 0, 0, 8], None, &mut buf).expect("len");
        assert_eq!(len, 1024);
        assert_eq!(&buf[0..10], &[0, 0, 1, 0, 2, 0, 3, 0, 4, 0]);
        assert_eq!(&buf[120..128], &[60, 0, 61, 0, 62, 0, 63, 0]);
        assert_eq!(&buf[1016..1024], &[252, 1, 253, 1, 254, 1, 255, 1]);

    });
}

#[test]
#[ignore]
fn test_1k_block_download_program0() {
    with_usb(&mut MkDFU {}, |mut dfu, transact| {
        let mut buf = [0u8; 1024];
        let mut len;

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0, 0, 0, 0, 6, 0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0, 0, 0, 0, 2, 0]); // dfuIdle

        /* Download block 2 (offset 0) */
        len = transact(
            &mut dfu,
            &[0x21, 0x1, 2, 0, 0, 0, 0, 4],
            Some(&[0; 1024]),
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

        /* Upload block 2 (offset 0) - must be zeroed */
        len = transact(&mut dfu, &[0xa1, 0x2, 2, 0, 0, 0, 0, 4], None, &mut buf).expect("len");
        assert_eq!(len, 1024);
        assert_eq!(&buf[0..128], &[0; 1024]);

    });
}

