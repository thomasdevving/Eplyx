//! Opaque, sortable, URL-safe identifiers.
//!
//! Every id here is a prefix, a millisecond timestamp and 80 bits of
//! randomness, Crockford base32 encoded: `proj_01JQZ3K5R8V2X7M9N4T6W1Y0BC`.
//!
//! Sortable because the timestamp leads, which is what makes "newest first"
//! a directory listing rather than a scan of every record. Opaque because a
//! name is not an identifier: a project renamed is the same project, two teams
//! may both call theirs "Lending", and a name that became a path segment would
//! put the caller in charge of where bytes land.

use std::time::{SystemTime, UNIX_EPOCH};

/// Crockford base32, without the letters that read as digits.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

fn encode(value: u128, length: usize) -> String {
    let mut out = vec![b'0'; length];
    let mut value = value;
    for slot in out.iter_mut().rev() {
        *slot = ALPHABET[(value & 31) as usize];
        value >>= 5;
    }
    String::from_utf8(out).expect("alphabet is ascii")
}

/// The last millisecond an id was minted at.
///
/// Two ids minted inside the same millisecond would otherwise order by their
/// random tail, which is to say not at all. Newest-first history and a cursor
/// that neither repeats nor skips both rest on the ordering being real, so the
/// clock is advanced rather than reused.
static LAST_MILLIS: std::sync::Mutex<u128> = std::sync::Mutex::new(0);

fn mint(prefix: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or_default();
    let millis = {
        let mut last = LAST_MILLIS.lock().unwrap_or_else(|held| held.into_inner());
        *last = (*last + 1).max(now);
        *last
    };
    let random: u128 = u128::from(rand::random::<u64>()) << 16 | u128::from(rand::random::<u16>());
    // 48 bits of time then 80 bits of entropy: the same shape as a ULID, and
    // the reason two ids minted in the same millisecond still sort stably.
    let value = (millis & 0xFFFF_FFFF_FFFF) << 80 | (random & ((1 << 80) - 1));
    format!("{prefix}_{}", encode(value, 26))
}

pub fn project() -> String {
    mint("proj")
}

pub fn bundle() -> String {
    mint("bndl")
}

pub fn token() -> String {
    mint("tok")
}

pub fn run() -> String {
    mint("run")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_is_url_safe_and_a_valid_path_segment() {
        for id in [project(), bundle(), token()] {
            assert!(crate::storage::valid_id(&id), "{id} is not a safe segment");
            assert!(
                id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
                "{id} needs escaping in a URL"
            );
        }
    }

    #[test]
    fn ids_are_distinct_and_prefixed() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..2000 {
            let id = project();
            assert!(id.starts_with("proj_"), "{id}");
            assert_eq!(id.len(), 5 + 26, "{id}");
            assert!(seen.insert(id), "minted the same id twice");
        }
    }

    /// Time leads, so a directory listing sorts chronologically and paging
    /// through history needs no index of its own. This has to hold for ids
    /// minted back to back, not merely for ids minted milliseconds apart.
    #[test]
    fn ids_minted_back_to_back_still_sort_in_order() {
        let mut previous = project();
        for _ in 0..500 {
            let next = project();
            assert!(next > previous, "{previous} then {next}");
            previous = next;
        }
    }
}
