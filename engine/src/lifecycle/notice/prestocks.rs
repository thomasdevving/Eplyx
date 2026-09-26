//! Deterministic adapter for the one captured official SpaceX page. Machine
//! attributes supply identities; the rendered notice supplies its bounded grammar.
use super::*;
use anyhow::{bail, Context};
use chrono::{NaiveDateTime, TimeZone};
use std::collections::BTreeSet;
pub struct CapturedIssuerNotice;
fn location(source: &CapturedSourceDocument, start: usize, end: usize) -> SourceLocation {
    SourceLocation {
        source_digest: source.content_sha256.clone(),
        pointer: format!("/raw_content/utf8-bytes/{start}/{end}"),
        byte_range: Some([start, end]),
        exact_text: Some(source.raw_content[start..end].into()),
    }
}
/// Read quoted attributes without depending on attribute order or CSS classes.
fn attribute(tag: &str, name: &str) -> Option<(String, usize, usize)> {
    let mut i = tag.find(char::is_whitespace)?;
    let b = tag.as_bytes();
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'>' {
            i += 1;
        }
        if i == start {
            break;
        }
        let key = &tag[start..i];
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if b.get(i) != Some(&b'=') {
            continue;
        }
        i += 1;
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let quote = *b.get(i)?;
        if quote != b'\'' && quote != b'"' {
            return None;
        }
        i += 1;
        let value_start = i;
        while i < b.len() && b[i] != quote {
            i += 1;
        }
        if i == b.len() {
            return None;
        }
        if key == name {
            return Some((tag[value_start..i].into(), value_start, i));
        }
        i += 1;
    }
    None
}
fn tags<'a>(html: &'a str, name: &str) -> Vec<(usize, &'a str)> {
    let marker = format!("<{name}");
    html.match_indices(&marker)
        .filter_map(|(start, _)| {
            let boundary = html.as_bytes().get(start + marker.len())?;
            if !boundary.is_ascii_whitespace() && *boundary != b'>' {
                return None;
            }
            let end = start + html[start..].find('>')? + 1;
            Some((start, &html[start..end]))
        })
        .collect()
}
fn text(html: &str) -> Result<String> {
    let mut out = String::new();
    let mut i = 0;
    while i < html.len() {
        if html[i..].starts_with("<!--") {
            i += html[i..].find("-->").context("unterminated comment")? + 3;
        } else if html[i..].starts_with('<') {
            i += html[i..].find('>').context("unterminated tag")? + 1;
        } else {
            let c = html[i..].chars().next().unwrap();
            out.push(c);
            i += c.len_utf8();
        }
    }
    Ok(out.split_whitespace().collect::<Vec<_>>().join(" "))
}
fn unknown_identity(mint: &str) -> AssetIdentity {
    AssetIdentity {
        asserted_mint: mint.into(),
        observed_mint: None,
        status: IdentityStatus::Unknown,
        slot: None,
        observed_name: None,
        observed_symbol: None,
        provenance: vec![],
    }
}
impl LifecycleEventSource for CapturedIssuerNotice {
    fn normalize(&self, source: &CapturedSourceDocument) -> Result<NormalizedLifecycleEvent> {
        source.validate()?;
        ensure!(
            source.source_url == "https://prestocks.com/spacex"
                && source.source_type == "OfficialIssuerNotice",
            "unsupported issuer source"
        );
        // Next.js script data does not contain the notice's fields. Restrict extraction
        // to rendered HTML before scripts, excluding application/script strings.
        let mut rendered = source.raw_content.as_bytes().to_vec();
        let mut cursor = 0;
        while let Some(relative) = source.raw_content[cursor..].find("<script") {
            let a = cursor + relative;
            let b = a
                + source.raw_content[a..]
                    .find("</script>")
                    .context("unterminated script")?
                + 9;
            rendered[a..b].fill(b' ');
            cursor = b;
        }
        let rendered = String::from_utf8(rendered)?;
        let html = rendered.as_str();
        let mut issuer_fields = Vec::new();
        for (start, tag) in tags(html, "meta") {
            if attribute(tag, "property").is_some_and(|v| v.0 == "og:site_name") {
                let (v, a, b) = attribute(tag, "content").context("missing issuer metadata")?;
                issuer_fields.push((v, location(source, start + a, start + b)));
            }
        }
        ensure!(
            issuer_fields.len() == 1,
            "missing or ambiguous issuer metadata"
        );
        let (issuer, issuer_loc) = issuer_fields.remove(0);
        ensure!(issuer == source.issuer_label, "issuer label mismatch");
        let mut notices = Vec::new();
        let mut public_assertion = None;
        for (start, tag) in tags(html, "span") {
            let inner = start + tag.len();
            let end = inner
                + html[inner..]
                    .find("</span>")
                    .context("unterminated notice span")?;
            let plain = text(&html[inner..end])?;
            if plain.contains(" tokens must be swapped into") {
                notices.push((inner, end, plain.clone()));
            }
            if plain == "⚠️ SpaceX has gone public!" {
                public_assertion = Some((plain, location(source, inner, end)));
            }
        }
        ensure!(notices.len() == 1, "missing or ambiguous successor notice");
        let (start, end, plain) = notices.remove(0);
        let banner_loc = location(source, start, end);
        let source_name = plain
            .split_once(" tokens must be swapped into ")
            .context("unsupported notice grammar")?
            .0
            .to_owned();
        let banner = &html[start..end];
        let mut successor = None;
        for (offset, tag) in tags(banner, "a") {
            if let Some((href, a, b)) = attribute(tag, "href") {
                if let Some(mint) = href.strip_prefix("https://solscan.io/token/") {
                    ensure!(successor.is_none(), "ambiguous successor mint");
                    let symbol_start = start + offset + tag.len();
                    let symbol_end = symbol_start
                        + html[symbol_start..]
                            .find("</a>")
                            .context("missing successor symbol")?;
                    let symbol = text(&html[symbol_start..symbol_end])?;
                    ensure!(
                        symbol.starts_with('$') && symbol.len() > 1,
                        "missing successor symbol"
                    );
                    successor = Some((
                        mint.to_owned(),
                        location(source, start + offset + a, start + offset + b),
                        symbol[1..].to_owned(),
                        location(source, symbol_start, symbol_end),
                    ));
                }
            }
        }
        let (successor_mint, successor_loc, symbol, symbol_loc) =
            successor.context("missing linked successor mint")?;
        let suffix = plain
            .split_once("or any other token before ")
            .context("missing alternate destination or deadline")?
            .1;
        let wording = suffix
            .split_once(", or they will expire worthless.")
            .context("missing expiration assertion")?
            .0
            .to_owned();
        let (clock, date) = wording
            .split_once(" UTC on ")
            .context("unsupported deadline grammar")?;
        let dt = NaiveDateTime::parse_from_str(&format!("{date} {clock}"), "%d %B %Y %I:%M%P")
            .context("invalid issuer deadline")?;
        let deadline = Utc.from_utc_datetime(&dt);
        // Source asset links must occur outside and before the notice. Repeated logo
        // links may name the same mint; distinct mints are an explicit ambiguity.
        let mut source_mints = BTreeSet::new();
        let mut source_loc = None;
        for (offset, tag) in tags(&html[..start], "a") {
            if let Some((href, a, b)) = attribute(tag, "href") {
                if let Some(mint) = href.strip_prefix("https://solscan.io/token/") {
                    source_mints.insert(mint.to_owned());
                    source_loc.get_or_insert(location(source, offset + a, offset + b));
                }
            }
        }
        if source_mints.len() != 1 {
            bail!("missing or ambiguous source mint");
        }
        let source_mint = source_mints.into_iter().next().unwrap();
        for mint in [&source_mint, &successor_mint] {
            let _: solana_address::Address =
                mint.parse().context("invalid published mint address")?;
        }
        let mut assertions = BTreeMap::new();
        assertions.insert(
            "transition_and_expiry".into(),
            Field::new(plain, ProvenanceClass::IssuerAsserted, banner_loc.clone()),
        );
        if let Some((assertion, loc)) = public_assertion {
            assertions.insert(
                "public_listing".into(),
                Field::new(assertion, ProvenanceClass::IssuerAsserted, loc),
            );
        }
        let document_sha = digest(source)?;
        let mut field_provenance = BTreeMap::new();
        for (field, pointer, class) in [
            (
                "/schema_version",
                "/schema_version",
                ProvenanceClass::Derived,
            ),
            ("/event_id", "/content_sha256", ProvenanceClass::Derived),
            ("/adapter_version", "/source_type", ProvenanceClass::Derived),
            ("/source_url", "/source_url", ProvenanceClass::Derived),
            (
                "/source_content_sha256",
                "/content_sha256",
                ProvenanceClass::Derived,
            ),
            ("/captured_document_sha256", "", ProvenanceClass::Derived),
            ("/source_identity", "/raw_content", ProvenanceClass::Unknown),
            (
                "/successor_identity",
                "/raw_content",
                ProvenanceClass::Unknown,
            ),
        ] {
            field_provenance.insert(
                field.into(),
                ScenarioFieldBinding {
                    classifications: vec![class],
                    provenance: vec![SourceLocation {
                        source_digest: document_sha.clone(),
                        pointer: pointer.into(),
                        byte_range: None,
                        exact_text: None,
                    }],
                },
            );
        }
        Ok(NormalizedLifecycleEvent {
            schema_version: 1,
            event_id: format!(
                "issuer-successor-transition-{}",
                &source.content_sha256[..16]
            ),
            adapter_version: "prestocks-rendered-notice-v1".into(),
            source_url: source.source_url.clone(),
            source_content_sha256: source.content_sha256.clone(),
            captured_document_sha256: document_sha,
            field_provenance,
            issuer: Field::new(issuer, ProvenanceClass::IssuerAsserted, issuer_loc),
            event_type: Field::new(
                LifecycleEventType::SuccessorTransition,
                ProvenanceClass::Derived,
                banner_loc.clone(),
            ),
            source_asset: EventAsset {
                name: Field::new(
                    Some(source_name),
                    ProvenanceClass::IssuerAsserted,
                    banner_loc.clone(),
                ),
                symbol: Field::new(None, ProvenanceClass::Unknown, banner_loc.clone()),
                issuer_asserted_mint: Field::new(
                    source_mint.clone(),
                    ProvenanceClass::IssuerAsserted,
                    source_loc.unwrap(),
                ),
            },
            successor_asset: EventAsset {
                name: Field::new(None, ProvenanceClass::Unknown, banner_loc.clone()),
                symbol: Field::new(Some(symbol), ProvenanceClass::IssuerAsserted, symbol_loc),
                issuer_asserted_mint: Field::new(
                    successor_mint.clone(),
                    ProvenanceClass::IssuerAsserted,
                    successor_loc,
                ),
            },
            effective_at: Field::new(None, ProvenanceClass::Unknown, banner_loc.clone()),
            deadline: Field::new(
                deadline,
                ProvenanceClass::IssuerAsserted,
                banner_loc.clone(),
            ),
            deadline_wording: Field::new(
                wording,
                ProvenanceClass::IssuerAsserted,
                banner_loc.clone(),
            ),
            alternate_destination: Field::new(
                "any other token".into(),
                ProvenanceClass::IssuerAsserted,
                banner_loc.clone(),
            ),
            conversion_ratio: Field::new(None, ProvenanceClass::Unknown, banner_loc.clone()),
            official_mechanism: Field::new(
                MechanismType::Unknown,
                ProvenanceClass::Unknown,
                banner_loc.clone(),
            ),
            official_execution_status: Field::new(
                PathStatus::NotTested,
                ProvenanceClass::Unknown,
                banner_loc,
            ),
            issuer_assertions: assertions,
            source_identity: unknown_identity(&source_mint),
            successor_identity: unknown_identity(&successor_mint),
        })
    }
}
