use bytes::Bytes;

use crate::Result;

/// Request body compression. Bodies smaller than 1 KiB are always sent unchanged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Compression {
    /// Use zstd when compiled with the `zstd` feature (enabled by default).
    #[default]
    Auto,
    /// Require zstd support; configuration fails if the feature is disabled.
    Zstd,
    /// Send uncompressed bodies, including when using an API gateway.
    Disabled,
}

impl Compression {
    pub(crate) fn validate(self) -> Result<()> {
        if self == Self::Zstd && !cfg!(feature = "zstd") {
            return Err(crate::Error::Configuration(
                "zstd compression requires the zstd Cargo feature".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn encode(self, bytes: Bytes) -> Result<(Bytes, bool)> {
        self.validate()?;
        if self == Self::Disabled || bytes.len() < 1024 {
            return Ok((bytes, false));
        }
        #[cfg(feature = "zstd")]
        {
            let mut compressor =
                zstd::bulk::Compressor::new(3).map_err(crate::Error::Compression)?;
            // The API accepts zstd windows up to 8 MiB.
            compressor
                .set_parameter(zstd::zstd_safe::CParameter::WindowLog(23))
                .map_err(crate::Error::Compression)?;
            let compressed = compressor
                .compress(&bytes)
                .map_err(crate::Error::Compression)?;
            Ok((compressed.into(), true))
        }
        #[cfg(not(feature = "zstd"))]
        Ok((bytes, false))
    }
}
