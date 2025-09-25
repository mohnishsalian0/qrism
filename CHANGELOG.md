# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

[0.1.0]: https://github.com/mohnishsalian0/qrism/releases/tag/v0.1.0