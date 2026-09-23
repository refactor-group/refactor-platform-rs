//! Coaching note images: validate uploaded bytes, then store the object before recording it.
//!
//! Validation lives here because this is the security boundary: the declared content type
//! from the request never reaches this module, only the bytes themselves.
//!
//! An upload is spooled to a temporary file as it arrives and streamed to storage from
//! there, so the memory an upload holds does not grow with the size of the image.

use std::io::{BufRead, BufReader, Read, Seek};

use entity_api::coaching_session_image::NewCoachingSessionImage;
use log::*;
use sea_orm::ConnectionTrait;
use sea_orm::DatabaseConnection;
use service::config::Config;
use tempfile::TempPath;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::coaching_session_images::Model;
use crate::error::{DomainErrorKind, Error, InternalErrorKind};
use crate::gateway::object_storage::{self, io_error, ObjectStore};
use crate::Id;

pub use entity_api::coaching_session_image::{find_by_id, restore, soft_delete};

/// Image types we will accept and later serve from our own origin. `image/svg+xml` is
/// absent deliberately: SVG is XML that can carry script, so serving it back would be
/// stored XSS against a signed-in coach.
const ALLOWED_MIME_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];

/// Pixel ceiling. A decompression bomb is small on the wire but ruinous once decoded,
/// and no legitimate pasted screenshot comes close.
const MAX_PIXELS: u64 = 50_000_000;

/// Bytes read to sniff an upload's type. Every allowed signature sits in the first dozen.
const SNIFF_BYTES: u64 = 8 * 1024;

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
    pub width: i32,
    pub height: i32,
}

/// An upload being written to disk as it arrives.
pub struct Spool {
    file: File,
    path: TempPath,
    len: u64,
}

impl Spool {
    /// Opens a fresh temporary file, removed when the spool or its [`Upload`] is dropped,
    /// including when the client disconnects mid-upload.
    ///
    /// # Errors
    ///
    /// Returns `InternalErrorKind::Other` when the file cannot be created.
    pub async fn new() -> Result<Self, Error> {
        let (file, path) = tokio::task::spawn_blocking(|| {
            tempfile::Builder::new()
                .prefix("coaching-session-image-")
                .tempfile()
        })
        .await
        .map_err(task_error)?
        .map_err(|e| io_error("create an upload spool file", e))?
        .into_parts();

        Ok(Self {
            file: File::from_std(file),
            path,
            len: 0,
        })
    }

    /// Appends one chunk of the upload.
    ///
    /// # Errors
    ///
    /// Returns `InternalErrorKind::Other` when the write fails.
    pub async fn write(&mut self, chunk: &[u8]) -> Result<(), Error> {
        self.file
            .write_all(chunk)
            .await
            .map_err(|e| io_error("spool an upload chunk", e))?;
        self.len += chunk.len() as u64;
        Ok(())
    }

    /// Flushes every write to disk, so what is read back is everything that arrived.
    ///
    /// # Errors
    ///
    /// Returns `InternalErrorKind::Other` when the flush fails.
    pub async fn finish(mut self) -> Result<Upload, Error> {
        self.file
            .flush()
            .await
            .map_err(|e| io_error("flush an upload spool file", e))?;

        Ok(Upload {
            path: self.path,
            len: self.len,
        })
    }
}

/// A fully spooled upload. Its file is removed when this is dropped.
pub struct Upload {
    path: TempPath,
    len: u64,
}

impl Upload {
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Reads what the upload's header says it is. Only the header is read.
    ///
    /// # Errors
    ///
    /// Returns `InternalErrorKind::Other` when the file cannot be read. A header that does
    /// not parse is not an error here; [`inspect_image`] rejects it.
    pub async fn probe(&self) -> Result<ImageProbe, Error> {
        let path = self.path.to_path_buf();
        let len = self.len;

        tokio::task::spawn_blocking(move || {
            std::fs::File::open(path).and_then(|file| ImageProbe::read(BufReader::new(file), len))
        })
        .await
        .map_err(task_error)?
        .map_err(|e| io_error("read an upload's header", e))
    }
}

/// What an upload's own header says it is. Plain data, so [`inspect_image`] stays a pure
/// policy over it.
pub struct ImageProbe {
    byte_len: u64,
    /// Sniffed mime type and extension, when the signature matches anything at all.
    kind: Option<(&'static str, &'static str)>,
    /// Header width and height, when the header parses.
    dimensions: Option<(usize, usize)>,
}

impl ImageProbe {
    /// Reads the signature and header from `reader`, which holds `byte_len` bytes. The
    /// parser seeks to wherever the format declares its size, so a JPEG whose metadata
    /// pushes the frame header deep into the file still resolves.
    fn read(mut reader: impl BufRead + Seek, byte_len: u64) -> std::io::Result<Self> {
        let mut head = Vec::new();
        reader.by_ref().take(SNIFF_BYTES).read_to_end(&mut head)?;
        reader.rewind()?;

        Ok(Self {
            byte_len,
            kind: infer::get(&head).map(|kind| (kind.mime_type(), kind.extension())),
            dimensions: imagesize::reader_size(&mut reader)
                .ok()
                .map(|size| (size.width, size.height)),
        })
    }
}

/// Apply the upload policy to a probe: the size cap, the type allowlist, and dimensions
/// small enough that a browser can decode them.
///
/// # Errors
///
/// Returns `ImageRejection::TooLarge` when the upload exceeds `max_bytes` or the header's
/// pixel count exceeds the bomb ceiling, and `ImageRejection::UnsupportedType` when the
/// sniffed type is absent or outside the allowlist, or its header does not parse.
pub fn inspect_image(probe: &ImageProbe, max_bytes: u64) -> Result<InspectedImage, ImageRejection> {
    if probe.byte_len > max_bytes {
        return Err(ImageRejection::TooLarge);
    }

    // Magic bytes only. What the request claimed the file was is never consulted.
    let (mime_type, extension) = probe
        .kind
        .filter(|(mime_type, _)| ALLOWED_MIME_TYPES.contains(mime_type))
        .ok_or(ImageRejection::UnsupportedType)?;

    // Every allowed format declares its size in the header, so one that will not parse is
    // corrupt. Letting it through would also skip the pixel ceiling below.
    let (width, height) = probe
        .dimensions
        .filter(|&(width, height)| width > 0 && height > 0)
        .ok_or(ImageRejection::UnsupportedType)?;

    // Header parse, never a decode, which is what keeps a bomb cheap to reject.
    if (width as u64).saturating_mul(height as u64) > MAX_PIXELS {
        return Err(ImageRejection::TooLarge);
    }

    Ok(InspectedImage {
        mime_type: mime_type.to_string(),
        extension: extension.to_string(),
        width: i32::try_from(width).map_err(|_| ImageRejection::TooLarge)?,
        height: i32::try_from(height).map_err(|_| ImageRejection::TooLarge)?,
    })
}

/// A blocking spool task that panicked or was cancelled.
fn task_error(err: tokio::task::JoinError) -> Error {
    warn!("Upload spool task failed: {err}");
    Error {
        source: Some(Box::new(err)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
            "upload spool task failed".to_string(),
        )),
    }
}

/// An inspected upload, ready to store.
pub struct StoreImageParams<'a> {
    pub coaching_session_id: Id,
    pub uploaded_by_id: Id,
    pub upload: Upload,
    pub inspected: &'a InspectedImage,
}

/// Object storage path for one note image. Session-prefixed so a future cleanup can drop
/// a whole prefix, and so other asset kinds can sit under a sibling prefix.
///
/// `object_id` is generated per object and is deliberately **not** the row id: the row id
/// travels in image URLs, and reusing it here would make every stored object's path
/// guessable from one. The row's `storage_key` column is the only link between the two.
fn storage_key(coaching_session_id: Id, object_id: Id, extension: &str) -> String {
    format!("coaching-sessions/{coaching_session_id}/images/{object_id}.{extension}")
}

/// Streams the upload to storage and records the metadata row that points at it.
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
    let object_id = Id::new_v4();
    let key = storage_key(
        params.coaching_session_id,
        object_id,
        &params.inspected.extension,
    );

    // Object first, row second: a failed insert leaves a harmless orphan object, whereas a
    // failed put after the insert would leave a row pointing at bytes that do not exist.
    store
        .put(&key, &params.upload.path, &params.inspected.mime_type)
        .await?;

    Ok(entity_api::coaching_session_image::create(
        db,
        NewCoachingSessionImage {
            coaching_session_id: params.coaching_session_id,
            uploaded_by_id: params.uploaded_by_id,
            storage_key: key,
            mime_type: params.inspected.mime_type.clone(),
            byte_size: params.upload.len() as i64,
            width: Some(params.inspected.width),
            height: Some(params.inspected.height),
        },
    )
    .await?)
}

/// Storage keys of every image belonging to `coaching_session_ids`.
///
/// Deleting a coaching session cascades its image rows away, taking the only pointer to
/// the bytes with them, so the keys have to be read while the rows still exist. Pair this
/// with [`destroy_objects`] after the delete commits.
///
/// An upload committing between this read and the delete is cascaded away with its object
/// left behind. Locking the session would not help: [`create`] stores the object before its
/// row, so a blocked upload orphans it on the failed insert instead. A reaper owns these.
///
/// # Errors
///
/// Returns a `DomainErrorKind::Internal` converted from `entity_api` when the query fails.
pub async fn storage_keys_for_sessions(
    db: &impl ConnectionTrait,
    coaching_session_ids: &[Id],
) -> Result<Vec<String>, Error> {
    Ok(
        entity_api::coaching_session_image::find_by_coaching_session_ids(db, coaching_session_ids)
            .await?
            .into_iter()
            .map(|image| image.storage_key)
            .collect(),
    )
}

/// Destroys the stored objects behind `storage_keys`, after the rows that pointed at them
/// are gone.
///
/// Best effort, like the Tiptap cleanup it sits beside: a storage outage must not fail a
/// delete the database has already committed, and the worst outcome is bytes nothing
/// points at. Called only for a session that no longer exists, so there is no undo left to
/// protect and nothing to defer behind the purge job's grace period.
pub async fn destroy_objects(config: &Config, storage_keys: &[String]) {
    if storage_keys.is_empty() {
        return;
    }

    let Some(store) = object_storage::from_config(config) else {
        warn!(
            "Object storage is unconfigured; {} note image object(s) are stranded",
            storage_keys.len()
        );
        return;
    };

    for key in storage_keys {
        if let Err(e) = store.delete(key).await {
            warn!("Could not destroy orphaned note image object {key}: {e:?}");
        }
    }
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use std::io::Cursor;

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

    /// Probes in-memory bytes through the same reader path a spooled file takes.
    fn probe(bytes: &[u8]) -> ImageProbe {
        ImageProbe::read(Cursor::new(bytes), bytes.len() as u64).expect("a cursor cannot fail")
    }

    /// `TINY_PNG` with its header's width and height replaced.
    fn png_declaring(width: u32, height: u32) -> Vec<u8> {
        let mut png = TINY_PNG.to_vec();
        png[16..20].copy_from_slice(&width.to_be_bytes());
        png[20..24].copy_from_slice(&height.to_be_bytes());
        png
    }

    /// A JPEG whose frame header sits behind `segments` maximal APP1 segments, the way a
    /// phone photo's EXIF and XMP push it deep into the file.
    fn jpeg_with_metadata(segments: usize, width: u16, height: u16) -> Vec<u8> {
        let mut jpeg = vec![0xFF, 0xD8];
        for _ in 0..segments {
            // APP1 with the largest length a segment can declare, counting its own two bytes.
            jpeg.extend_from_slice(&[0xFF, 0xE1, 0xFF, 0xFF]);
            jpeg.resize(jpeg.len() + 65533, 0);
        }
        jpeg.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08]);
        jpeg.extend_from_slice(&height.to_be_bytes());
        jpeg.extend_from_slice(&width.to_be_bytes());
        jpeg.extend_from_slice(&[0x03, 0x01, 0x22, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01]);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        jpeg
    }

    #[test]
    fn accepts_a_png_and_reports_its_type() {
        let inspected = inspect_image(&probe(&TINY_PNG), TEN_MB).expect("a real PNG is accepted");

        assert_eq!(inspected.mime_type, "image/png");
        assert_eq!(inspected.extension, "png");
    }

    #[test]
    fn rejects_svg_as_an_unsupported_type() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><script>alert(1)</script></svg>"#;

        assert_eq!(
            inspect_image(&probe(svg), TEN_MB).err(),
            Some(ImageRejection::UnsupportedType),
            "SVG must stay off the allowlist: it is scriptable XML served from our own origin"
        );
    }

    /// Guards the allowlist itself. The SVG case above never reaches it: `infer` has no
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
            inspect_image(&probe(&TINY_BMP), TEN_MB).err(),
            Some(ImageRejection::UnsupportedType)
        );
    }

    #[test]
    fn rejects_an_oversize_png_distinguishably_from_an_unsupported_type() {
        let oversize = inspect_image(&probe(&TINY_PNG), (TINY_PNG.len() - 1) as u64).err();

        assert_eq!(oversize, Some(ImageRejection::TooLarge));
        // TooLarge maps to 413 and UnsupportedType to 415, so the two must not collapse.
        assert_ne!(oversize, Some(ImageRejection::UnsupportedType));
    }

    #[test]
    fn rejects_arbitrary_non_image_bytes() {
        assert_eq!(
            inspect_image(&probe(b"not an image at all"), TEN_MB).err(),
            Some(ImageRejection::UnsupportedType)
        );
    }

    #[test]
    fn reports_real_dimensions_for_a_png() {
        let inspected = inspect_image(&probe(&TINY_PNG), TEN_MB).expect("a real PNG is accepted");

        assert_eq!((inspected.width, inspected.height), (2, 3));
    }

    /// An allowed signature over a header that ends early. Passing it would also skip the
    /// pixel ceiling, since there are no dimensions to check.
    #[test]
    fn rejects_an_allowed_signature_whose_header_does_not_parse() {
        let truncated_png = &TINY_PNG[..20];
        let truncated_jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0x00];

        for (name, bytes) in [("png", truncated_png), ("jpeg", &truncated_jpeg[..])] {
            let probed = probe(bytes);
            assert!(
                probed.kind.is_some(),
                "{name}: the signature must sniff, or the header check is never reached"
            );
            assert_eq!(
                inspect_image(&probed, TEN_MB).err(),
                Some(ImageRejection::UnsupportedType),
                "{name}: an unparseable header must be refused"
            );
        }
    }

    #[test]
    fn rejects_a_header_declaring_an_empty_image() {
        assert_eq!(
            inspect_image(&probe(&png_declaring(0, 3)), TEN_MB).err(),
            Some(ImageRejection::UnsupportedType)
        );
    }

    /// A decompression bomb: tiny on the wire, 100 megapixels once a browser decodes it.
    #[test]
    fn rejects_a_header_declaring_more_pixels_than_the_ceiling() {
        assert_eq!(
            inspect_image(&probe(&png_declaring(10_000, 10_000)), TEN_MB).err(),
            Some(ImageRejection::TooLarge)
        );
    }

    /// The frame header sits 128 KiB in. Reading dimensions from a fixed-size prefix would
    /// miss it; seeking through the file finds it.
    #[test]
    fn reads_dimensions_declared_deep_in_a_jpeg() {
        let inspected = inspect_image(&probe(&jpeg_with_metadata(2, 640, 480)), TEN_MB)
            .expect("a JPEG with large metadata is accepted");

        assert_eq!(inspected.mime_type, "image/jpeg");
        assert_eq!((inspected.width, inspected.height), (640, 480));
    }

    /// The path production takes: chunks spooled to disk, then probed from the file.
    #[tokio::test]
    async fn a_spooled_upload_is_probed_from_disk() -> Result<(), Error> {
        let jpeg = jpeg_with_metadata(2, 640, 480);

        let mut spool = Spool::new().await?;
        for chunk in jpeg.chunks(16 * 1024) {
            spool.write(chunk).await?;
        }
        let upload = spool.finish().await?;

        assert_eq!(upload.len(), jpeg.len() as u64);
        let inspected =
            inspect_image(&upload.probe().await?, TEN_MB).expect("the spooled JPEG is accepted");
        assert_eq!((inspected.width, inspected.height), (640, 480));

        Ok(())
    }

    #[tokio::test]
    async fn an_upload_removes_its_file_when_dropped() -> Result<(), Error> {
        let mut spool = Spool::new().await?;
        spool.write(&TINY_PNG).await?;
        let upload = spool.finish().await?;
        let path = upload.path.to_path_buf();
        assert!(
            path.exists(),
            "the spool file exists while the upload is held"
        );

        drop(upload);

        assert!(!path.exists(), "the spool file must not outlive its upload");
        Ok(())
    }

    /// A client that disconnects mid-upload drops the spool before it finishes.
    #[tokio::test]
    async fn an_abandoned_spool_removes_its_file() -> Result<(), Error> {
        let mut spool = Spool::new().await?;
        spool.write(&TINY_PNG[..10]).await?;
        let path = spool.path.to_path_buf();

        drop(spool);

        assert!(
            !path.exists(),
            "an abandoned upload must not leave a file behind"
        );
        Ok(())
    }
}
