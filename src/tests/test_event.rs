use crate::event::*;
use crate::types::*;

#[test]
fn test_quote_event_mid_price() {
    let q = QuoteEvent {
        ts: 1_000_000,
        bid_price: 100,
        bid_qty: 50,
        ask_price: 102,
        ask_qty: 30,
    };
    assert_eq!(q.mid_price(), 101);
}

#[test]
fn test_quote_event_mid_price_odd_spread() {
    let q = QuoteEvent {
        ts: 1_000_000,
        bid_price: 100,
        bid_qty: 50,
        ask_price: 103,
        ask_qty: 30,
    };
    // Integer division: (100 + 103) / 2 = 101
    assert_eq!(q.mid_price(), 101);
}

#[test]
fn test_quote_event_spread() {
    let q = QuoteEvent {
        ts: 1_000_000,
        bid_price: 100,
        bid_qty: 50,
        ask_price: 105,
        ask_qty: 30,
    };
    assert_eq!(q.spread(), 5);
}

#[test]
fn test_quote_event_is_crossed() {
    let normal = QuoteEvent {
        ts: 1_000_000,
        bid_price: 100,
        bid_qty: 50,
        ask_price: 101,
        ask_qty: 30,
    };
    assert!(!normal.is_crossed());

    let crossed = QuoteEvent {
        ts: 1_000_000,
        bid_price: 102,
        bid_qty: 50,
        ask_price: 100,
        ask_qty: 30,
    };
    assert!(crossed.is_crossed());

    let locked = QuoteEvent {
        ts: 1_000_000,
        bid_price: 100,
        bid_qty: 50,
        ask_price: 100,
        ask_qty: 30,
    };
    assert!(locked.is_crossed());
}

#[test]
fn test_market_event_timestamp() {
    let quote_event = MarketEvent::Quote(QuoteEvent {
        ts: 123456,
        bid_price: 100,
        bid_qty: 10,
        ask_price: 101,
        ask_qty: 10,
    });
    assert_eq!(quote_event.timestamp(), 123456);

    let trade_event = MarketEvent::Trade(TradeEvent {
        ts: 789012,
        price: 101,
        qty: 50,
        aggressor: Aggressor::Buy,
    });
    assert_eq!(trade_event.timestamp(), 789012);
}

#[test]
fn test_market_event_type_checks() {
    let quote = MarketEvent::Quote(QuoteEvent {
        ts: 0,
        bid_price: 100,
        bid_qty: 10,
        ask_price: 101,
        ask_qty: 10,
    });
    assert!(quote.is_quote());
    assert!(!quote.is_trade());

    let trade = MarketEvent::Trade(TradeEvent {
        ts: 0,
        price: 100,
        qty: 10,
        aggressor: Aggressor::Sell,
    });
    assert!(trade.is_trade());
    assert!(!trade.is_quote());
}