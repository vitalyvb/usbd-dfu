use core::cmp::min;
use core::marker::PhantomData;
use usb_device::{class_prelude::*, control::Request};

const USB_CLASS_APPLICATION_SPECIFIC: u8 = 0xFE;
const USB_SUBCLASS_DFU: u8 = 0x01;

#[allow(dead_code)]
const USB_PROTOCOL_RUN_TIME: u8 = 0x01;
const USB_PROTOCOL_DFU_MODE: u8 = 0x02;

#[allow(dead_code)]
const DFU_DETACH: u8 = 0x00;
const DFU_DNLOAD: u8 = 0x01;
const DFU_UPLOAD: u8 = 0x02;
const DFU_GETSTATUS: u8 = 0x03;
const DFU_CLRSTATUS: u8 = 0x04;
const DFU_GETSTATE: u8 = 0x05;
const DFU_ABORT: u8 = 0x06;

const DESC_DESCTYPE_DFU: u8 = 0x21;

const HAS_READ_UNPROTECT: bool = false;

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum DFUState {
    /// Device is running its normal application.
    #[allow(dead_code)]
    AppIdle = 0,
    /// Device is running its normal application, has received the DFU_DETACH request, and is waiting for a USB reset.
    #[allow(dead_code)]
    AppDetach = 1,
    /// Device is operating in the DFU mode and is waiting for requests.
    DfuIdle = 2,
    /// Device has received a block and is waiting for the host to solicit the status via DFU_GETSTATUS.
    DfuDnloadSync = 3,
    /// Device is programming a control-write block into its nonvolatile memories.
    DfuDnBusy = 4,
    /// Device is processing a download operation. Expecting DFU_DNLOAD requests.
    DfuDnloadIdle = 5,
    /// Device has received the final block of firmware from the hostand is waiting for receipt of DFU_GETSTATUS to begin the Manifestation phase; or device has completed the Manifestation phase and is waiting for receipt of DFU_GETSTATUS. (Devices that can enter this state after the Manifestation phase set bmAttributes bit bitManifestationTolerant to 1.)
    DfuManifestSync = 6,
    /// Device is in the Manifestation phase. (Not all devices will be able to respond to DFU_GETSTATUS when in this state.)
    DfuManifest = 7,
    /// Device has programmed its memories and is waiting for a USB reset or a power on reset. (Devices that must enter this state clear bitManifestationTolerant to 0.)
    DfuManifestWaitReset = 8,
    /// The device is processing an upload operation. Expecting DFU_UPLOAD requests.
    DfuUploadIdle = 9,
    /// An error has occurred. Awaiting the DFU_CLRSTATUS request.
    DfuError = 10,
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum DFUStatusCode {
    /// No error condition is present.
    OK = 0x00,
    /// File is not targeted for use by this device.
    ErrTarget = 0x01,
    /// File is for this device but fails some vendor-specific verification test.
    ErrFile = 0x02,
    /// Device is unable to write memory.
    ErrWrite = 0x03,
    /// Memory erase function failed.
    ErrErase = 0x04,
    /// Memory erase check failed.
    ErrCheckErased = 0x05,
    /// Program memory function failed.
    ErrProg = 0x06,
    /// Programmed memory failed verification.
    ErrVerify = 0x07,
    /// Cannot program memory due to received address that is out of range.
    ErrAddress = 0x08,
    /// Received DFU_DNLOAD with wLength = 0, but device does not think it has all of the data yet.
    ErrNotdone = 0x09,
    /// Device’s firmware is corrupt. It cannot return to run-time (non-DFU) operations.
    ErrFirmware = 0x0A,
    /// iString indicates a vendor-specific error.
    ErrVendor = 0x0B,
    /// Device detected unexpected USB reset signaling.
    ErrUsbr = 0x0C,
    /// Device detected unexpected power on reset.
    ErrPOR = 0x0D,
    /// Something went wrong, but the device does not know what it was.
    ErrUnknown = 0x0E,
    /// Device stalled an unexpected request.
    ErrStalledPkt = 0x0F,
}

#[repr(u8)]
enum DnloadCommand {
    GetCommands = 0x00,
    SetAddressPointer = 0x21,
    Erase = 0x41,
    ReadUnprotect = 0x92,
}

/// Errors that may happen when working with the memory
/// (reading, erasing, writting). These will be translated
/// to a corresponding error codes in DFU protocol.
#[repr(u8)]
pub enum DFUMemError {
    /// File is not targeted for use by this device.
    Target = DFUStatusCode::ErrTarget as u8,
    /// File is for this device but fails some vendor-specific verification test.
    File = DFUStatusCode::ErrFile as u8,
    /// Device is unable to write memory.
    Write = DFUStatusCode::ErrWrite as u8,
    /// Memory erase function failed.
    Erase = DFUStatusCode::ErrErase as u8,
    /// Memory erase check failed.
    CheckErased = DFUStatusCode::ErrCheckErased as u8,
    /// Program memory function failed.
    Prog = DFUStatusCode::ErrProg as u8,
    /// Programmed memory failed verification.
    Verify = DFUStatusCode::ErrVerify as u8,
    /// Something went wrong, but the device does not know what it was.
    Unknown = DFUStatusCode::ErrUnknown as u8,
    /// Cannot program memory due to received address that is out of range.
    Address = DFUStatusCode::ErrAddress as u8,
    /// A vendor-specific error. iString in DFU_GETSTATUS reply will always be 0.
    ErrVendor = DFUStatusCode::ErrVendor as u8,
}

/// Errors that may happen when device enter Manifestation phase
#[repr(u8)]
pub enum DFUManifestationError {
    /// File is not targeted for use by this device.
    Target = DFUStatusCode::ErrTarget as u8,
    /// File is for this device but fails some vendor-specific verification test.
    File = DFUStatusCode::ErrFile as u8,
    /// Received DFU_DNLOAD with wLength = 0, but device does not think it has all of the data yet.
    NotDone = DFUStatusCode::ErrNotdone as u8,
    /// Device’s firmware is corrupt. It cannot return to run-time (non-DFU) operations.
    Firmware = DFUStatusCode::ErrFirmware as u8,
    /// A vendor-specific error. iString in DFU_GETSTATUS reply will always be 0.
    ErrVendor = DFUStatusCode::ErrVendor as u8,
    /// Something went wrong, but the device does not know what it was.
    Unknown = DFUStatusCode::ErrUnknown as u8,
}

/// Trait that describes the abstraction used to access memory
/// on a device. [`DFUClass`] will call corresponding
/// functions and will use provided constants to tailor
/// DFU features and, for example time interval values that
/// are used in the protocol.
pub trait DFUMemIO {
    /// Specifies the default value of Address Pointer
    ///
    /// Usually, it's start address of a memory region.
    ///
    const INITIAL_ADDRESS_POINTER: u32;

    /// Specifies USB interface descriptor string. It should describe a memory region this interface works with.
    ///
    /// *Disclaimer*: I haven't found the specification, this is what it looks like from dfu-util sources.
    ///
    /// The string is formatted as follows:
    ///
    /// @ *name*/*address*/*area*[,*area*...]
    ///
    /// > `@` (at sign), `/` (slash), `,` (coma) symbols
    ///
    /// > *name* - Region name, e.g. "Flash"
    ///
    /// > *address* - Memory address of a regions, e.g. "0x08000000"
    ///
    /// > *area* - count of pages, page size, and supported operations for the region, e.g. 8*1Ke - 8 pages of 1024 bytes, available for reading and writing.
    ///
    /// Page size supports these suffixes: **K**, **M**, **G**, or ` ` (space) for bytes.
    ///
    /// And a letter that specifies region's supported operation:
    ///
    /// | letter | Read | Erase | Write |
    /// |--------|------|-------|-------|
    /// | **a**  |   +  |       |       |
    /// | **b**  |      |   +   |       |
    /// | **c**  |   +  |   +   |       |
    /// | **d**  |      |       |   +   |
    /// | **e**  |   +  |       |   +   |
    /// | **f**  |      |   +   |   +   |
    /// | **g**  |   +  |   +   |   +   |
    ///
    /// For example:
    /// ```text
    /// @Flash/0x08000000/16*1Ka,48*1Kg
    /// ```
    ///
    /// Denotes a memory region named "Flash", with a starting address `0x08000000`,
    /// the first 16 pages with a size 1K are available only for reading, and the next
    /// 48 1K-pages are avaiable for reading, erase, and write operations.
    const MEM_INFO_STRING: &'static str;

    /// If set, DFU descriptor will have *bitCanDnload* bit set. Default is `true`.
    ///
    /// Should be set to true if firmware download (host to device) is supported.
    const HAS_DOWNLOAD: bool = true;

    /// If set, DFU descriptor will have *bitCanUpload* bit set. Default is `true`.
    ///
    /// Should be set to true if firmware upload (device to host) is supported.
    const HAS_UPLOAD: bool = true;

    /// If set, DFU descriptor will have *bitManifestationTolerant* bit set. Default is `true`.
    ///
    /// See also [`MANIFESTATION_TIME_MS`](DFUMemIO::MANIFESTATION_TIME_MS).
    const MANIFESTATION_TOLERANT: bool = true;

    // /// Remove device's flash read protection. This operation should erase
    // /// memory contents.
    // const HAS_READ_UNPROTECT : bool = false;

    /// Time in milliseconds host must wait before issuing the next command after
    /// block program request.
    ///
    /// This is the time that program of one block or [`TRANSFER_SIZE`](DFUMemIO::TRANSFER_SIZE) bytes
    /// takes.
    ///
    /// DFU programs data as follows:
    ///
    /// > 1. Host transfers `TRANSFER_SIZE` bytes to a device
    /// > 2. Device stores this data in a buffer
    /// > 3. Host issues `DFU_GETSTATUS` command, confirms that device state is correct,
    /// >    and checks the reply for 24-bit value how much time it must wait
    /// >    before issuing the next command. Device, after submitting a reply
    /// >    starts program operation.
    /// > 4. After waiting for a specified number of milliseconds, host continues to send new commands.
    const BLOCK_PROGRAM_TIME_MS: u32;

    /// Similar to [`BLOCK_PROGRAM_TIME_MS`](DFUMemIO::BLOCK_PROGRAM_TIME_MS), but for a page erase operation.
    const PAGE_ERASE_TIME_MS: u32;

    /// Similar to [`BLOCK_PROGRAM_TIME_MS`](DFUMemIO::BLOCK_PROGRAM_TIME_MS), but for a full erase operation.
    const FULL_ERASE_TIME_MS: u32;

    /// Time in milliseconds host must wait after submitting the final firware download
    /// (host to device) command. Default is `1` ms.
    ///
    /// DFU protocol allows the device to enter a Manifestation state when it can activate
    /// the uploaded firmware.
    ///
    /// After the activation is completed, device may need to reset (if
    /// [`MANIFESTATION_TOLERANT`](DFUMemIO::MANIFESTATION_TOLERANT) is `false`), or it can return to IDLE state
    /// (if `MANIFESTATION_TOLERANT` is `true`)
    ///
    /// See also [`BLOCK_PROGRAM_TIME_MS`](DFUMemIO::BLOCK_PROGRAM_TIME_MS).
    const MANIFESTATION_TIME_MS: u32 = 1;

    /// wDetachTimeOut field in DFU descriptor. Default value: `250` ms.
    ///
    /// Probably unused if device does not support DFU in run-time mode to
    /// handle `DFU_DETACH` command.
    ///
    /// Time in milliseconds that device will wait after receipt of `DFU_DETACH` request
    /// if USB reset request is not received before reverting to a normal operation.
    const DETACH_TIMEOUT: u16 = 250;

    /// Maximum allowed transfer size. Default value: `128` bytes.
    ///
    /// This is the maximum size of a block for [`read_block()`](DFUMemIO::read_block) and [`program_block()`](DFUMemIO::program_block) functions.
    ///
    /// All DFU transfers use Control endpoint only.
    ///
    /// **Warning**: must be less or equal of `usb-device`'s control endpoint buffer size (usually `128` bytes),
    /// otherwise data transfers may fail for no obvious reason.
    const TRANSFER_SIZE: u16 = 128;

    // /// Not supported, implementation would probably need some
    // /// non-trivial locking.
    // const MEMIO_IN_USB_INTERRUPT: bool = true;

    /// Collect data which comes from USB, possibly in chunks, to a buffer in RAM.
    ///
    /// [`DFUClass`] does not have an internal memory buffer for a read/write operations,
    /// incoming data should be stored in a buffer managed by this trait's implementation.
    ///
    /// This function should not write data to Flash or trigger memory Erase.
    ///
    /// The same buffer may be shared for both write and read operations.
    /// DFU protocol will not trigger block write while sending data to host, and
    /// will ensure that buffer has valid data before program operation is requested.
    ///
    /// This function is called from `usb_dev.poll([])` (USB interrupt context).
    ///
    fn store_write_buffer(&mut self, src: &[u8]) -> Result<(), ()>;

    /// Read memory and return it to device.
    ///
    /// If Upload operation is supported ([`HAS_UPLOAD`](DFUMemIO::HAS_UPLOAD) is `true`), this function
    /// returns memory contents to a host.
    ///
    /// Implementation must check that address is in a target region and that the
    /// whole block fits in this region too.
    ///
    /// This function is called from `usb_dev.poll([])` (USB interrupt context).
    ///
    fn read_block(&mut self, address: u32, length: usize) -> Result<&[u8], DFUMemError>;

    /// Trigger block program
    ///
    /// Implementation must check that address is in a target region and that the
    /// whole block fits in this region too.
    ///
    /// This function is called from `usb_dev.poll([])` (USB interrupt context).
    // / This function by default is called from USB interrupt context, depending on
    // / [`MEMIO_IN_USB_INTERRUPT`](DFUMemIO::MEMIO_IN_USB_INTERRUPT) value.
    ///
    fn program_block(&mut self, address: u32, length: usize) -> Result<(), DFUMemError>;

    /// Trigger page erase.
    ///
    /// Implementation must ensure that address is valid, or return an error.
    ///
    /// This function is called from `usb_dev.poll([])` (USB interrupt context).
    // / This function by default is called from USB interrupt context, depending on
    // / [`MEMIO_IN_USB_INTERRUPT`](DFUMemIO::MEMIO_IN_USB_INTERRUPT) value.
    ///
    fn erase_page(&mut self, address: u32) -> Result<(), DFUMemError>;

    /// Trigger full erase.
    ///
    /// This function is called from `usb_dev.poll([])` (USB interrupt context).
    // / This function by default is called from USB interrupt context, depending on
    // / [`MEMIO_IN_USB_INTERRUPT`](DFUMemIO::MEMIO_IN_USB_INTERRUPT) value.
    ///
    fn erase_all_pages(&mut self) -> Result<(), DFUMemError>;

    /// Finish writing firmware to a persistent storage, and optionally activate it.
    ///
    /// This funciton should return if [`MANIFESTATION_TOLERANT`](DFUMemIO::MANIFESTATION_TOLERANT) is `true`.
    ///
    /// This funciton should not return `Ok()` if `MANIFESTATION_TOLERANT` is `false`.
    /// Instead device should activate and start new main firmware.
    ///
    /// This function is called from `usb_dev.poll([])` (USB interrupt context).
    // / This function by default is called from USB interrupt context, depending on
    // / [`MEMIO_IN_USB_INTERRUPT`](DFUMemIO::MEMIO_IN_USB_INTERRUPT) value.
    ///
    fn manifestation(&mut self) -> Result<(), DFUManifestationError>;

    /// Called every time when USB is reset.
    ///
    /// After firmware update is done, device should switch to an application
    /// firmware if it's possible and this function should not return.
    ///
    /// Handler will need to distinguish between actual host resets and
    /// when the device connects the first time at startup to avoid
    /// device reset and revert to main firmware at boot.
    ///
    /// If firmware is corrupt, this funciton should return and DFU will switch
    /// to ERROR state so host could try to recover. This is the default.
    ///
    /// This function is called from `usb_dev.poll([])` (USB interrupt context).
    ///
    fn usb_reset(&mut self) {}
}

impl From<DFUMemError> for DFUStatusCode {
    fn from(e: DFUMemError) -> Self {
        match e {
            DFUMemError::File => DFUStatusCode::ErrFile,
            DFUMemError::Target => DFUStatusCode::ErrTarget,
            DFUMemError::Address => DFUStatusCode::ErrAddress,
            DFUMemError::CheckErased => DFUStatusCode::ErrCheckErased,
            DFUMemError::Erase => DFUStatusCode::ErrErase,
            DFUMemError::Write => DFUStatusCode::ErrWrite,
            DFUMemError::Prog => DFUStatusCode::ErrProg,
            DFUMemError::Verify => DFUStatusCode::ErrVerify,
            DFUMemError::Unknown => DFUStatusCode::ErrUnknown,
            DFUMemError::ErrVendor => DFUStatusCode::ErrVendor,
        }
    }
}

impl From<DFUManifestationError> for DFUStatusCode {
    fn from(e: DFUManifestationError) -> Self {
        match e {
            DFUManifestationError::NotDone => DFUStatusCode::ErrNotdone,
            DFUManifestationError::Firmware => DFUStatusCode::ErrFirmware,
            DFUManifestationError::Unknown => DFUStatusCode::ErrUnknown,
            DFUManifestationError::ErrVendor => DFUStatusCode::ErrVendor,
            DFUManifestationError::File => DFUStatusCode::ErrFile,
            DFUManifestationError::Target => DFUStatusCode::ErrTarget,
        }
    }
}

/// DFU protocol USB class implementation for usb-device library.
pub struct DFUClass<B: UsbBus, M: DFUMemIO> {
    if_num: InterfaceNumber,
    status: DFUStatus,
    interface_string: StringIndex,
    _bus: PhantomData<B>,
    mem: M,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Command {
    None,
    EraseAll,
    EraseBlock(u32),
    SetAddressPointer(u32),
    ReadUnprotect,
    WriteMemory { block_num: u16, len: u16 },
    LeaveDFU,
}

#[derive(Clone, Copy)]
struct DFUStatus {
    status: DFUStatusCode,
    poll_timeout: u32,
    state: DFUState,
    address_pointer: u32,
    command: Command,
    pending: Command,
}

impl DFUStatus {
    pub fn new(addr: u32) -> Self {
        Self {
            status: DFUStatusCode::OK,
            poll_timeout: 0,
            state: DFUState::DfuIdle,
            address_pointer: addr,
            command: Command::None,
            pending: Command::None,
        }
    }

    fn new_state_ok(&mut self, state: DFUState) {
        self.new_state_status(state, DFUStatusCode::OK);
    }

    fn new_state_status(&mut self, state: DFUState, status: DFUStatusCode) {
        self.status = status;
        self.state = state;
    }

    fn state(&self) -> DFUState {
        self.state
    }
}

impl From<DFUStatus> for [u8; 6] {
    fn from(dfu: DFUStatus) -> Self {
        [
            // bStatus
            dfu.status as u8,
            // bwPollTimeout
            (dfu.poll_timeout & 0xff) as u8,
            ((dfu.poll_timeout >> 8) & 0xff) as u8,
            ((dfu.poll_timeout >> 16) & 0xff) as u8,
            // bState
            dfu.state as u8,
            // iString: Index of status description in string table.
            0,
        ]
    }
}

impl<B: UsbBus, M: DFUMemIO> UsbClass<B> for DFUClass<B, M> {
    fn get_configuration_descriptors(
        &self,
        writer: &mut DescriptorWriter,
    ) -> usb_device::Result<()> {
        writer.interface_alt(
            self.if_num,
            0,
            USB_CLASS_APPLICATION_SPECIFIC,
            USB_SUBCLASS_DFU,
            USB_PROTOCOL_DFU_MODE,
            Some(self.interface_string),
        )?;

        // DFU Functional descriptor
        writer.write(
            DESC_DESCTYPE_DFU,
            &[
                // bmAttributes
                // Bit 7: bitAcceleratedST
                (if false {0x80} else {0}) |
                    // Bit 4-6: Reserved
                    // Bit 3: bitWillDetach
                    (if true {0x8} else {0}) |
                    // Bit 2: bitManifestationTolerant
                    (if M::MANIFESTATION_TOLERANT {0x4} else {0}) |
                    // Bit 1: bitCanUpload
                    (if M::HAS_UPLOAD {0x2} else {0}) |
                    // Bit 0: bitCanDnload
                    (if M::HAS_DOWNLOAD {0x1} else {0}),
                // wDetachTimeOut
                (M::DETACH_TIMEOUT & 0xff) as u8,
                (M::DETACH_TIMEOUT >> 8) as u8,
                // wTransferSize
                (M::TRANSFER_SIZE & 0xff) as u8,
                (M::TRANSFER_SIZE >> 8) as u8,
                // bcdDFUVersion
                0x1a,
                0x01,
            ],
        )?;

        //

        Ok(())
    }

    fn get_string(&self, index: StringIndex, lang_id: u16) -> Option<&str> {
        if lang_id == usb_device::descriptor::lang_id::ENGLISH_US || lang_id == 0 {
            if index == self.interface_string {
                return Some(M::MEM_INFO_STRING);
            }
        }
        None
    }

    // Handle control requests to the host.
    fn control_in(&mut self, xfer: ControlIn<B>) {
        let req = *xfer.request();

        if req.request_type != control::RequestType::Class {
            return;
        }

        if req.recipient != control::Recipient::Interface {
            return;
        }

        if req.index != u8::from(self.if_num) as u16 {
            return;
        }

        match req.request {
            DFU_UPLOAD => {
                self.upload(xfer, req);
            }
            DFU_GETSTATUS => {
                self.get_status(xfer, req);
            }
            DFU_GETSTATE => {
                self.get_state(xfer, req);
            }
            _ => {
                xfer.reject().ok();
            }
        }
    }

    // Handle a control request from the host.
    fn control_out(&mut self, xfer: ControlOut<B>) {
        let req = *xfer.request();

        if req.request_type != control::RequestType::Class {
            return;
        }

        if req.recipient != control::Recipient::Interface {
            return;
        }

        if req.index != u8::from(self.if_num) as u16 {
            return;
        }

        match req.request {
            //DFU_DETACH => {},
            DFU_DNLOAD => {
                self.download(xfer, req);
            }
            DFU_CLRSTATUS => {
                self.clear_status(xfer);
            }
            DFU_ABORT => {
                self.abort(xfer);
            }
            _ => {
                xfer.reject().ok();
            }
        }
    }

    fn reset(&mut self) {
        // may not return
        self.mem.usb_reset();

        // Try to signal possible error to a host.
        // Not exactly clear what status should be.
        match self.status.state() {
            DFUState::DfuUploadIdle
            | DFUState::DfuDnloadIdle
            | DFUState::DfuDnloadSync
            | DFUState::DfuDnBusy
            | DFUState::DfuError
            | DFUState::DfuManifest
            | DFUState::DfuManifestSync => {
                self.status
                    .new_state_status(DFUState::DfuError, DFUStatusCode::ErrUsbr);
            }
            DFUState::DfuIdle
            | DFUState::AppDetach
            | DFUState::AppIdle
            | DFUState::DfuManifestWaitReset => {}
        }
    }

    fn poll(&mut self) {
        self.update_impl();
    }
}

impl<B: UsbBus, M: DFUMemIO> DFUClass<B, M> {
    /// Creates a new DFUClass with the provided UsbBus and
    /// DFUMemIO
    pub fn new<'a>(alloc: &'a UsbBusAllocator<B>, mem: M) -> Self {
        Self {
            if_num: alloc.interface(),
            status: DFUStatus::new(M::INITIAL_ADDRESS_POINTER),
            interface_string: alloc.string(),
            _bus: PhantomData,
            mem,
        }
    }

    /// This function may be called just after `DFUClass::new()` to
    /// set DFU error state to "Device detected unexpected power on reset"
    /// instead of the usual `dfuIdle`.
    pub fn set_unexpected_reset_state(&mut self) {
        self.status
            .new_state_status(DFUState::DfuError, DFUStatusCode::ErrPOR);
    }

    /// This function may be called just after `DFUClass::new()` to
    /// set DFU error state to "Device’s firmware is corrupt. It cannot return to run-time (non-DFU) operations"
    /// instead of the usual `dfuIdle`.
    pub fn set_firmware_corrupted_state(&mut self) {
        self.status
            .new_state_status(DFUState::DfuError, DFUStatusCode::ErrFirmware);
    }

    /// Return current Address Pointer value.
    pub fn get_address_pointer(&self) -> u32 {
        self.status.address_pointer
    }

    fn clear_status(&mut self, xfer: ControlOut<B>) {
        match self.status.state() {
            DFUState::DfuError => {
                self.status.command = Command::None;
                self.status.pending = Command::None;
                self.status.new_state_ok(DFUState::DfuIdle);
                xfer.accept().ok();
            }
            _ => {
                self.status
                    .new_state_status(DFUState::DfuError, DFUStatusCode::ErrStalledPkt);
                xfer.reject().ok();
            }
        }
    }

    fn abort(&mut self, xfer: ControlOut<B>) {
        match self.status.state() {
            DFUState::DfuIdle
            | DFUState::DfuUploadIdle
            | DFUState::DfuDnloadIdle
            | DFUState::DfuDnloadSync
            | DFUState::DfuManifestSync => {
                self.status.command = Command::None;
                self.status.pending = Command::None;
                self.status.new_state_ok(DFUState::DfuIdle);
                xfer.accept().ok();
            }
            DFUState::AppDetach
            | DFUState::AppIdle
            | DFUState::DfuDnBusy
            | DFUState::DfuManifest
            | DFUState::DfuManifestWaitReset
            | DFUState::DfuError => {
                xfer.reject().ok();
            }
        }
    }

    fn download(&mut self, xfer: ControlOut<B>, req: Request) {
        let initial_state = self.status.state();

        if initial_state != DFUState::DfuIdle && initial_state != DFUState::DfuDnloadIdle {
            self.status
                .new_state_status(DFUState::DfuError, DFUStatusCode::ErrStalledPkt);
            xfer.reject().ok();
            return;
        }

        if req.length == 0 {
            self.status.command = Command::LeaveDFU;
            self.status.new_state_ok(DFUState::DfuManifestSync);
            xfer.accept().ok();
            return;
        }

        if req.value > 1 {
            let data = xfer.data();
            if !data.is_empty() {
                // store the whole buffer, chunked operation in not supported
                match self.mem.store_write_buffer(data) {
                    Err(_) => {
                        self.status
                            .new_state_status(DFUState::DfuError, DFUStatusCode::ErrStalledPkt);
                        xfer.reject().ok();
                    }
                    Ok(_) => {
                        let block_num = req.value - 2;
                        self.status.command = Command::WriteMemory {
                            block_num,
                            len: data.len() as u16,
                        };
                        self.status.new_state_ok(DFUState::DfuDnloadSync);
                        xfer.accept().ok();
                    }
                }
                return;
            }
        } else if req.value == 0 {
            let data = xfer.data();
            if req.length >= 1 {
                let command = data[0];

                if command == DnloadCommand::SetAddressPointer as u8 {
                    if req.length == 5 {
                        let addr = (data[1] as u32)
                            | ((data[2] as u32) << 8)
                            | ((data[3] as u32) << 16)
                            | ((data[4] as u32) << 24);
                        self.status.command = Command::SetAddressPointer(addr);
                        self.status.new_state_ok(DFUState::DfuDnloadSync);
                        xfer.accept().ok();
                        return;
                    }
                } else if command == DnloadCommand::Erase as u8 {
                    if req.length == 5 {
                        let addr = (data[1] as u32)
                            | ((data[2] as u32) << 8)
                            | ((data[3] as u32) << 16)
                            | ((data[4] as u32) << 24);
                        self.status.command = Command::EraseBlock(addr);
                        self.status.new_state_ok(DFUState::DfuDnloadSync);
                        xfer.accept().ok();
                        return;
                    } else if req.length == 1 {
                        self.status.command = Command::EraseAll;
                        self.status.new_state_ok(DFUState::DfuDnloadSync);
                        xfer.accept().ok();
                        return;
                    }
                } else if HAS_READ_UNPROTECT && command == DnloadCommand::ReadUnprotect as u8 {
                    self.status.command = Command::ReadUnprotect;
                    self.status.new_state_ok(DFUState::DfuDnloadSync);
                    xfer.accept().ok();
                    return;
                }
            }
        }

        self.status
            .new_state_status(DFUState::DfuError, DFUStatusCode::ErrStalledPkt);
        xfer.reject().ok();
    }

    fn upload(&mut self, xfer: ControlIn<B>, req: Request) {
        let initial_state = self.status.state();

        if initial_state != DFUState::DfuIdle && initial_state != DFUState::DfuUploadIdle {
            self.status
                .new_state_status(DFUState::DfuError, DFUStatusCode::ErrStalledPkt);
            xfer.reject().ok();
            return;
        }

        if req.value == 0 {
            // Get command
            let commands = [
                DnloadCommand::GetCommands as u8,
                DnloadCommand::SetAddressPointer as u8,
                DnloadCommand::Erase as u8,
                // XXX read unprotect
            ];

            if req.length as usize >= commands.len() {
                self.status.new_state_ok(DFUState::DfuIdle);
                xfer.accept_with(&commands).ok();
                return;
            }
        } else if req.value > 1 {
            // upload command
            let block_num = req.value - 2;
            let transfer_size = min(M::TRANSFER_SIZE, req.length);

            if let Some(address) = self
                .status
                .address_pointer
                .checked_add((block_num as u32) * (transfer_size as u32))
            {
                match self.mem.read_block(address, transfer_size as usize) {
                    Ok(b) => {
                        if b.len() < M::TRANSFER_SIZE as usize {
                            // short frame, back to idle
                            self.status.new_state_ok(DFUState::DfuIdle);
                        } else {
                            self.status.new_state_ok(DFUState::DfuUploadIdle);
                        }
                        xfer.accept_with(&b).ok();
                        return;
                    }
                    Err(e) => {
                        self.status.new_state_status(DFUState::DfuError, e.into());
                        xfer.reject().ok();
                        return;
                    }
                }
            } else {
                // overflow
                self.status
                    .new_state_status(DFUState::DfuError, DFUStatusCode::ErrAddress);
                xfer.reject().ok();
                return;
            }
        }

        self.status
            .new_state_status(DFUState::DfuError, DFUStatusCode::ErrStalledPkt);
        xfer.reject().ok();
    }

    fn get_state(&mut self, xfer: ControlIn<B>, req: Request) {
        // return current state, without any state transition
        if req.length > 0 {
            let v = self.status.state() as u8;
            xfer.accept_with(&[v]).ok();
        } else {
            self.status
                .new_state_status(DFUState::DfuError, DFUStatusCode::ErrStalledPkt);
            xfer.reject().ok();
        }
    }

    fn get_status(&mut self, xfer: ControlIn<B>, req: Request) {
        if req.length >= 6 && self.process() {
            self.status.poll_timeout = self.expected_timeout();
            let v: [u8; 6] = self.status.into();
            xfer.accept_with(&v).ok();
            return;
        }

        self.status
            .new_state_status(DFUState::DfuError, DFUStatusCode::ErrStalledPkt);
        xfer.reject().ok();
    }

    fn expected_timeout(&self) -> u32 {
        match self.status.pending {
            Command::WriteMemory {
                block_num: _,
                len: _,
            } => M::PAGE_PROGRAM_TIME_MS,
            Command::EraseAll => M::FULL_ERASE_TIME_MS,
            Command::EraseBlock(_) => M::PAGE_ERASE_TIME_MS,
            Command::LeaveDFU => M::MANIFESTATION_TIME_MS,
            _ => 0,
        }
    }

    // ///
    // /// Handle some DFU state transitions, and call `DFUMemIO`'s erase, program,
    // /// and manifestation functions.
    // ///
    // /// This function will be called internally by if [`M::MEMIO_IN_USB_INTERRUPT`](DFUMemIO::MEMIO_IN_USB_INTERRUPT)
    // /// is `true` (default) as one of a final steps of `usb_dev.poll([...])` which is itself usually called
    // /// from USB interrupt.
    // ///
    // /// This function must be called if [`M::MEMIO_IN_USB_INTERRUPT`](DFUMemIO::MEMIO_IN_USB_INTERRUPT) is `false`
    // /// and erase, program, and manifestation should be called from a different context than `usb_dev.poll([...])`.
    // ///
    // pub fn update(&mut self) {
    //     debug_assert!(!M::MEMIO_IN_USB_INTERRUPT, "not requried with MEMIO_IN_USB_INTERRUPT");
    //     if !M::MEMIO_IN_USB_INTERRUPT {
    //         self.update_impl()
    //     }
    // }

    // /// Returns `true` if [`update()`](DFUClass::update) needs to be called to
    // /// process a pending operation.
    // pub fn update_pending(&self) -> bool {
    //     match self.status.pending {
    //         Command::None => false,
    //         _ => true,
    //     }
    // }

    fn update_impl(&mut self) {
        match self.status.pending {
            Command::EraseAll => match self.mem.erase_all_blocks() {
                Err(e) => self.status.new_state_status(DFUState::DfuError, e.into()),
                Ok(_) => self.status.new_state_ok(DFUState::DfuDnloadSync),
            },
            Command::EraseBlock(b) => match self.mem.erase_block(b) {
                Err(e) => self.status.new_state_status(DFUState::DfuError, e.into()),
                Ok(_) => self.status.new_state_ok(DFUState::DfuDnloadSync),
            },
            Command::LeaveDFU => {
                // may not return
                let mr = self.mem.manifestation();

                match mr {
                    Err(e) => self.status.new_state_status(DFUState::DfuError, e.into()),
                    Ok(_) => {
                        if M::MANIFESTATION_TOLERANT {
                            self.status.new_state_ok(DFUState::DfuManifestSync)
                        } else {
                            self.status.new_state_ok(DFUState::DfuManifestWaitReset)
                        }
                    }
                }
            }
            Command::ReadUnprotect => {
                // XXX not implemented
                // self.status.state = DFUState::DfuDnloadSync;
                self.status
                    .new_state_status(DFUState::DfuError, DFUStatusCode::ErrStalledPkt)
            }
            Command::WriteMemory { block_num, len } => {
                if let Some(pointer) = self
                    .status
                    .address_pointer
                    .checked_add((block_num as u32) * (len as u32))
                {
                    match self.mem.program_block(pointer, len as usize) {
                        Err(e) => self.status.new_state_status(DFUState::DfuError, e.into()),
                        Ok(_) => self.status.new_state_ok(DFUState::DfuDnloadSync),
                    }
                } else {
                    // overflow
                    self.status
                        .new_state_status(DFUState::DfuError, DFUStatusCode::ErrAddress);
                }
            }
            Command::SetAddressPointer(p) => {
                self.status.address_pointer = p;
                self.status.new_state_ok(DFUState::DfuDnloadSync)
            }
            Command::None => {}
        }
        self.status.pending = Command::None;
    }

    fn process(&mut self) -> bool {
        let initial_state = self.status.state();
        if initial_state == DFUState::DfuDnloadSync {
            match self.status.command {
                Command::WriteMemory {
                    block_num: _,
                    len: _,
                }
                | Command::SetAddressPointer(_)
                | Command::ReadUnprotect
                | Command::EraseAll
                | Command::EraseBlock(_) => {
                    self.status.pending = self.status.command;
                    self.status.command = Command::None;
                    self.status.new_state_ok(DFUState::DfuDnBusy);
                }
                //Command::None => {}
                _ => {
                    self.status.new_state_ok(DFUState::DfuDnloadIdle);
                }
            }
        } else if initial_state == DFUState::DfuManifestSync {
            match self.status.command {
                Command::None => {
                    if M::MANIFESTATION_TOLERANT {
                        // Leave manifestation, back to Idle
                        self.status.command = Command::None;
                        self.status.new_state_ok(DFUState::DfuIdle);
                    }
                }
                _ => {
                    // Start manifestation
                    self.status.pending = self.status.command;
                    self.status.command = Command::None;
                    self.status.new_state_ok(DFUState::DfuManifest);
                }
            }
        } else if initial_state == DFUState::DfuDnBusy {
            return false;
        }

        true
    }
}
