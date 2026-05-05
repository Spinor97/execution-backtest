use crate::types::*;

#[test]
fn test_side_opposite() {
    assert_eq!(Side::Buy.opposite(), Side::Sell);
    assert_eq!(Side::Sell.opposite(), Side::Buy);
}

#[test]
fn test_side_sign() {
    assert_eq!(Side::Buy.sign(), 1);
    assert_eq!(Side::Sell.sign(), -1);
}

#[test]
fn test_aggressor_as_side() {
    assert_eq!(Aggressor::Buy.as_side(), Some(Side::Buy));
    assert_eq!(Aggressor::Sell.as_side(), Some(Side::Sell));
    assert_eq!(Aggressor::Unknown.as_side(), None);
}

#[test]
fn test_instrument_price_conversion() {
    let inst = Instrument {
        symbol: "TEST".into(),
        tick_size: 0.01,
        tick_value: 0.01,
        lot_size: 1,
        price_scale: 100.0,
        maker_fee_per_unit: -0.002,
        taker_fee_per_unit: 0.003,
    };

    // Float to integer
    assert_eq!(inst.float_to_price(150.25), 15025);
    assert_eq!(inst.float_to_price(99.99), 9999);
    assert_eq!(inst.float_to_price(0.01), 1);

    // Integer to float
    assert!((inst.price_to_float(15025) - 150.25).abs() < 1e-10);
    assert!((inst.price_to_float(9999) - 99.99).abs() < 1e-10);
}

#[test]
fn test_instrument_price_roundtrip() {
    let inst = Instrument {
        price_scale: 10000.0,
        ..Default::default()
    };

    let original = 123.4567;
    let as_int = inst.float_to_price(original);
    let back = inst.price_to_float(as_int);
    assert!((back - original).abs() < 1e-4);
}

#[test]
fn test_instrument_default() {
    let inst = Instrument::default();
    assert_eq!(inst.symbol, "UNKNOWN");
    assert_eq!(inst.price_scale, 100.0);
    assert_eq!(inst.lot_size, 1);
}