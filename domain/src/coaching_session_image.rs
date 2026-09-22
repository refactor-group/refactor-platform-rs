//! Coaching note images: validate uploaded bytes, then store the object before recording it.
//!
//! Validation lives here because this is the security boundary — the declared content type
//! from the request never reaches this module, only the bytes themselves.

use entity_api::coaching_session_image::NewCoachingSessionImage;
use sea_orm::DatabaseConnection;

use crate::coaching_session_images::Model;
use crate::error::Error;
use crate::gateway::object_storage::ObjectStore;
use crate::Id;

pub use entity_api::coaching_session_image::find_by_id;

/// Image types we will accept and later serve from our own origin. `image/svg+xml` is
/// absent deliberately: SVG is XML that can carry script, so serving it back would be
/// stored XSS against a signed-in coach.
const ALLOWED_MIME_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];

/// Pixel ceiling. A decompression bomb is small on the wire but ruinous once decoded,
/// and no legitimate pasted screenshot comes close.
const MAX_PIXELS: u64 = 50_000_000;

/// Why uploaded bytes were refused. A value, not an error variant: the web layer
/// maps it to 413/415, which `DomainErrorKind` has no representation for.
#[derive(Debug, PartialEq, Eq)]
pub enum ImageRejection {
    TooLarge,
    UnsupportedType,
}

/// The outcome of inspecting uploaded bytes.
pub struct InspectedImage {
    pub mime_type: String,
    pub extension: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

/// Inspect raw uploaded bytes: sniff the real type, enforce the allowlist and the
/// size cap, and read dimensions without decoding the full image.
///
/// # Errors
///
/// Returns `ImageRejection::TooLarge` when the bytes exceed `max_bytes` or the header's
/// pixel count exceeds the bomb ceiling, and `ImageRejection::UnsupportedType` when the
/// sniffed type is absent or outside the allowlist. Undeterminable dimensions are not an
/// error — both come back `None`.
pub fn inspect_image(bytes: &[u8], max_bytes: u64) -> Result<InspectedImage, ImageRejection> {
    if bytes.len() as u64 > max_bytes {
        return Err(ImageRejection::TooLarge);
    }

    // Magic bytes only. What the request claimed the file was is never consulted.
    let kind = infer::get(bytes).ok_or(ImageRejection::UnsupportedType)?;
    if !ALLOWED_MIME_TYPES.contains(&kind.mime_type()) {
        return Err(ImageRejection::UnsupportedType);
    }

    // Header parse, never a decode, which is what keeps a bomb cheap to reject.
    let dimensions = imagesize::blob_size(bytes).ok();
    if dimensions
        .as_ref()
        .is_some_and(|size| (size.width as u64) * (size.height as u64) > MAX_PIXELS)
    {
        return Err(ImageRejection::TooLarge);
    }

    Ok(InspectedImage {
        mime_type: kind.mime_type().to_string(),
        extension: kind.extension().to_string(),
        width: dimensions.as_ref().map(|size| size.width as i32),
        height: dimensions.as_ref().map(|size| size.height as i32),
    })
}

/// An already-inspected image and the bytes it was inspected from, ready to store.
pub struct StoreImageParams<'a> {
    pub coaching_session_id: Id,
    pub uploaded_by_id: Id,
    pub bytes: Vec<u8>,
    pub inspected: &'a InspectedImage,
}

/// Object storage path for one note image. Session-prefixed so a future cleanup can drop
/// a whole prefix, and so other asset kinds can sit under a sibling prefix.
fn storage_key(coaching_session_id: Id, image_id: Id, extension: &str) -> String {
    format!("coaching-sessions/{coaching_session_id}/images/{image_id}.{extension}")
}

/// Stores the bytes and records the metadata row that points at them.
///
/// # Errors
///
/// Returns the object store's error when the upload fails, and a `DomainErrorKind::Internal`
/// converted from `entity_api` when the insert fails.
pub async fn create(
    db: &DatabaseConnection,
    store: &dyn ObjectStore,
    params: StoreImageParams<'_>,
) -> Result<Model, Error> {
    let image_id = Id::new_v4();
    let key = storage_key(
        params.coaching_session_id,
        image_id,
        &params.inspected.extension,
    );
    let byte_size = params.bytes.len() as i64;

    // Object first, row second: a failed insert leaves a harmless orphan object, whereas a
    // failed put after the insert would leave a row pointing at bytes that do not exist.
    store
        .put(&key, params.bytes, &params.inspected.mime_type)
        .await?;

    Ok(entity_api::coaching_session_image::create(
        db,
        NewCoachingSessionImage {
            coaching_session_id: params.coaching_session_id,
            uploaded_by_id: params.uploaded_by_id,
            storage_key: key,
            mime_type: params.inspected.mime_type.clone(),
            byte_size,
            width: params.inspected.width,
            height: params.inspected.height,
        },
    )
    .await?)
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;

    /// A valid 2x3 truecolor PNG, built byte by byte so no fixture file is needed.
    const TINY_PNG: [u8; 73] = [
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x03, 0x08, 0x02, 0x00, 0x00, 0x00, 0x36,
        0x88, 0x49, 0xd6, 0x00, 0x00, 0x00, 0x10, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8,
        0xcf, 0xc0, 0x00, 0x44, 0x0c, 0x28, 0x14, 0x00, 0x44, 0xd0, 0x05, 0xfb, 0xa4, 0xcf, 0xde,
        0x80, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    const TEN_MB: u64 = 10 * 1024 * 1024;

    #[test]
    fn accepts_a_png_and_reports_its_type() {
        let inspected = inspect_image(&TINY_PNG, TEN_MB).expect("a real PNG is accepted");

        assert_eq!(inspected.mime_type, "image/png");
        assert_eq!(inspected.extension, "png");
    }

    #[test]
    fn rejects_svg_as_an_unsupported_type() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><script>alert(1)</script></svg>"#;

        assert_eq!(
            inspect_image(svg, TEN_MB).err(),
            Some(ImageRejection::UnsupportedType),
            "SVG must stay off the allowlist: it is scriptable XML served from our own origin"
        );
    }

    /// Guards the allowlist itself. The SVG case above never reaches it — `infer` has no
    /// signature for SVG, so the sniff returns nothing and that test stops one step earlier.
    /// BMP is the case that matters: recognizable, and deliberately not on the allowlist.
    #[test]
    fn rejects_a_sniffable_format_that_is_off_the_allowlist() {
        /// A valid 1x1 24-bit BMP: 14-byte file header, 40-byte DIB header, one pixel.
        const TINY_BMP: [u8; 58] = [
            0x42, 0x4d, 0x3a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x36, 0x00, 0x00, 0x00,
            0x28, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00,
            0x18, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff,
            0x00, 0x00,
        ];

        // Without this the test would be vacuous: unsniffable bytes fail one check earlier.
        assert_eq!(
            infer::get(&TINY_BMP).map(|kind| kind.mime_type()),
            Some("image/bmp"),
            "these bytes must be sniffable, otherwise the allowlist is never consulted"
        );

        assert_eq!(
            inspect_image(&TINY_BMP, TEN_MB).err(),
            Some(ImageRejection::UnsupportedType)
        );
    }

    #[test]
    fn rejects_an_oversize_png_distinguishably_from_an_unsupported_type() {
        let oversize = inspect_image(&TINY_PNG, (TINY_PNG.len() - 1) as u64).err();

        assert_eq!(oversize, Some(ImageRejection::TooLarge));
        // B3 maps TooLarge to 413 and UnsupportedType to 415, so the two must not collapse.
        assert_ne!(oversize, Some(ImageRejection::UnsupportedType));
    }

    #[test]
    fn rejects_arbitrary_non_image_bytes() {
        assert_eq!(
            inspect_image(b"not an image at all", TEN_MB).err(),
            Some(ImageRejection::UnsupportedType)
        );
    }

    #[test]
    fn reports_real_dimensions_for_a_png() {
        let inspected = inspect_image(&TINY_PNG, TEN_MB).expect("a real PNG is accepted");

        assert_eq!(inspected.width, Some(2));
        assert_eq!(inspected.height, Some(3));
    }
}
