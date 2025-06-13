TODO:
Address the failing testcase in codec
✅ Optimize functions for grayscale images by only considering 1 value instead of 3 channels. Following functions need an update
    Timing scan func
    Extract payload
    Verdict: This optimization doesn't improve the performance by any perceivable margin. So rejecting it
Optimize the tolerance for qr fitness. Right now it is 50%
✅ Investigate the distorted qr detected in lots image
    Larger timing pattern was wrong. Switched from taking max to average of the 2 timing patterns. Added 32 successful decoding
    The alignment threshold of 0.95 was too relaxed. It is viable for finder, because finders go through multiple check. Divided the alignment
    pattern check into 2 stages. First stage checks if the average run length is roughly equal to estimate mod size with 50% tolerance.
    Second stage check if all run lengths are equal to average with 80% tolerance
✅ Identify & investigate the 2 images which fail when top and bottom ring pt checks are added
    In bright_spots 2nd image, the ring doesn't completely encapsulate the stone
    In lots 1st image, a black spec/noise causes failure
✅ Identify & investigate the 4 images which fail when alignment search radius is increased to 20
    The alignment pattern from neighboring symbol was being used. Changed the radius back to 15, since the radius is too big anyway
Noise filter
Use alignment patterns in higher version
