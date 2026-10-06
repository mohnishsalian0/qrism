# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-10-06

### Added
- `Metadata` is now exported, with `version()`, `ec_level()` and `mask()` getters
- `QRError` and `QRResult` are now exported for matching on decode errors
- `Symbol::outline()` returns the four corners of a detected QR in image coordinates

### Changed
- Faster binarization using bit-packed, block-by-block processing
- Finder pattern detection rewritten around contour tracing, with diagonal cross-checks and module-size gating when grouping finders
- Sub-pixel precision for finder and alignment pattern centres
- Improved alignment pattern search and symbol location, especially for larger QR versions
- **Breaking:** `Symbol` is now exported as `qrism::Symbol` (previously `qrism::reader::symbol::Symbol`)

### Removed
- **Breaking:** `qrism::reader::binarize` is no longer public (`BinaryImage`, `Pixel`, `Binarize`)
- **Breaking:** `Symbol` internals are no longer public (`new`, `get`, `ver`, `map`, `raw_map`, `highlight`, `read_version_info`, `read_format_info`, `read_capacity_info`, `get_number`, `extract_payload`), along with `SymbolLocation` and `measure_timing_patterns`
- The crate no longer ships a `qrism` binary; see `examples/` for usage

## [0.1.0] - 2025-09-25

### Added
- Initial release of qrism QR code library
- QR code generation with customizable versions, error correction levels, and capacity
- QR code reading and detection from images with robust error correction
- Reed-Solomon error correction with configurable levels (L, M, Q, H)
- Experimental high capacity QR support with 3x storage capacity using RGB color channels
- Advanced image processing with binarization and geometric correction
- Support for traditional monochromatic QR codes (versions 1-40)
- Backward compatibility for reading standard black-and-white QR codes
- Comprehensive examples demonstrating basic and advanced usage
- Full test suite with 128+ unit tests
- Documentation with examples and API reference

### Features
- `QRBuilder` for flexible QR code generation
- `detect_qr()` function for standard QR code detection
- `detect_hc_qr()` function for high capacity multicolor QR detection
- Automatic version selection based on data size
- Configurable mask patterns with automatic optimization
- Support for Numeric, Alphanumeric, Byte, and Kanji encoding modes

[0.2.0]: https://github.com/mohnishsalian0/qrism/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/mohnishsalian0/qrism/releases/tag/v0.1.0
