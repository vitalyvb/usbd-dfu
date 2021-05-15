use std::{cell::RefCell, cmp::min, rc::Rc};

use usb_device::bus::PollResult;
use usb_device::bus::{UsbBus, UsbBusAllocator};
use usb_device::class::UsbClass;
use usb_device::endpoint::{EndpointAddress, EndpointType};
use usb_device::prelude::*;
use usb_device::{Result, UsbDirection};

#[derive(Debug, PartialEq, Eq)]
pub enum EPErr {
    Stalled,
}

// #[derive(Clone, Copy)]
struct EP {
    alloc: bool,
    stall: bool,
    read_len: usize,
    read: [u8; 1024],
    read_ready: bool,
    write_len: usize,
    write: [u8; 1024],
    write_done: bool,
    setup: bool,
    max_size: usize,
}
impl EP {
    fn new() -> Self {
        EP {
            alloc: false,
            stall: false,
            read_len: 0,
            read: [0; 1024],
            read_ready: false,
            write_len: 0,
            write: [0; 1024],
            write_done: false,
            setup: false,
            max_size: 0,
        }
    }

    fn set_read(&mut self, data: &[u8], setup: bool) {
        self.read_len = data.len();
        for (i, v) in data.iter().enumerate() {
            self.read[i] = *v;
        }
        self.setup = setup;
        self.read_ready = true;
    }

    fn get_write(&mut self, data: &mut [u8]) -> usize {
        let res = self.write_len;
        dbg!("g", self.write_len);
        self.write_len = 0;
        data[..res].clone_from_slice(&self.write[..res]);
        self.write_done = true;
        res
    }
}
struct TestBusIO {
    ep_i: [RefCell<EP>; 4],
    ep_o: [RefCell<EP>; 4],
}

unsafe impl Sync for TestBusIO {}

impl TestBusIO {
    fn new() -> Self {
        Self {
            ep_i: [
                RefCell::new(EP::new()),
                RefCell::new(EP::new()),
                RefCell::new(EP::new()),
                RefCell::new(EP::new()),
            ],
            ep_o: [
                RefCell::new(EP::new()),
                RefCell::new(EP::new()),
                RefCell::new(EP::new()),
                RefCell::new(EP::new()),
            ],
        }
    }

    fn epidx(&self, ep_addr: EndpointAddress) -> &RefCell<EP> {
        match ep_addr.direction() {
            UsbDirection::In => self.ep_i.get(ep_addr.index()).unwrap(),
            UsbDirection::Out => self.ep_o.get(ep_addr.index()).unwrap(),
        }
    }

    fn get_write(&self, ep_addr: EndpointAddress, data: &mut [u8]) -> usize {
        let mut ep = self.epidx(ep_addr).borrow_mut();
        ep.get_write(data)
    }

    fn set_read(&self, ep_addr: EndpointAddress, data: &[u8], setup: bool) {
        let mut ep = self.epidx(ep_addr).borrow_mut();
        if setup && ep_addr.index() == 0 && ep_addr.direction() == UsbDirection::Out {
            // setup packet on EP0OUT removes stall condition
            ep.stall = false;
            let mut ep0in = self.ep_i.get(0).unwrap().borrow_mut();
            ep0in.stall = false;
        }
        ep.set_read(data, setup)
    }
    fn stalled0(&self) -> bool {
        let in0 = EndpointAddress::from_parts(0, UsbDirection::In);
        let out0 = EndpointAddress::from_parts(0, UsbDirection::Out);
        {
            let ep = self.epidx(in0).borrow();
            if ep.stall {
                return true;
            }
        }
        {
            let ep = self.epidx(out0).borrow();
            if ep.stall {
                return true;
            }
        }
        false
    }
}

pub struct TestBus {
    rrio: Rc<RefCell<TestBusIO>>,
}

unsafe impl Sync for TestBus {}

impl TestBus {
    fn new(rrio: &Rc<RefCell<TestBusIO>>) -> Self {
        Self { rrio: rrio.clone() }
    }
    fn io(&self) -> &RefCell<TestBusIO> {
        self.rrio.as_ref()
    }
}

impl usb_device::bus::UsbBus for TestBus {
    fn alloc_ep(
        &mut self,
        _ep_dir: UsbDirection,
        ep_addr: Option<EndpointAddress>,
        _ep_type: EndpointType,
        max_packet_size: u16,
        _interval: u8,
    ) -> Result<EndpointAddress> {
        if let Some(ea) = ep_addr {
            let io = self.io().borrow();
            let mut sep = io.epidx(ea).borrow_mut();
            assert!(!sep.alloc);
            sep.alloc = true;
            sep.stall = false;
            sep.max_size = max_packet_size as usize;

            Ok(ea)
        } else {
            panic!("ep_addr is required, endpoint allocation is not implemented");
        }
    }
    fn enable(&mut self) {}
    fn force_reset(&self) -> Result<()> {
        Ok(())
    }
    fn poll(&self) -> PollResult {
        let in0 = EndpointAddress::from_parts(0, UsbDirection::In);
        let out0 = EndpointAddress::from_parts(0, UsbDirection::Out);

        let io = self.io().borrow();
        let ep0out = io.epidx(out0).borrow();
        let mut ep0in = io.epidx(in0).borrow_mut();

        let ep0_write_done = ep0in.write_done;
        let ep0_can_read = ep0out.read_ready | ep0in.read_ready;
        let ep0_setup = ep0out.setup;

        ep0in.write_done = false;
        // dbg!(ep0out.read_ready , ep0in.read_ready);

        dbg!(ep0_write_done, ep0_can_read, ep0_setup);

        if ep0_write_done || ep0_can_read || ep0_setup {
            PollResult::Data {
                ep_in_complete: if ep0_write_done { 1 } else { 0 },
                ep_out: if ep0_can_read { 1 } else { 0 },
                ep_setup: if ep0_setup { 1 } else { 0 },
            }
        } else {
            PollResult::None
        }
    }
    fn read(&self, ep_addr: EndpointAddress, buf: &mut [u8]) -> Result<usize> {
        let io = self.io().borrow();
        let mut ep = io.epidx(ep_addr).borrow_mut();
        let len = min(buf.len(), min(ep.read_len, ep.max_size));

        dbg!("read len from", buf.len(), len, ep_addr);

        if len == 0 {
            return Err(UsbError::WouldBlock);
        }

        buf[..len].clone_from_slice(&ep.read[..len]);

        ep.read_len -= len;
        ep.read.copy_within(len.., 0);

        if ep.read_len == 0 {
            ep.setup = false;
        }

        ep.read_ready = ep.read_len > 0;

        Ok(len)
    }
    fn reset(&self) {}
    fn resume(&self) {}
    fn suspend(&self) {}
    fn set_device_address(&self, addr: u8) {
        assert_eq!(addr, 5);
    }
    fn is_stalled(&self, ep_addr: EndpointAddress) -> bool {
        let io = self.io().borrow();
        let ep = io.epidx(ep_addr).borrow();
        ep.stall
    }
    fn set_stalled(&self, ep_addr: EndpointAddress, stalled: bool) {
        let io = self.io().borrow();
        let mut ep = io.epidx(ep_addr).borrow_mut();
        ep.stall = stalled;
    }
    fn write(&self, ep_addr: EndpointAddress, buf: &[u8]) -> Result<usize> {
        let io = self.io().borrow();
        let mut ep = io.epidx(ep_addr).borrow_mut();
        let offset = ep.write_len;
        let mut len = 0;

        dbg!("write", buf.len());

        if buf.len() > ep.max_size {
            return Err(UsbError::BufferOverflow);
        }

        for (i, e) in ep.write[offset..].iter_mut().enumerate() {
            if i >= buf.len() {
                break;
            }
            *e = buf[i];
            len += 1;
        }

        dbg!("wrote", len);
        ep.write_len += len;
        ep.write_done = false;
        Ok(len)
    }
}

const EP0_SIZE: u8 = 32;

pub trait ClsMaker<B: UsbBus, T> {
    fn create<'a>(&mut self, alloc: &'a UsbBusAllocator<B>) -> T;
    fn poll(&mut self, cls: &mut T) {}
}

pub fn with_usb<T, M>(
    maker: &mut M,
    case: fn(
        dfu: &mut T,
        transact: &mut dyn FnMut(
            &mut T,
            &[u8],
            Option<&[u8]>,
            &mut [u8],
        ) -> core::result::Result<usize, EPErr>,
    ),
) where
    T: UsbClass<TestBus>,
    M: ClsMaker<TestBus, T>,
{
    let stio: TestBusIO = TestBusIO::new();
    let io = Rc::new(RefCell::new(stio));
    let bus = TestBus::new(&io);

    let alloc: usb_device::bus::UsbBusAllocator<TestBus> = UsbBusAllocator::new(bus);

    let mut cls = maker.create(&alloc);

    let mut usb_dev = UsbDeviceBuilder::new(&alloc, UsbVidPid(0x1234, 0x1234))
        .manufacturer("Test")
        .product("Test")
        .serial_number("Test")
        .device_release(0x0200)
        .self_powered(false)
        .max_power(250)
        .max_packet_size_0(EP0_SIZE)
        .build();

    usb_dev.poll(&mut [&mut cls]);
    maker.poll(&mut cls);

    // helper function to communicate with the device

    let usb = io.as_ref();
    let dev = &mut usb_dev;

    let mut transact = |d: &mut T,
                        setup: &[u8],
                        data: Option<&[u8]>,
                        out: &mut [u8]|
     -> core::result::Result<usize, EPErr> {
        let out0 = EndpointAddress::from_parts(0, UsbDirection::Out);
        let in0 = EndpointAddress::from_parts(0, UsbDirection::In);

        usb.borrow().set_read(out0, setup, true);
        dev.poll(&mut [d]);
        maker.poll(d);
        if usb.borrow().stalled0() {
            return Err(EPErr::Stalled);
        }

        if let Some(val) = data {
            usb.borrow().set_read(out0, val, false);
            for i in 1..100 {
                let res = dev.poll(&mut [d]);
                maker.poll(d);
                if !res {
                    break;
                }
                if i >= 99 {
                    panic!("read too much");
                }
            }
            if usb.borrow().stalled0() {
                return Err(EPErr::Stalled);
            }
        };

        let mut len = 0;

        loop {
            let one = usb.borrow().get_write(in0, &mut out[len..]);
            dev.poll(&mut [d]);
            maker.poll(d);
            if usb.borrow().stalled0() {
                return Err(EPErr::Stalled);
            }

            len += one;
            if one < EP0_SIZE as usize {
                // short read - last block
                break;
            }
        }

        Ok(len)
    };

    // basic usb device setup
    {
        let mut buf = [0; 8];
        let mut len;

        // set address
        len = transact(&mut cls, &[0, 0x5, 5, 0, 0, 0, 0, 0], None, &mut buf).expect("len");
        assert_eq!(len, 0);

        // set configuration
        len = transact(&mut cls, &[0, 0x9, 0, 0, 0, 0, 0, 0], None, &mut buf).expect("len");
        assert_eq!(len, 0);

        // set interface
        len = transact(&mut cls, &[1, 0xb, 0, 0, 0, 0, 0, 0], None, &mut buf).expect("len");
        assert_eq!(len, 0);
    }

    // run test
    case(&mut cls, &mut transact);
}
