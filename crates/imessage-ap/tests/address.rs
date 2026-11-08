//! Address normalization, with cases from fixtures/address-cases.tsv.

use imessage_ap::address::normalize;

const CASES: &str = include_str!("../../../fixtures/address-cases.tsv");

#[test]
fn normalizes_every_fixture_case() {
    let mut n = 0;
    for line in CASES
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let (raw, expected) = line.split_once('\t').expect("raw<TAB>normalized");
        assert_eq!(normalize(raw), expected, "{raw:?}");
        n += 1;
    }
    assert!(n >= 10);
}
