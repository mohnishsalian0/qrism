TODO:
Binarize image ✅
Find finder ✅
    Scan line ✅
    Validate finder with flood fill ✅
Group finders ✅
Compute perspective and boundaries ✅
Measure timing pattern ✅
Locate alignment pattern ✅
Test fitness ✅
Update reader to use perspective and get modules
Utils
    Bresenham's line ✅
    Flood fill ✅
    Homography ✅
Inject palette into timing

OUT OF SCOPE:
Detecting branded (custom colors) traditional QR
Detecting unconventional/hard to perceive QR
Detecting multiple QRs

Image debug code:
use std::path::Path;
println!("R {r}, Y {y}");
let _ = img.save(Path::new("assets/ring.png"));
panic!("Exiting early");
