//! Hex encoding for account data in fixture and report JSON.
//!
//! Hex rather than base64: fixture files are meant to be read and diffed by
//! humans, and account layouts here are small enough that density is not a
//! concern.

use serde::{Deserialize, Deserializer, Serializer};

pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from_digit((byte >> 4) as u32, 16).unwrap());
        out.push(char::from_digit((byte & 0x0f) as u32, 16).unwrap());
    }
    out
}

pub fn decode(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err(format!("hex string has odd length {}", text.len()));
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(text.len() / 2);
    for pair in bytes.chunks(2) {
        let hi = (pair[0] as char)
            .to_digit(16)
            .ok_or_else(|| format!("invalid hex digit {:?}", pair[0] as char))?;
        let lo = (pair[1] as char)
            .to_digit(16)
            .ok_or_else(|| format!("invalid hex digit {:?}", pair[1] as char))?;
        out.push(((hi << 4) | lo) as u8);
    }
    Ok(out)
}

pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&encode(bytes))
}

pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    let text = String::deserialize(deserializer)?;
    decode(&text).map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trips() {
        let cases: &[&[u8]] = &[&[], &[0x00], &[0xff, 0x01, 0x7a], &[0xde, 0xad, 0xbe, 0xef]];
        for case in cases {
            let encoded = super::encode(case);
            assert_eq!(super::decode(&encoded).unwrap(), *case);
        }
    }

    #[test]
    fn rejects_malformed() {
        assert!(super::decode("abc").is_err());
        assert!(super::decode("zz").is_err());
    }
}
