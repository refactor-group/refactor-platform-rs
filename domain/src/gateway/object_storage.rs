//! Object storage gateway.
//!
//! One `ObjectStore` trait with two backends: the local filesystem for development and
//! DigitalOcean Spaces everywhere else. Spaces requests are signed by `rusty-s3`, which
//! does no I/O of its own, so every byte still travels over our own rustls client.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use log::*;
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE};
use reqwest::{Body, Url};
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use service::config::Config;
use tokio::fs;
use tokio_util::io::ReaderStream;

use crate::error::{DomainErrorKind, EntityErrorKind, Error, ExternalErrorKind, InternalErrorKind};

/// Served when a backend cannot tell us what an object actually is.
const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";

/// Suffix of the sibling file that records a local object's content type.
const CONTENT_TYPE_SUFFIX: &str = ".content-type";

/// Signature lifetime for uploads and deletes. Both are sent immediately after signing.
const IMMEDIATE_SIGNATURE_TTL: Duration = Duration::from_secs(60);

/// Bounds every Spaces request, so a hung connection cannot hold a caller forever. The
/// purge job holds a row lock across its delete, which makes an unbounded wait a stall.
const SPACES_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const SPACES_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Read size when streaming an upload from disk. The only upload buffer held in memory.
const UPLOAD_STREAM_CHUNK_BYTES: usize = 64 * 1024;

/// Objects are keyed by a generated id and never rewritten, so a long byte cache is safe.
///
/// It is not as long as it looks. Every signing stamps a fresh `X-Amz-Date` and signature,
/// so each redirect points at a different URL and the browser's cache, keyed on that URL,
/// starts over. The 24 hours only apply within one signature's lifetime; a viewer still
/// re-fetches the bytes roughly once per presign TTL. A stable URL would need a signature
/// that lives as long as the cache entry, which trades link lifetime for bandwidth.
const IMMUTABLE_CACHE_CONTROL: &str = "private, max-age=86400, immutable";

/// A stored object's bytes plus the content type to serve them with.
pub struct StoredObject {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

/// Storage backend for binary assets. One implementation is built at startup and shared.
#[async_trait]
pub trait ObjectStore: Send + Sync + 'static {
    /// Store the file at `source` under `key`, recording `content_type` on the object.
    ///
    /// Takes a file rather than bytes so no backend ever needs the whole object in memory.
    ///
    /// # Errors
    ///
    /// Returns `InternalErrorKind::Config` for a key that escapes the store root, and
    /// a transport error kind when the write itself fails.
    async fn put(&self, key: &str, source: &Path, content_type: &str) -> Result<(), Error>;

    /// A time-limited URL a browser can fetch directly, when this backend supports signing.
    /// Returns `Ok(None)` when it does not (the local backend), in which case callers fall
    /// back to `get`.
    ///
    /// Signing touches no network, so a key with no object behind it still signs. The
    /// caller's 404 for a missing object therefore only comes from [`ObjectStore::get`];
    /// on a presigning backend the miss surfaces as the store's own 404 at the signed URL.
    ///
    /// # Errors
    ///
    /// Returns `InternalErrorKind::Config` for a key this backend refuses to sign.
    fn presigned_get(&self, key: &str, ttl: Duration) -> Result<Option<String>, Error>;

    /// Read an object's bytes. Used by backends that cannot presign.
    ///
    /// # Errors
    ///
    /// Returns `InternalErrorKind::Config` for a key that escapes the store root,
    /// `EntityErrorKind::NotFound` when there is no object at `key`, and a transport
    /// error kind when the object cannot be read.
    async fn get(&self, key: &str) -> Result<StoredObject, Error>;

    /// Remove an object.
    ///
    /// # Errors
    ///
    /// Returns a transport error kind when the removal fails. An already-absent object is `Ok`.
    async fn delete(&self, key: &str) -> Result<(), Error>;
}

/// Misconfiguration, or a request we refuse to make. Logged here so every caller does not have to.
fn config_error(message: String) -> Error {
    warn!("{message}");
    Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
    }
}

/// The remote store was reachable but refused, or could not be reached at all.
fn transport_error(message: String) -> Error {
    warn!("{message}");
    Error {
        source: None,
        error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
    }
}

/// A Spaces request that never completed. The URL is dropped before the error is ever
/// rendered: these requests are presigned, so the URL carries `X-Amz-Credential` (which
/// embeds the access key id) and `X-Amz-Signature`, and both `Debug` and `Display` on a
/// `reqwest::Error` print it. Propagating the error as-is would write a usable signature
/// and a permanent key id into the logs on any DNS blip or connect failure.
fn spaces_transport_error(operation: &str, key: &str, err: reqwest::Error) -> Error {
    transport_error(format!(
        "Spaces {operation} of {key} failed: {}",
        err.without_url()
    ))
}

/// The object is not there. A row can outlive its bytes (a failed upload, a purge that
/// got halfway, a bucket restored from an older snapshot), and that is a 404, not a fault
/// in this server. Mapping it to an internal error would page someone for a missing file.
fn object_not_found(key: &str) -> Error {
    warn!("Object store has no object at {key}");
    Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::NotFound)),
    }
}

/// Local disk failure. `Internal(Other)` rather than `External(Network)` because nothing
/// left the process, and a reader seeing "network" for a full disk would be misled.
pub(crate) fn io_error(context: &str, err: std::io::Error) -> Error {
    warn!("Failed to {context}: {err}");
    Error {
        source: Some(Box::new(err)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(format!(
            "failed to {context}"
        ))),
    }
}

/// Filesystem-backed store for local development. Cannot sign URLs, so reads stream bytes.
pub struct LocalObjectStore {
    root: PathBuf,
}

impl LocalObjectStore {
    /// Builds a store rooted at `root`. The directory is created lazily on first write.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Maps `key` to a path under the root, refusing anything that could escape it.
    ///
    /// Absolute keys and `..` components are rejected outright; the deepest existing
    /// ancestor is then canonicalized to catch a symlink pointing out of the tree, which
    /// a purely lexical check cannot see.
    ///
    /// # Errors
    ///
    /// Returns `InternalErrorKind::Config` for any key that would land outside the root.
    fn resolve(&self, key: &str) -> Result<PathBuf, Error> {
        let candidate = Path::new(key);
        let escapes = candidate.is_absolute()
            || candidate
                .components()
                .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir));

        if escapes {
            return Err(config_error(format!(
                "Rejected object key that escapes the local store root: {key}"
            )));
        }

        let path = self.root.join(candidate);
        self.confirm_within_root(&path)?;
        Ok(path)
    }

    /// Confirms `path` resolves inside the root once symlinks are followed.
    fn confirm_within_root(&self, path: &Path) -> Result<(), Error> {
        let (Ok(root), Some(Ok(anchor))) = (
            self.root.canonicalize(),
            path.ancestors()
                .find(|ancestor| ancestor.exists())
                .map(Path::canonicalize),
        ) else {
            // Nothing on disk to resolve yet, so the lexical check above stands alone.
            return Ok(());
        };

        anchor.starts_with(&root).then_some(()).ok_or_else(|| {
            config_error(format!(
                "Rejected object key resolving outside the local store root: {}",
                path.display()
            ))
        })
    }

    /// Path of the sibling file holding an object's content type.
    fn content_type_path(path: &Path) -> PathBuf {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(CONTENT_TYPE_SUFFIX);
        PathBuf::from(sidecar)
    }
}

#[async_trait]
impl ObjectStore for LocalObjectStore {
    async fn put(&self, key: &str, source: &Path, content_type: &str) -> Result<(), Error> {
        let path = self.resolve(key)?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| io_error("create the object's parent directory", e))?;
        }

        // A copy, not a rename: the source may sit on another filesystem.
        fs::copy(source, &path)
            .await
            .map_err(|e| io_error("write the object", e))?;

        // Recorded alongside the bytes so `get` reports what the uploader declared
        // rather than re-sniffing and possibly disagreeing with the stored row.
        fs::write(Self::content_type_path(&path), content_type)
            .await
            .map_err(|e| io_error("write the object's content type", e))
    }

    fn presigned_get(&self, _key: &str, _ttl: Duration) -> Result<Option<String>, Error> {
        Ok(None) // A local file has no signable URL; callers fall back to `get`.
    }

    async fn get(&self, key: &str) -> Result<StoredObject, Error> {
        let path = self.resolve(key)?;

        let bytes = fs::read(&path).await.map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => object_not_found(key),
            _ => io_error("read the object", e),
        })?;

        let content_type = fs::read_to_string(Self::content_type_path(&path))
            .await
            .unwrap_or_else(|_| DEFAULT_CONTENT_TYPE.to_string());

        Ok(StoredObject {
            bytes,
            content_type,
        })
    }

    async fn delete(&self, key: &str) -> Result<(), Error> {
        let path = self.resolve(key)?;

        match fs::remove_file(&path).await {
            // An already-absent object is the outcome we wanted.
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io_error("remove the object", e)),
        }?;

        // Best effort: an orphaned sidecar is harmless, and its absence is the normal case.
        let _ = fs::remove_file(Self::content_type_path(&path)).await;
        Ok(())
    }
}

/// DigitalOcean Spaces store. `rusty-s3` signs; this client sends.
pub struct SpacesObjectStore {
    bucket: Bucket,
    credentials: Credentials,
    /// Header-less rustls client. Presigned URLs carry their signature in the query
    /// string and reject an extra `Authorization` header.
    client: reqwest::Client,
}

impl SpacesObjectStore {
    /// Builds a Spaces store from config.
    ///
    /// # Errors
    ///
    /// Returns `InternalErrorKind::Config` when the endpoint, bucket, or either
    /// credential is missing or malformed.
    pub fn from_config(config: &Config) -> Result<Self, Error> {
        let endpoint = config
            .spaces_endpoint()
            .ok_or_else(|| config_error("SPACES_ENDPOINT is not set".to_string()))?;
        let endpoint = Url::parse(&endpoint)
            .map_err(|e| config_error(format!("SPACES_ENDPOINT is not a valid URL: {e}")))?;
        let bucket_name = config
            .spaces_bucket()
            .ok_or_else(|| config_error("SPACES_BUCKET is not set".to_string()))?;
        let access_key_id = config
            .spaces_access_key_id()
            .ok_or_else(|| config_error("SPACES_ACCESS_KEY_ID is not set".to_string()))?;
        let secret_access_key = config
            .spaces_secret_access_key()
            .ok_or_else(|| config_error("SPACES_SECRET_ACCESS_KEY is not set".to_string()))?;

        // Spaces addresses buckets as `<bucket>.<region>.digitaloceanspaces.com`.
        let bucket = Bucket::new(
            endpoint,
            UrlStyle::VirtualHost,
            bucket_name,
            config.spaces_region().to_string(),
        )
        .map_err(|e| config_error(format!("Failed to build the Spaces bucket: {e}")))?;

        Ok(Self {
            bucket,
            credentials: Credentials::new(access_key_id, secret_access_key),
            client: reqwest::Client::builder()
                .use_rustls_tls()
                .connect_timeout(SPACES_CONNECT_TIMEOUT)
                .timeout(SPACES_REQUEST_TIMEOUT)
                .build()?,
        })
    }
}

#[async_trait]
impl ObjectStore for SpacesObjectStore {
    async fn put(&self, key: &str, source: &Path, content_type: &str) -> Result<(), Error> {
        let file = fs::File::open(source)
            .await
            .map_err(|e| io_error("open the upload to stream it", e))?;
        // S3 rejects a chunked PUT, so the length has to be declared before the first byte.
        let content_length = file
            .metadata()
            .await
            .map_err(|e| io_error("read the upload's length", e))?
            .len();

        let url = self
            .bucket
            .put_object(Some(&self.credentials), key)
            .sign(IMMEDIATE_SIGNATURE_TTL);

        let response = self
            .client
            .put(url)
            .header(CONTENT_TYPE, content_type)
            .header(CONTENT_LENGTH, content_length)
            .body(Body::wrap_stream(ReaderStream::with_capacity(
                file,
                UPLOAD_STREAM_CHUNK_BYTES,
            )))
            .send()
            .await
            .map_err(|e| spaces_transport_error("upload", key, e))?;

        match response.status() {
            status if status.is_success() => Ok(()),
            status => Err(transport_error(format!(
                "Spaces rejected the upload of {key} with status {status}"
            ))),
        }
    }

    fn presigned_get(&self, key: &str, ttl: Duration) -> Result<Option<String>, Error> {
        let mut action = self.bucket.get_object(Some(&self.credentials), key);
        // Part of the signed query, so the cache directive cannot be stripped in transit.
        action
            .query_mut()
            .insert("response-cache-control", IMMUTABLE_CACHE_CONTROL);

        Ok(Some(action.sign(ttl).to_string()))
    }

    async fn get(&self, key: &str) -> Result<StoredObject, Error> {
        let url = self
            .presigned_get(key, IMMEDIATE_SIGNATURE_TTL)?
            .ok_or_else(|| config_error(format!("Failed to sign a GET URL for {key}")))?;

        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| spaces_transport_error("read", key, e))?;

        let status = response.status();
        if status.as_u16() == 404 {
            return Err(object_not_found(key));
        }
        if !status.is_success() {
            return Err(transport_error(format!(
                "Spaces refused to serve {key} with status {status}"
            )));
        }

        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or(DEFAULT_CONTENT_TYPE)
            .to_string();

        // `Vec::from` takes over the buffer where it can; `to_vec` always copied it.
        let bytes = Vec::from(
            response
                .bytes()
                .await
                .map_err(|e| spaces_transport_error("read", key, e))?,
        );

        Ok(StoredObject {
            bytes,
            content_type,
        })
    }

    async fn delete(&self, key: &str) -> Result<(), Error> {
        let url = self
            .bucket
            .delete_object(Some(&self.credentials), key)
            .sign(IMMEDIATE_SIGNATURE_TTL);

        let response = self
            .client
            .delete(url)
            .send()
            .await
            .map_err(|e| spaces_transport_error("delete", key, e))?;

        match response.status() {
            // 404 means the object is already gone, which is the outcome we wanted.
            status if status.is_success() || status.as_u16() == 404 => Ok(()),
            status => Err(transport_error(format!(
                "Spaces refused to delete {key} with status {status}"
            ))),
        }
    }
}

/// Build the configured object store, or `None` when storage is not configured.
/// A `None` store makes the image endpoints return 503 rather than preventing boot.
pub fn from_config(config: &Config) -> Option<Arc<dyn ObjectStore>> {
    match config.object_store_backend() {
        "local" => Some(Arc::new(LocalObjectStore::new(
            config.object_store_local_path(),
        ))),
        "spaces" => SpacesObjectStore::from_config(config)
            .map(|store| Arc::new(store) as Arc<dyn ObjectStore>)
            .inspect_err(|_| {
                warn!("OBJECT_STORE_BACKEND is \"spaces\" but Spaces is not fully configured — object storage disabled")
            })
            .ok(),
        other => {
            warn!("Unrecognized OBJECT_STORE_BACKEND \"{other}\" — object storage disabled");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    /// A unique directory outside the repo, removed when the test ends.
    struct TempRoot {
        path: PathBuf,
    }

    impl TempRoot {
        fn new() -> Self {
            Self {
                path: std::env::temp_dir().join(format!("object-store-test-{}", Uuid::new_v4())),
            }
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// Writes `bytes` to a source file outside the store root, as an upload would arrive.
    fn source_file(bytes: &[u8]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().expect("source file is created");
        std::io::Write::write_all(&mut file, bytes).expect("source bytes are written");
        file
    }

    fn assert_config_error(error: Error, context: &str) {
        assert_eq!(
            error.error_kind,
            DomainErrorKind::Internal(InternalErrorKind::Config),
            "{context} should be refused as a misconfigured key"
        );
    }

    fn spaces_store() -> SpacesObjectStore {
        let config = Config::from_args([
            "refactor-platform-rs",
            "--object-store-backend=spaces",
            "--spaces-endpoint=https://nyc3.digitaloceanspaces.com",
            "--spaces-region=nyc3",
            "--spaces-bucket=test-bucket",
            "--spaces-access-key-id=test-key-id",
            "--spaces-secret-access-key=test-secret",
        ]);

        SpacesObjectStore::from_config(&config).expect("fully configured Spaces store builds")
    }

    #[tokio::test]
    async fn local_put_get_round_trip_preserves_bytes_and_content_type() {
        let root = TempRoot::new();
        let store = LocalObjectStore::new(&root.path);
        let key = "coaching-sessions/abc/images/def.png";

        let source = source_file(&[0x89, b'P', b'N', b'G']);
        store
            .put(key, source.path(), "image/png")
            .await
            .expect("put succeeds");

        let stored = store.get(key).await.expect("get succeeds");
        assert_eq!(stored.bytes, vec![0x89, b'P', b'N', b'G']);
        assert_eq!(stored.content_type, "image/png");
    }

    /// A row can outlive its bytes, and the caller has to be able to tell that apart from
    /// this server being broken. Without the `NotFound` arm in `get` this is an internal
    /// error and the endpoint answers 500.
    #[tokio::test]
    async fn local_get_reports_a_missing_object_as_not_found() {
        let root = TempRoot::new();
        let store = LocalObjectStore::new(&root.path);

        let Err(error) = store
            .get("coaching-sessions/abc/images/never-written.png")
            .await
        else {
            panic!("a missing object must be an error");
        };

        assert_eq!(
            error.error_kind,
            DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::NotFound)),
            "a missing object must map to 404, not 500"
        );
    }

    /// The sibling of the test above: a real disk failure must stay a 500, or a broken
    /// mount would quietly read as "no such image" to every caller.
    #[tokio::test]
    async fn local_get_reports_a_broken_path_as_an_internal_error() {
        let root = TempRoot::new();
        let store = LocalObjectStore::new(&root.path);
        let key = "coaching-sessions/abc/images/dir-not-file.png";

        // A directory where the object should be: readable metadata, unreadable as a file.
        std::fs::create_dir_all(root.path.join(key)).expect("directory stands in for the object");

        let Err(error) = store.get(key).await else {
            panic!("reading a directory must be an error");
        };

        assert_ne!(
            error.error_kind,
            DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::NotFound)),
            "an I/O failure that is not absence must not be reported as absence"
        );
    }

    #[test]
    fn local_presigned_get_is_unsupported() {
        let root = TempRoot::new();
        let store = LocalObjectStore::new(&root.path);

        assert_eq!(
            store
                .presigned_get("any/key.png", Duration::from_secs(60))
                .expect("presign is not an error, just unsupported"),
            None
        );
    }

    #[tokio::test]
    async fn local_store_rejects_path_traversal_keys() {
        let root = TempRoot::new();
        let store = LocalObjectStore::new(&root.path);

        let source = source_file(&[1, 2, 3]);
        for key in ["../../etc/passwd", "/etc/passwd"] {
            let put = store
                .put(key, source.path(), "image/png")
                .await
                .expect_err("traversal key must not be written");
            assert_config_error(put, &format!("put({key})"));

            let Err(get) = store.get(key).await else {
                panic!("traversal key must not be read: {key}");
            };
            assert_config_error(get, &format!("get({key})"));
        }

        assert!(
            !Path::new("/etc/passwd.content-type").exists(),
            "a rejected key must not have written a sidecar outside the root"
        );
    }

    /// The production upload path. S3 refuses a chunked `PUT`, so the streamed body has to
    /// go out with its length declared up front, and every byte has to arrive. The source
    /// spans several read chunks so the stream is really a stream.
    #[tokio::test]
    async fn spaces_put_streams_the_file_with_a_declared_length() {
        let mut server = mockito::Server::new_async().await;
        let body: Vec<u8> = (0..300 * 1024).map(|i| (i % 251) as u8).collect();
        let source = source_file(&body);

        let upload = server
            .mock(
                "PUT",
                mockito::Matcher::Regex(
                    r"^/test-bucket/coaching-sessions/abc/images/def\.png\?".into(),
                ),
            )
            .match_header("content-length", body.len().to_string().as_str())
            .match_header("content-type", "image/png")
            .match_header("transfer-encoding", mockito::Matcher::Missing)
            .match_body(body.clone())
            .with_status(200)
            .create_async()
            .await;

        let endpoint = Url::parse(&server.url()).expect("the mock server has a URL");
        let store = SpacesObjectStore {
            // Path style: a virtual-host bucket would need DNS for `test-bucket.127.0.0.1`.
            bucket: Bucket::new(endpoint, UrlStyle::Path, "test-bucket", "nyc3")
                .expect("the bucket builds"),
            credentials: Credentials::new("test-key-id", "test-secret"),
            client: reqwest::Client::new(),
        };

        store
            .put(
                "coaching-sessions/abc/images/def.png",
                source.path(),
                "image/png",
            )
            .await
            .expect("the upload succeeds");

        upload.assert_async().await;
    }

    #[test]
    fn spaces_presigned_get_signs_an_immutable_cache_url() {
        let url = spaces_store()
            .presigned_get(
                "coaching-sessions/abc/images/def.png",
                Duration::from_secs(900),
            )
            .expect("signing succeeds")
            .expect("Spaces can sign");

        assert!(url.contains("test-bucket"), "url names the bucket: {url}");
        assert!(
            url.contains("coaching-sessions/abc/images/def.png"),
            "url names the key: {url}"
        );
        assert!(url.contains("X-Amz-Signature="), "url is signed: {url}");
        assert!(
            url.contains("response-cache-control=private%2C%20max-age%3D86400%2C%20immutable"),
            "url overrides cache-control: {url}"
        );
    }

    #[test]
    fn from_config_declines_incomplete_or_unknown_backends() {
        let spaces = Config::from_args(["refactor-platform-rs", "--object-store-backend=spaces"]);
        assert!(
            from_config(&spaces).is_none(),
            "spaces without credentials must not yield a store"
        );

        let unknown = Config::from_args(["refactor-platform-rs", "--object-store-backend=gcs"]);
        assert!(
            from_config(&unknown).is_none(),
            "an unrecognized backend must not yield a store"
        );
    }
}
