#![no_main]

use libfuzzer_sys::fuzz_target;
use typst::foundations::Bytes;
use typst::visualize::{ExchangeFormat, RasterImage};

fuzz_target!(|data: &[u8]| {
    let bytes = Bytes::new(data.to_vec());
    _ = std::hint::black_box(RasterImage::plain(bytes, ExchangeFormat::Jxl));
    comemo::evict(0);
});
