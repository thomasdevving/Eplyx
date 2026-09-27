//! What may leave the machine. Synced artifacts are exact engine bytes, so they
//! are never rewritten: any artifact text that looks like it carries a URL, an
//! absolute local path or a credential is refused before upload and again by
//! the server. Leaky reproduction records are refused too; stored bytes never change.
use anyhow::{bail, Result};

/// Substrings that mark an absolute local path on the platforms Eplyx ships.
const PATH_MARKERS: &[&str] = &[
    "/Users/",
    "/home/",
    "/root/",
    "/private/",
    "/var/folders/",
    "/tmp/",
    "/Volumes/",
    "/mnt/",
    "/opt/",
    "/etc/",
    "\\\\?\\",
    "\\Users\\",
    "\\\\Users\\\\",
];

/// Substrings that mark credentials or credential-bearing configuration.
const SECRET_MARKERS: &[&str] = &[
    "api-key",
    "api_key",
    "apikey",
    "access_token",
    "eplyx_u_",
    "eplyx_ci_",
    "eplyx_proj_",
    "eplyx_s_",
    "PRIVATE KEY",
    "SOLANA_RPC_URL",
    "EPLYX_TOKEN",
];

/// Refuse text that could carry a URL (any scheme), an absolute path, a drive
/// letter path or a credential marker. `what` names the artifact in errors.
pub fn scan(what: &str, text: &str) -> Result<()> {
    if text.contains("://") {
        bail!("{what} contains a URL; Eplyx never uploads provider or other URLs");
    }
    if let Some(marker) = PATH_MARKERS.iter().find(|m| text.contains(**m)) {
        bail!("{what} contains an absolute local path ({marker}…); it stays local");
    }
    if has_drive_path(text) {
        bail!("{what} contains a Windows drive path; it stays local");
    }
    if let Some(marker) = SECRET_MARKERS
        .iter()
        .find(|m| text.to_ascii_lowercase().contains(&m.to_ascii_lowercase()))
    {
        bail!("{what} contains a credential marker ({marker}); it stays local");
    }
    Ok(())
}

/// Refuse text containing any of the caller's known secret values, such as the
/// configured RPC URL or the Eplyx access token. Empty values are ignored.
pub fn scan_secrets(what: &str, text: &str, secrets: &[String]) -> Result<()> {
    for secret in secrets.iter().filter(|s| s.trim().len() >= 8) {
        if text.contains(secret.as_str()) {
            bail!("{what} contains a local secret value; nothing was uploaded");
        }
    }
    Ok(())
}

fn has_drive_path(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.windows(3).enumerate().any(|(position, w)| {
        w[0].is_ascii_alphabetic()
            && w[1] == b':'
            && (w[2] == b'\\' || (w[2] == b'/' && bytes.get(position + 3) != Some(&b'/')))
            && (position == 0 || !bytes[position - 1].is_ascii_alphanumeric())
    })
}

/// JSON escapes cannot conceal a URL, machine path or credential from the scanner.
pub fn scan_json(what: &str, value: &serde_json::Value) -> Result<()> {
    match value {
        serde_json::Value::String(s) => scan(what, s),
        serde_json::Value::Array(items) => {
            for value in items {
                scan_json(what, value)?;
            }
            Ok(())
        }
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                scan(what, key)?;
                scan_json(what, value)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaped_json_cannot_hide_urls_or_credentials() {
        for text in [
            r#"{"x":"https\u003a\u002f\u002fprovider.invalid"}"#,
            r#"{"access\u005ftoken":"example"}"#,
            r#"{"x":["C\u003a\u005cwork"]}"#,
        ] {
            let value: serde_json::Value = serde_json::from_str(text).unwrap();
            assert!(scan_json("document", &value).is_err());
        }
    }
    #[test]
    fn urls_paths_and_credentials_are_refused() {
        for text in [
            "https://provider.invalid/?api-key=example",
            "wss://provider.example",
            "/Users/dev/project/.eplyx",
            "/home/runner/work/app",
            "C:\\\\Users\\\\dev",
            "D:\\build",
            "C:/work/app",
            "eplyx_u_abcdefghijk",
            "set SOLANA_RPC_URL",
        ] {
            assert!(scan("artifact", text).is_err(), "{text}");
        }
        for text in [
            "{\"run_id\":\"run_1\",\"sourceMint\":\"PreANxuXjsy2pvisWWMNB6YaJNzr7681wJJr2rHsfTh\"}",
            "ratio 1:2 floor",
            "Custom(13): insufficient funds",
            "runs/run_x/package",
            "~/work/app",
        ] {
            assert!(scan("artifact", text).is_ok(), "{text}");
        }
    }

    #[test]
    fn known_secret_values_are_refused() {
        let secrets = vec!["https://rpc.example/key-123456".to_owned(), String::new()];
        assert!(scan_secrets("doc", "x https://rpc.example/key-123456 y", &secrets).is_err());
        assert!(scan_secrets("doc", "nothing here", &secrets).is_ok());
    }

    #[test]
    fn leaky_free_text_is_refused_without_rewriting() {
        let text = "missing /home/dev/app/.eplyx/runs/run_a at https://rpc.example/k";
        let original = text.to_owned();
        assert!(scan("error", text).is_err());
        assert_eq!(text, original);
    }
}
