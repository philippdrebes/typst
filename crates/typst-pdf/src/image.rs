use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use ecow::eco_format;
use image::{DynamicImage, EncodableLayout, GenericImageView, Rgba};
use jxl_oxide::{AllocTracker, JpegReconstructionStatus, JxlImage};
use krilla::image::{BitsPerComponent, CustomImage, ImageColorspace};
use krilla::pdf::PdfDocument;
use krilla::surface::Surface;
use krilla_svg::{SurfaceExt, SvgSettings};
use typst_library::diag::{At, SourceResult};
use typst_library::foundations::{Bytes, Smart};
use typst_library::layout::{Abs, Angle, Ratio, Size, Transform};
use typst_library::visualize::{
    ExchangeFormat, Image, ImageKind, ImageScaling, PdfImage, RasterFormat, RasterImage,
};
use typst_syntax::Span;
use typst_utils::defer;

use crate::convert::{FrameContext, GlobalContext};
use crate::tags;
use crate::util::{SizeExt, TransformExt};

#[typst_macros::time(name = "handle image")]
pub(crate) fn handle_image(
    gc: &mut GlobalContext,
    fc: &mut FrameContext,
    image: &Image,
    size: Size,
    surface: &mut Surface,
    span: Span,
) -> SourceResult<()> {
    surface.push_transform(&fc.state().transform().to_krilla());
    surface.set_location(span.into_raw());
    let mut surface = defer(surface, |s| {
        s.pop();
        s.reset_location();
    });

    let interpolate = image.scaling() == Smart::Custom(ImageScaling::Smooth);

    gc.image_spans.insert(span);

    let mut handle = tags::image(gc, fc, &mut surface, image, size);
    let surface = handle.surface();

    match image.kind() {
        ImageKind::Raster(raster) => {
            let (image, rotation) = convert_raster(raster.clone(), interpolate)
                .map_err(|err| eco_format!("failed to process image ({err})"))
                .at(span)?;

            let (exif_transform, new_size) = exif_transform(rotation, size);
            surface.push_transform(&exif_transform.to_krilla());
            let mut surface = defer(surface, |s| s.pop());

            if !gc.image_to_spans.contains_key(&image) {
                gc.image_to_spans.insert(image.clone(), span);
            }

            if let Some(size) = new_size.to_krilla() {
                surface.draw_image(image, size);
            }
        }
        ImageKind::Svg(svg) => {
            if let Some(size) = size.to_krilla() {
                surface.draw_svg(
                    svg.tree(),
                    size,
                    SvgSettings { embed_text: true, ..Default::default() },
                );
            }
        }
        ImageKind::Pdf(pdf) => {
            if let Some(size) = size.to_krilla() {
                surface.draw_pdf_page(&convert_pdf(pdf), size, pdf.page_index());
            }
        }
    }

    Ok(())
}

/// A wrapper around `RasterImage` so that we can implement `CustomImage`.
#[derive(Clone)]
struct PdfRasterImage(Arc<PdfRasterImageInner>);

/// The internal representation of a [`PdfRasterImage`].
struct PdfRasterImageInner {
    /// The original, underlying raster image.
    raster: RasterImage,
    /// The alpha channel of the raster image, if existing.
    alpha_channel: OnceLock<Option<Vec<u8>>>,
    /// A (potentially) converted version of the dynamic image stored `raster` that is
    /// guaranteed to either be in luma8 or rgb8, and thus can be used for the
    /// `color_channel` method of `CustomImage`.
    actual_dynamic: OnceLock<Arc<DynamicImage>>,
}

impl PdfRasterImage {
    /// Wraps a raster image.
    pub fn new(raster: RasterImage) -> Self {
        Self(Arc::new(PdfRasterImageInner {
            raster,
            alpha_channel: OnceLock::new(),
            actual_dynamic: OnceLock::new(),
        }))
    }
}

impl Hash for PdfRasterImage {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // `alpha_channel` and `actual_dynamic` are generated from the underlying `RasterImage`,
        // so this is enough. Since `raster` is prehashed, this is also very cheap.
        self.0.raster.hash(state);
    }
}

impl CustomImage for PdfRasterImage {
    fn color_channel(&self) -> &[u8] {
        self.0
            .actual_dynamic
            .get_or_init(|| {
                let dynamic = self.0.raster.dynamic();
                let channel_count = dynamic.color().channel_count();

                match (dynamic.as_ref(), channel_count) {
                    // Pure luma8 or rgb8 image, can use it directly.
                    (DynamicImage::ImageLuma8(_), _) => dynamic.clone(),
                    (DynamicImage::ImageRgb8(_), _) => dynamic.clone(),
                    // Grey-scale image, convert to luma8.
                    (_, 1 | 2) => Arc::new(DynamicImage::ImageLuma8(dynamic.to_luma8())),
                    // Anything else, convert to rgb8.
                    _ => Arc::new(DynamicImage::ImageRgb8(dynamic.to_rgb8())),
                }
            })
            .as_bytes()
    }

    fn alpha_channel(&self) -> Option<&[u8]> {
        self.0
            .alpha_channel
            .get_or_init(|| {
                self.0.raster.dynamic().color().has_alpha().then(|| {
                    self.0
                        .raster
                        .dynamic()
                        .pixels()
                        .map(|(_, _, Rgba([_, _, _, a]))| a)
                        .collect()
                })
            })
            .as_ref()
            .map(|v| &**v)
    }

    fn bits_per_component(&self) -> BitsPerComponent {
        BitsPerComponent::Eight
    }

    fn size(&self) -> (u32, u32) {
        (self.0.raster.width(), self.0.raster.height())
    }

    fn icc_profile(&self) -> Option<&[u8]> {
        if matches!(
            self.0.raster.dynamic().as_ref(),
            DynamicImage::ImageLuma8(_)
                | DynamicImage::ImageLumaA8(_)
                | DynamicImage::ImageRgb8(_)
                | DynamicImage::ImageRgba8(_)
        ) {
            self.0.raster.icc().map(|b| b.as_bytes())
        } else {
            // In all other cases, the dynamic will be converted into RGB8 or LUMA8, so the ICC
            // profile may become invalid, and thus we don't include it.
            None
        }
    }

    fn color_space(&self) -> ImageColorspace {
        // Remember that we convert all images to either RGB or luma.
        if self.0.raster.dynamic().color().has_color() {
            ImageColorspace::Rgb
        } else {
            ImageColorspace::Luma
        }
    }
}

/// Converts a raster image into a krilla image.
///
/// Also returns the EXIF orientation that must still be applied to the
/// converted image as a transform. This is only necessary for JPEGs, which we
/// embed as-is.
#[comemo::memoize]
fn convert_raster(
    raster: RasterImage,
    interpolate: bool,
) -> Result<(krilla::image::Image, Option<u32>), String> {
    let jpeg = match raster.format() {
        RasterFormat::Exchange(ExchangeFormat::Jpg) => {
            Some((raster.data().clone(), raster.exif_rotation()))
        }
        // A JPEG XL file may contain a losslessly recompressed JPEG. If so,
        // we reconstruct and embed the original JPEG instead of re-encoding
        // the decoded pixels, which would be much larger.
        RasterFormat::Exchange(ExchangeFormat::Jxl) => reconstruct_jpeg(raster.data()),
        _ => None,
    };

    if let Some((data, rotation)) = jpeg {
        let image_data: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(data);
        let icc_profile = raster.icc().map(|i| {
            let i: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(i.clone());
            i
        });

        let image = krilla::image::Image::from_jpeg_with_icc(
            image_data.into(),
            icc_profile.map(|i| i.into()),
            interpolate,
        )?;
        Ok((image, rotation))
    } else {
        let image =
            krilla::image::Image::from_custom(PdfRasterImage::new(raster), interpolate)?;
        Ok((image, None))
    }
}

/// Tries to reconstruct the original JPEG from a JPEG XL image and returns it
/// together with the orientation from the JPEG XL image header.
fn reconstruct_jpeg(data: &[u8]) -> Option<(Bytes, Option<u32>)> {
    let max_alloc = image::Limits::default().max_alloc;
    let tracker = AllocTracker::with_limit(
        max_alloc
            .and_then(|max| usize::try_from(max).ok())
            .unwrap_or(usize::MAX),
    );
    let image = JxlImage::builder().alloc_tracker(tracker).read(data).ok()?;
    if image.jpeg_reconstruction_status() != JpegReconstructionStatus::Available {
        return None;
    }

    let mut jpeg = vec![];
    image.reconstruct_jpeg(&mut jpeg).ok()?;
    let orientation = image.image_header().metadata.orientation;
    Some((Bytes::new(jpeg), Some(orientation)))
}

#[comemo::memoize]
fn convert_pdf(pdf: &PdfImage) -> PdfDocument {
    PdfDocument::new(pdf.document().pdf().clone())
}

/// Computes the transform that applies an EXIF orientation to an image that
/// is embedded as-is (i.e. a JPEG).
///
/// For other formats, the orientation is already baked into the decoded image
/// data, so `rotation` is `None` for them.
fn exif_transform(rotation: Option<u32>, size: Size) -> (Transform, Size) {
    let base = |hp: bool, vp: bool, mut base_ts: Transform, size: Size| {
        if hp {
            // Flip horizontally in-place.
            base_ts = base_ts.pre_concat(
                Transform::scale(-Ratio::one(), Ratio::one())
                    .pre_concat(Transform::translate(-size.x, Abs::zero())),
            );
        }

        if vp {
            // Flip vertically in-place.
            base_ts = base_ts.pre_concat(
                Transform::scale(Ratio::one(), -Ratio::one())
                    .pre_concat(Transform::translate(Abs::zero(), -size.y)),
            );
        }

        base_ts
    };

    let no_flipping =
        |hp: bool, vp: bool| (base(hp, vp, Transform::identity(), size), size);

    let with_flipping = |hp: bool, vp: bool| {
        let base_ts = Transform::rotate_at(Angle::deg(90.0), Abs::zero(), Abs::zero())
            .pre_concat(Transform::scale(Ratio::one(), -Ratio::one()));
        let inv_size = Size::new(size.y, size.x);
        (base(hp, vp, base_ts, inv_size), inv_size)
    };

    match rotation {
        Some(2) => no_flipping(true, false),
        Some(3) => no_flipping(true, true),
        Some(4) => no_flipping(false, true),
        Some(5) => with_flipping(false, false),
        Some(6) => with_flipping(false, true),
        Some(7) => with_flipping(true, true),
        Some(8) => with_flipping(true, false),
        _ => no_flipping(false, false),
    }
}

#[cfg(test)]
mod tests {
    use typst_library::visualize::ExchangeFormat;

    use super::*;

    /// Exports a single image into a PDF and returns the PDF's bytes.
    fn export(data: &[u8], format: ExchangeFormat) -> Vec<u8> {
        let raster = RasterImage::plain(Bytes::new(data.to_vec()), format).unwrap();
        let (image, _) = convert_raster(raster, false).unwrap();
        let mut document = krilla::Document::new();
        let mut page = document.start_page();
        let mut surface = page.surface();
        surface.draw_image(image, krilla::geom::Size::from_wh(64.0, 48.0).unwrap());
        surface.finish();
        page.finish();
        document.finish().unwrap()
    }

    #[test]
    fn test_jxl_jpeg_reconstruction() {
        // TODO: Load these from `typst-dev-assets` once they are available
        // there.
        let jxl = include_bytes!("../../../tests/jxl-assets/recompressed.jxl");
        let jpg = include_bytes!("../../../tests/jxl-assets/recompressed.jpg");
        assert_eq!(reconstruct_jpeg(jxl).unwrap().0.as_slice(), jpg);

        // A recompressed JPEG is embedded as the original JPEG, so the PDF
        // has (almost) the same size as when using that JPEG directly.
        let from_jxl = export(jxl, ExchangeFormat::Jxl).len() as f64;
        let from_jpg = export(jpg, ExchangeFormat::Jpg).len() as f64;
        assert!(
            (from_jxl - from_jpg).abs() <= 0.05 * from_jpg,
            "PDF size with JPEG XL ({from_jxl} bytes) is not within 5% of the \
             size with JPEG ({from_jpg} bytes)",
        );
    }
}
