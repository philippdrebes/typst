#![no_main]

use libfuzzer_sys::fuzz_target;
use typst::foundations::Bytes;
use typst::visualize::{ExchangeFormat, RasterImage};

fuzz_target!(|data: &[u8]| {
    let bytes = Bytes::new(data.to_vec());
    if RasterImage::plain(bytes, ExchangeFormat::Jxl).is_ok() {
        // Also exercise JPEG reconstruction, which PDF export uses for
        // recompressed JPEGs.
        if let Ok(image) = jxl_oxide::JxlImage::builder().read(data) {
            _ = std::hint::black_box(image.reconstruct_jpeg(std::io::sink()));
        }
    }
    comemo::evict(0);
});
