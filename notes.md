TODO:
Address the failing testcase in codec
Optimize functions for grayscale images by only considering 1 value instead of 3 channels. Following functions need an update
    Timing scan func
    Extract payload
Optimize the tolerance for qr fitness. Right now it is 50%
Investigate the distorted qr detected in lots image
Identify & investigate the 2 images which fail when top and bottom ring pt checks are added
Identify & investigate the 4 images which fail when alignment search radius is increased to 20

OUT OF SCOPE:
Detecting branded (custom colors) traditional QR
Detecting unconventional/hard to perceive QR
Detecting multiple QRs
