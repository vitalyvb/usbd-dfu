#![allow(unused_variables)]

use usb_device::bus::UsbBusAllocator;
use usbd_dfu::class::*;

pub struct TestMem {
}


impl TestMem {
    fn new() -> Self {
        Self {
        }
    }

}

const TESTMEM_BASE : u32 = 0x0200_0000;

impl DFUMemIO for TestMem {
    const INITIAL_ADDRESS_POINTER : u32 = TESTMEM_BASE;
    const MANIFESTATION_TOLERANT : bool = true;
    const MANIFESTATION_TIME_MS : u32 = 0x123;
    const PAGE_PROGRAM_TIME_MS : u32 = 0;
    const PAGE_ERASE_TIME_MS : u32 = 0;
    const FULL_ERASE_TIME_MS : u32 = 0;
    const MEM_INFO_STRING: &'static str = "@Flash/0x02000000/16*1Ka,48*1Kg";
    const HAS_DOWNLOAD: bool = false;
    const HAS_UPLOAD: bool = false;
    const DETACH_TIMEOUT: u16 = 0x1122;
    const TRANSFER_SIZE: u16 = 128;
    // const MEMIO_IN_USB_INTERRUPT: bool = false;

    fn read_block(&mut self, address: u32, length: usize) -> core::result::Result<&[u8], DFUMemError> {
        Err(DFUMemError::Address)
    }

    fn erase_block(&mut self, address: u32) -> core::result::Result<(), DFUMemError> {
        Ok(())
    }

    fn erase_all_blocks(&mut self) -> Result<(), DFUMemError> {
        Ok(())   
    }

    fn store_write_buffer(&mut self, src:&[u8]) -> core::result::Result<(), ()>{
        Ok(())
    }

    fn program_block(&mut self, address: u32, length: usize) -> core::result::Result<(), DFUMemError>{
        Err(DFUMemError::Address)
    }

    fn manifestation(&mut self) -> Result<(), DFUManifestationError> {
        Ok(())
    }
}

mod mockusb;
use mockusb::*;

/// Default DFU class factory
struct MkDFU {}

impl ClsMaker<TestBus, DFUClass<TestBus, TestMem>> for MkDFU {
    fn create<'a>(&mut self, alloc: &'a UsbBusAllocator<TestBus>) -> DFUClass<TestBus, TestMem> {
        DFUClass::new(&alloc, TestMem::new())
    }

    // fn poll(&mut self, dfu:&mut DFUClass<TestBus, TestMem>) {
    //     if dfu.update_pending() {
    //         dfu.update();
    //     }
    // }
}



#[test]
fn test_manifestation() {
    with_usb(&mut MkDFU{}, |mut dfu, transact| {
        let mut buf = [0u8;256];
        let mut len;

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0,0, 0,0, 6,0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0,0,0,0,2,0]); // dfuIdle

        /* Download block 3 (offset 1) len 0, trigger manifestation */
        len = transact(&mut dfu, &[0x21, 0x1, 3,0, 0,0, 0,0], None, &mut buf).expect("len");
        assert_eq!(len, 0);

        /* Get State */
        len = transact(&mut dfu, &[0xa1, 0x5, 0,0, 0,0, 1,0], None, &mut buf).expect("len");
        assert_eq!(len, 1);
        assert_eq!(buf[0], 6); // dfuManifestSync
        
        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0,0, 0,0, 6,0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0,0x23,0x1,0,7,0]); // dfuManifest

        /* Get State */
        len = transact(&mut dfu, &[0xa1, 0x5, 0,0, 0,0, 1,0], None, &mut buf).expect("len");
        assert_eq!(len, 1);
        assert_eq!(buf[0], 6); // dfuManifestSync
        
        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0,0, 0,0, 6,0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0,0,0,0,2,0]); // dfuIdle
    });
}

#[test]
fn test_err_por() {
    with_usb(&mut MkDFU{}, |mut dfu, transact| {
        let mut buf = [0u8;256];
        let mut len;

        dfu.set_unexpected_reset_state();

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0,0, 0,0, 6,0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[13,0,0,0,10,0]); // DfuError: ErrPOR

        /* Clear Status */
        len = transact(&mut dfu, &[0x21, 0x4, 0,0, 0,0, 0,0], None, &mut buf).expect("len");
        assert_eq!(len, 0);

        /* Get Status */
        len = transact(&mut dfu, &[0xa1, 0x3, 0,0, 0,0, 6,0], None, &mut buf).expect("len");
        assert_eq!(len, 6);
        assert_eq!(&buf[0..6], &[0,0,0,0,2,0]); // dfuIdle
    });
}

