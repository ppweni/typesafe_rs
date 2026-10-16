use std::{fmt, path::PathBuf, sync::Arc};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use serde::{Serialize, Serializer};
use tokio::sync::OnceCell;

use crate::{Error, Result};

const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;

/// An image's bytes or file path, with a stable reference for use in request content.
/// Register each image using `SystemOneRequest::asset` or `SystemOneRequest::assets`.
/// Clones share the reference and lazily cached file bytes.
#[derive(Clone)]
pub struct Image(Arc<ImageInner>);

struct ImageInner {
    reference: String,
    source: Source,
    loaded: OnceCell<Bytes>,
}

enum Source {
    Bytes(Bytes),
    File(PathBuf),
}

/// Media assets currently consist of images.
pub type Asset = Image;
/// Images keyed by their reference strings in the request.
pub type Assets = std::collections::BTreeMap<String, Asset>;

/// How to upload registered images.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AssetEncoding {
    /// Send raw files in a multipart body (the default).
    #[default]
    Multipart,
    /// Send base64 image data in the JSON body.
    Base64,
}

impl Image {
    /// Create an image from PNG or JPEG file bytes. Signatures are checked when sent;
    /// the API checks decoding and dimensions.
    pub fn from_bytes(data: impl Into<Bytes>) -> Result<Self> {
        let data = data.into();
        validate_size(data.len())?;
        Ok(Self::new(Source::Bytes(data)))
    }

    /// Load a file asynchronously on its first request; successful reads are cached.
    /// Recreate the image to send a changed file.
    pub fn from_file(path: impl Into<PathBuf>) -> Self {
        Self::new(Source::File(path.into()))
    }

    /// Decode standard padded base64, allowing ASCII whitespace but no data URL prefix.
    pub fn from_base64(data: impl AsRef<[u8]>) -> Result<Self> {
        let max_encoded = MAX_IMAGE_BYTES.div_ceil(3) * 4;
        let data: Vec<_> = data
            .as_ref()
            .iter()
            .copied()
            .filter(|b| !b.is_ascii_whitespace())
            .take(max_encoded + 1)
            .collect();
        // Bound allocations before decoding, allowing the padded final group.
        if data.len() > max_encoded {
            return Err(Error::InvalidInput("image exceeds 16 MiB".into()));
        }
        let decoded = STANDARD
            .decode(data)
            .map_err(|_| Error::InvalidInput("image must contain valid standard base64".into()))?;
        Self::from_bytes(decoded)
    }

    fn new(source: Source) -> Self {
        Self(Arc::new(ImageInner {
            reference: format!("<image:{:032x}>", fastrand::u128(..)),
            source,
            loaded: OnceCell::new(),
        }))
    }

    /// The placeholder to put anywhere in state, instructions, or criteria.
    pub fn reference(&self) -> &str {
        &self.0.reference
    }

    pub(crate) async fn bytes(&self) -> Result<Bytes> {
        match &self.0.source {
            Source::Bytes(bytes) => Ok(bytes.clone()),
            Source::File(path) => self
                .0
                .loaded
                .get_or_try_init(|| async {
                    // Check metadata first to avoid allocating an oversized file. Take also bounds
                    // reads if the file grows after metadata was checked.
                    use tokio::io::AsyncReadExt;
                    let file = tokio::fs::File::open(path)
                        .await
                        .map_err(Error::AssetRead)?;
                    let length = file.metadata().await.map_err(Error::AssetRead)?.len();
                    if length > MAX_IMAGE_BYTES as u64 {
                        return Err(Error::InvalidInput("image exceeds 16 MiB".into()));
                    }
                    let mut bytes = Vec::with_capacity(length as usize);
                    file.take(MAX_IMAGE_BYTES as u64 + 1)
                        .read_to_end(&mut bytes)
                        .await
                        .map_err(Error::AssetRead)?;
                    validate_size(bytes.len())?;
                    Ok(bytes.into())
                })
                .await
                .cloned(),
        }
    }
}

impl fmt::Display for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reference())
    }
}
impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Image")
            .field("reference", &self.reference())
            .finish_non_exhaustive()
    }
}
impl PartialEq for Image {
    fn eq(&self, other: &Self) -> bool {
        self.reference() == other.reference()
    }
}
impl Eq for Image {}
impl Serialize for Image {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.reference())
    }
}

pub(crate) fn format(bytes: &[u8]) -> Result<(&'static str, &'static str)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Ok(("png", "image/png"))
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Ok(("jpg", "image/jpeg"))
    } else {
        Err(Error::InvalidInput(
            "image must be a PNG or JPEG file".into(),
        ))
    }
}

fn validate_size(length: usize) -> Result<()> {
    if length == 0 || length > MAX_IMAGE_BYTES {
        return Err(Error::InvalidInput(
            "image must contain 1 byte to 16 MiB".into(),
        ));
    }
    Ok(())
}
