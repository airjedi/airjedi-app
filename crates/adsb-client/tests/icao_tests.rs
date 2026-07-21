use adsb_client::Icao;

#[test]
fn from_message_extracts_bytes_1_2_3() {
    // DF byte + ICAO A1B2C3 + padding
    let data = [0x8D, 0xA1, 0xB2, 0xC3, 0x00, 0x00, 0x00];
    let icao = Icao::from_message(&data);
    assert_eq!(icao, Icao(0xA1B2C3));
}

#[test]
fn from_parity_masks_24_bits() {
    let icao = Icao::from_parity(0x00A1B2C3);
    assert_eq!(icao, Icao(0xA1B2C3));

    let icao_overflow = Icao::from_parity(0xFFA1B2C3);
    assert_eq!(icao_overflow, Icao(0xA1B2C3));
}

#[test]
fn from_hex_parses_uppercase_and_lowercase() {
    assert_eq!(Icao::from_hex("A1B2C3"), Some(Icao(0xA1B2C3)));
    assert_eq!(Icao::from_hex("a1b2c3"), Some(Icao(0xA1B2C3)));
    assert_eq!(Icao::from_hex("ZZZZZZ"), None);
}

#[test]
fn display_formats_as_six_digit_uppercase_hex() {
    assert_eq!(format!("{}", Icao(0xA1B2C3)), "A1B2C3");
    assert_eq!(format!("{}", Icao(0x00000F)), "00000F");
}

#[test]
fn icao_is_copy_and_hashable() {
    use std::collections::HashSet;
    let a = Icao(0xA1B2C3);
    let b = a; // Copy
    let mut set = HashSet::new();
    set.insert(a);
    assert!(set.contains(&b));
}
