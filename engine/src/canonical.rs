//! Deterministic documents for new analysis artifacts: pretty JSON and one
//! trailing newline. Callers use structs/BTreeMaps and canonical input values.
//! This is not the compact, domain-separated ChangeSpec identity encoding.

use anyhow::Result;
use serde::Serialize;

pub fn document<T: Serialize + ?Sized>(value: &T) -> Result<String> {
    Ok(serde_json::to_string_pretty(value)? + "\n")
}

pub fn digest<T: Serialize + ?Sized>(value: &T) -> Result<String> {
    Ok(crate::replay::hash_bytes(document(value)?.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_and_digest_include_one_final_newline() {
        let value = serde_json::json!({"z": [true, null], "a": "9007199254740993"});
        let expected =
            "{\n  \"a\": \"9007199254740993\",\n  \"z\": [\n    true,\n    null\n  ]\n}\n";
        assert_eq!(document(&value).unwrap(), expected);
        assert_eq!(
            digest(&value).unwrap(),
            crate::replay::hash_bytes(expected.as_bytes())
        );
        assert_ne!(
            digest(&value).unwrap(),
            crate::replay::hash_bytes(expected.trim_end().as_bytes())
        );
    }
}
