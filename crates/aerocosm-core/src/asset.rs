//! Binary assets (plan M5 D104): a short header naming the kind of asset and
//! its format version, then the value encoded with postcard and compressed
//! with zstd. Pure Rust both ways, so the web build reads them too.

use std::io::Read;

use anyhow::{Context, Result};
use ruzstd::{
    decoding::StreamingDecoder,
    encoding::{CompressionLevel, compress_to_vec},
};
use serde::{Serialize, de::DeserializeOwned};

use crate::CoreError;

/// Every asset starts with these bytes.
const MAGIC: [u8; 4] = *b"AERO";
/// Magic, kind tag and a little-endian `u32` version.
const HEADER: usize = 12;

/// What an asset holds and which format version this build reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetKind {
    /// Four bytes naming the kind, e.g. `WRLD`.
    pub tag: [u8; 4],
    /// How errors name it, e.g. "world asset".
    pub name: &'static str,
    /// The format version.
    pub version: u32,
}

/// `value` as an asset of `kind`. Compressed at ruzstd's fastest level, the
/// only one it implements (about zstd level 1).
pub fn encode<T: Serialize>(kind: AssetKind, value: &T) -> Result<Vec<u8>> {
    let raw =
        postcard::to_stdvec(value).with_context(|| format!("cannot encode the {}", kind.name))?;
    let packed = compress_to_vec(raw.as_slice(), CompressionLevel::Fastest);
    let mut bytes = Vec::with_capacity(HEADER + packed.len());
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(&kind.tag);
    bytes.extend_from_slice(&kind.version.to_le_bytes());
    bytes.extend_from_slice(&packed);
    Ok(bytes)
}

/// The value in `bytes`, an asset of `kind` in the version this build reads.
pub fn decode<T: DeserializeOwned>(kind: AssetKind, bytes: &[u8]) -> Result<T> {
    let not_one = || CoreError::NotAnAsset { file: kind.name };
    let (header, mut packed) = bytes.split_at_checked(HEADER).ok_or_else(not_one)?;
    if header[..4] != MAGIC || header[4..8] != kind.tag {
        return Err(not_one().into());
    }
    let mut version = [0; 4];
    version.copy_from_slice(&header[8..]);
    let found = u32::from_le_bytes(version);
    if found != kind.version {
        return Err(CoreError::UnsupportedFileFormat {
            file: kind.name,
            found,
            supported: kind.version,
        }
        .into());
    }
    let mut raw = Vec::with_capacity(packed.len() * 4);
    StreamingDecoder::new(&mut packed)
        .map_err(|error| anyhow::anyhow!("{error}"))
        .and_then(|mut decoder| Ok(decoder.read_to_end(&mut raw)?))
        .with_context(|| format!("cannot decompress the {}", kind.name))?;
    postcard::from_bytes(&raw).with_context(|| format!("cannot decode the {}", kind.name))
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Sample {
        name: String,
        values: Vec<u32>,
    }

    const SAMPLE: AssetKind = AssetKind {
        tag: *b"TEST",
        name: "test asset",
        version: 3,
    };

    fn sample() -> Sample {
        Sample {
            name: "Frankfurt".to_owned(),
            values: (0..2_000).map(|v| v % 7).collect(),
        }
    }

    #[test]
    fn test_assets_round_trip_and_compress() -> Result<()> {
        let bytes = encode(SAMPLE, &sample())?;
        let back: Sample = decode(SAMPLE, &bytes)?;
        assert_eq!(back, sample());
        // 2,000 repetitive numbers squeeze well below their raw size.
        assert!(bytes.len() < 500, "{} bytes", bytes.len());
        Ok(())
    }

    #[test]
    fn test_assets_of_another_kind_or_version_are_refused() -> Result<()> {
        let bytes = encode(SAMPLE, &sample())?;
        let other = AssetKind {
            tag: *b"WRLD",
            name: "world asset",
            version: 3,
        };
        let error = decode::<Sample>(other, &bytes)
            .err()
            .context("another kind")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::NotAnAsset {
                file: "world asset"
            })
        );
        let newer = AssetKind {
            version: 4,
            ..SAMPLE
        };
        let error = decode::<Sample>(newer, &bytes)
            .err()
            .context("another version")?;
        assert_eq!(
            error.downcast_ref::<CoreError>(),
            Some(&CoreError::UnsupportedFileFormat {
                file: "test asset",
                found: 3,
                supported: 4,
            })
        );
        Ok(())
    }

    #[test]
    fn test_assets_reject_garbage() -> Result<()> {
        assert!(decode::<Sample>(SAMPLE, b"").is_err());
        assert!(decode::<Sample>(SAMPLE, b"AERO").is_err());
        let mut bytes = encode(SAMPLE, &sample())?;
        let last = bytes.len() - 1;
        bytes.truncate(last / 2);
        assert!(decode::<Sample>(SAMPLE, &bytes).is_err());
        Ok(())
    }
}
