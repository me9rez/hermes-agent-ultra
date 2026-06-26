//! GBK decoding and display-text sanity checks for Chinese market APIs.

use encoding_rs::GBK;

/// Decode Tencent `qt.gtimg.cn` response body (GBK, per UZI `data_sources._fetch_price_tencent_qt`).
#[must_use]
pub fn decode_tencent_qt_body(bytes: &[u8]) -> String {
    let (decoded, _, had_errors) = GBK.decode(bytes);
    if had_errors {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        decoded.into_owned()
    }
}

/// Whether a company name is safe to show in brief/HTML (reject mojibake / replacement chars).
#[must_use]
pub fn is_usable_company_name(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.contains('\u{FFFD}') {
        return false;
    }
    // GBK body misread as UTF-8 often injects Cyrillic into Chinese names (e.g. ST��԰).
    if trimmed
        .chars()
        .any(|c| ('\u{0400}'..='\u{04FF}').contains(&c))
    {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_gbk_st_changyuan() {
        // v_sh600525="1~ST长园~600525~4.95~..."
        let payload = b"1~ST\xB3\xA4\xD4\xB0~600525~4.95~4.90~";
        let mut body = Vec::from(b"v_sh600525=\"" as &[u8]);
        body.extend_from_slice(payload);
        body.extend_from_slice(b"\"");
        let decoded = decode_tencent_qt_body(&body);
        assert!(decoded.contains("ST长园"));
        assert!(!decoded.contains('\u{FFFD}'));
    }

    #[test]
    fn rejects_mojibake_name() {
        assert!(!is_usable_company_name("ST\u{FFFD}\u{FFFD}\u{04B0}"));
        assert!(is_usable_company_name("ST长园"));
        assert!(is_usable_company_name("贵州茅台"));
    }
}
