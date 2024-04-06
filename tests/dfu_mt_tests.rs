#![allow(unused_variables)]

mod helpers;
use helpers::*;

use usbd_class_tester::prelude::*;

use usb_device::bus::UsbBusAllocator;
use usbd_dfu::class::*;

pub struct TestMem {}

impl TestMem {
    fn new() -> Self {
        Self {}
    }
}

const TESTMEM_BASE: u32 = 0x0200_0000;

impl DFUMemIO for TestMem {
    const INITIAL_ADDRESS_POINTER: u32 = TESTMEM_BASE;
    const MANIFESTATION_TOLERANT: bool = true;
    const MANIFESTATION_TIME_MS: u32 = 0x123;
    const PROGRAM_TIME_MS: u32 = 0;
    const ERASE_TIME_MS: u32 = 0;
    const FULL_ERASE_TIME_MS: u32 = 0;
    const MEM_INFO_STRING: &'static str = "@Flash/0x02000000/16*1Ka,48*1Kg";
    const HAS_DOWNLOAD: bool = false;
    const HAS_UPLOAD: bool = false;
    const DETACH_TIMEOUT: u16 = 0x1122;
    const TRANSFER_SIZE: u16 = 128;
    // const MEMIO_IN_USB_INTERRUPT: bool = false;

    fn read(&mut self, address: u32, length: usize) -> core::result::Result<&[u8], DFUMemError> {
        Err(DFUMemError::Address)
    }

    fn erase(&mut self, address: u32) -> core::result::Result<(), DFUMemError> {
        Ok(())
    }

    fn erase_all(&mut self) -> Result<(), DFUMemError> {
        Ok(())
    }

    fn store_write_buffer(&mut self, src: &[u8]) -> core::result::Result<(), ()> {
        Ok(())
    }

    fn program(&mut self, address: u32, length: usize) -> core::result::Result<(), DFUMemError> {
        Err(DFUMemError::Address)
    }

    fn manifestation(&mut self) -> Result<(), DFUManifestationError> {
        Ok(())
    }
}

/// Default DFU class factory
struct MkDFU {}

impl UsbDeviceCtx<EmulatedUsbBus, DFUClass<EmulatedUsbBus, TestMem>> for MkDFU {
    fn create_class<'a>(
        &mut self,
        alloc: &'a UsbBusAllocator<EmulatedUsbBus>,
    ) -> AnyResult<DFUClass<EmulatedUsbBus, TestMem>> {
        Ok(DFUClass::new(&alloc, TestMem::new()))
    }

    // fn poll(&mut self, dfu:&mut DFUClass<TestBus, TestMem>) {
    //     if dfu.update_pending() {
    //         dfu.update();
    //     }
    // }
}

#[test]
fn test_manifestation() {
    with_usb(MkDFU {}, |mut dfu, mut dev| {
        let mut vec: Vec<u8>;

        /* Get Status */
        vec = dev.get_status(&mut dfu).expect("vec");
        assert_eq!(&vec[..], &status(STATUS_OK, 0, DFU_IDLE));

        /* Download block 3 (offset 1) len 0, trigger manifestation */
        vec = dev.download(&mut dfu, 3, &[]).expect("vec");
        assert_eq!(&vec[..], &[]);

        /* Get State */
        vec = dev.get_state(&mut dfu).expect("vec");
        assert_eq!(&vec[..], &[DFU_MANIFEST_SYNC]);

        /* Get Status */
        vec = dev.get_status(&mut dfu).expect("vec");
        assert_eq!(&vec[..], &status(STATUS_OK, 0x123, DFU_MANIFEST));

        /* Get State */
        vec = dev.get_state(&mut dfu).expect("vec");
        assert_eq!(&vec[..], &[DFU_MANIFEST_SYNC]);

        /* Get Status */
        vec = dev.get_status(&mut dfu).expect("vec");
        assert_eq!(&vec[..], &status(STATUS_OK, 0, DFU_IDLE));
    })
    .expect("with_usb");
}

#[test]
fn test_err_por() {
    with_usb(MkDFU {}, |mut dfu, mut dev| {
        let mut vec: Vec<u8>;

        dfu.set_unexpected_reset_state();

        /* Get Status */
        vec = dev.get_status(&mut dfu).expect("vec");
        assert_eq!(&vec[..], &status(STATUS_ERR_POR, 0, DFU_ERROR));

        /* Clear Status */
        vec = dev.clear_status(&mut dfu).expect("vec");
        assert_eq!(&vec[..], &[]);

        /* Get Status */
        vec = dev.get_status(&mut dfu).expect("vec");
        assert_eq!(&vec[..], &status(STATUS_OK, 0, DFU_IDLE));
    })
    .expect("with_usb");
}
