TODO:
Address the failing testcase in codec
Optimize functions for grayscale images by only considering 1 value instead of 3 channels. Following functions need an update
    Timing scan func
    Extract payload
Remove all reserving function from builder
Optimize the tolerance for qr fitness. Right now it is 50%

OUT OF SCOPE:
Detecting branded (custom colors) traditional QR
Detecting unconventional/hard to perceive QR
Detecting multiple QRs
