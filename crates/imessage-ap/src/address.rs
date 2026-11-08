//! Handle address normalization (spec: "Contacts and identity").
//!
//! Phone numbers become E.164 with the US as the default region; emails
//! become lowercase. Anything else (short codes, business sender IDs) is kept
//! as-is. Raw values are always stored too.

/// Normalizes a raw handle ID from a backup.
pub fn normalize(raw: &str) -> String {
    let t = raw.trim();
    if t.contains('@') {
        return t.to_lowercase();
    }
    let looks_like_phone = !t.is_empty()
        && t.chars()
            .all(|c| c.is_ascii_digit() || " +-().".contains(c))
        && t.chars().any(|c| c.is_ascii_digit());
    if !looks_like_phone {
        return t.to_owned();
    }
    let digits: String = t.chars().filter(char::is_ascii_digit).collect();
    if t.starts_with('+') {
        return format!("+{digits}");
    }
    match digits.len() {
        10 => format!("+1{digits}"),
        11 if digits.starts_with('1') => format!("+{digits}"),
        // Short codes and anything else stay as plain digits.
        _ => digits,
    }
}

// Tests: crates/imessage-ap/tests/address.rs, with cases in
// fixtures/address-cases.tsv (phone numbers and emails live only in fixtures/).
