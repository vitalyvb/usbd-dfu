# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0] - yyyy-mm-dd

### Changed
- `usb-device` dependency updated to 0.3.1
- Created CREDITS.md
- Updated README.md and copyright notice in LICENSE file
- Bump tests dependency stm32f1xx-hal to 0.10.0

## [0.3.1] - 2023-05-06

### Added
- New `DFUClass::release()` to consume class and release owned memory argument.
([#9](https://github.com/vitalyvb/usbd-dfu/pull/9))

## [0.3.0] - 2023-03-18

### Breaking Changes
- Behavior change: use `DFUMemIO::TRANSFER_SIZE` instead of a request length
for address compuration in Download and Upload commands. If Host programmer
is using smaller block sizes, uploads and downloads will be corrupted.

### Fixed
- Fixed programming error because of incorrect memory address calculation
when writing the last and short block of the firmware without SetAddressPointer
command ([#6](https://github.com/vitalyvb/usbd-dfu/pull/6))

### Changed
- Rust edition set to 2021
- `usb-device` dependency updated to 0.2.9
- Documentation of `DFUMemIO::TRANSFER_SIZE`

## [0.2.0] - 2021-08-08

### Breaking Changes
- Rename parts of the `DFUMemIO` API to remove confusing block/page terminology. ([#4](https://github.com/vitalyvb/usbd-dfu/pull/4)):
   - `DFUMemIO::PAGE_PROGRAM_TIME_MS` to `DFUMemIO::PROGRAM_TIME_MS`
   - `DFUMemIO::PAGE_ERASE_TIME_MS` to `DFUMemIO::ERASE_TIME_MS`
   - `DFUMemIO::read_block()` to `DFUMemIO::read()`
   - `DFUMemIO::erase_block()` to `DFUMemIO::erase()`
   - `DFUMemIO::erase_all_blocks()` to `DFUMemIO::erase_all()`
   - `DFUMemIO::program_block()` to `DFUMemIO::program()`

- Rename `Command::EraseBlock` to `Command::Erase`. ([#4](https://github.com/vitalyvb/usbd-dfu/pull/4))

### Fixed
- Some Clippy warnings

## [0.1.1] - 2021-05-15
### Added
- CI using GitHub Actions

### Fixed
- `DFUManifestationError::File` error status incorrectly returned `errTarget` to host
- `DFUManifestationError::Target` error status incorrectly returned `errFile` to host
- Clippy errors and some warnings

### Changed
- Code formatting to follow rustfmt
- Clarified the behavior of `DFUMemIO::usb_reset` in the documentation
- Documentation updates

## [0.1.0] - 2021-04-16

First version.

[Unreleased]: https://github.com/vitalyvb/usbd-dfu/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/vitalyvb/usbd-dfu/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/vitalyvb/usbd-dfu/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/vitalyvb/usbd-dfu/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/vitalyvb/usbd-dfu/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/vitalyvb/usbd-dfu/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/vitalyvb/usbd-dfu/releases/tag/v0.1.0
