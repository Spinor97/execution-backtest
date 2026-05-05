use crate::book::*;
use crate::event::*;
use crate::types::*;

fn make_quote(ts: TsMicros, bid: Price, bid_qty: Qty, ask: Price, ask_qty: Qty) -> QuoteEvent {
    QuoteEvent {
        ts,
        bid_price: bid,
        bid_qty,
        ask_price: ask,
        ask_qty,
    }
}

#[test]
fn test_l1book_initial_state() {
    let book = L1Book::new();
    assert_eq!(book.bbo.bid_price, 0);
    assert_eq!(book.bbo.ask_price, 0);
    assert!(!book.bbo.is_valid());
}

#[test]
fn test_l1book_on_quote_updates_bbo() {
    let mut book = L1Book::new();
    let q = make_quote(1000, 100, 50, 102, 30);
    book.on_quote(&q);

    assert_eq!(book.bbo.bid_price, 100);
    assert_eq!(book.bbo.bid_qty, 50);
    assert_eq!(book.bbo.ask_price, 102);
    assert_eq!(book.bbo.ask_qty, 30);
    assert_eq!(book.bbo.last_update_ts, 1000);
    assert!(book.bbo.is_valid());
}

#[test]
fn test_l1book_detects_price_change() {
    let mut book = L1Book::new();

    let q1 = make_quote(1000, 100, 50, 102, 30);
    let change1 = book.on_quote(&q1);
    // First update — changes from 0 to 100/102
    assert!(change1.bid_price_changed);
    assert!(change1.ask_price_changed);

    // Same prices, different qty
    let q2 = make_quote(2000, 100, 60, 102, 40);
    let change2 = book.on_quote(&q2);
    assert!(!change2.bid_price_changed);
    assert!(!change2.ask_price_changed);
    assert!(change2.bid_qty_changed);
    assert!(change2.ask_qty_changed);

    // Price change on bid only
    let q3 = make_quote(3000, 101, 60, 102, 40);
    let change3 = book.on_quote(&q3);
    assert!(change3.bid_price_changed);
    assert!(!change3.ask_price_changed);
}

#[test]
fn test_l1book_prev_bbo() {
    let mut book = L1Book::new();

    let q1 = make_quote(1000, 100, 50, 102, 30);
    book.on_quote(&q1);

    let q2 = make_quote(2000, 101, 60, 103, 40);
    book.on_quote(&q2);

    assert_eq!(book.prev_bbo().bid_price, 100);
    assert_eq!(book.prev_bbo().ask_price, 102);
    assert_eq!(book.bbo.bid_price, 101);
    assert_eq!(book.bbo.ask_price, 103);
}

#[test]
fn test_bbo_mid_price() {
    let bbo = Bbo {
        bid_price: 100,
        bid_qty: 50,
        ask_price: 104,
        ask_qty: 30,
        last_update_ts: 0,
    };
    assert_eq!(bbo.mid_price(), 102);
}

#[test]
fn test_bbo_spread() {
    let bbo = Bbo {
        bid_price: 100,
        bid_qty: 50,
        ask_price: 103,
        ask_qty: 30,
        last_update_ts: 0,
    };
    assert_eq!(bbo.spread(), 3);
}

#[test]
fn test_bbo_market_prices() {
    let bbo = Bbo {
        bid_price: 100,
        bid_qty: 50,
        ask_price: 102,
        ask_qty: 30,
        last_update_ts: 0,
    };
    assert_eq!(bbo.market_buy_price(), 102);
    assert_eq!(bbo.market_sell_price(), 100);
}

#[test]
fn test_bbo_qty_at_best() {
    let bbo = Bbo {
        bid_price: 100,
        bid_qty: 50,
        ask_price: 102,
        ask_qty: 30,
        last_update_ts: 0,
    };
    assert_eq!(bbo.qty_at_best(Side::Buy), 50);
    assert_eq!(bbo.qty_at_best(Side::Sell), 30);
}

#[test]
fn test_bbo_best_price() {
    let bbo = Bbo {
        bid_price: 100,
        bid_qty: 50,
        ask_price: 102,
        ask_qty: 30,
        last_update_ts: 0,
    };
    assert_eq!(bbo.best_price(Side::Buy), 100);
    assert_eq!(bbo.best_price(Side::Sell), 102);
}

#[test]
fn test_queue_consumption_buy_aggressor() {
    let mut book = L1Book::new();
    let q = make_quote(1000, 100, 200, 102, 150);
    book.on_quote(&q);

    let trade = TradeEvent {
        ts: 1100,
        price: 102,
        qty: 50,
        aggressor: Aggressor::Buy,
    };

    let consumed = book.estimate_queue_consumed(&trade);
    assert_eq!(consumed.side, Side::Sell);
    assert_eq!(consumed.price, 102);
    assert_eq!(consumed.qty_consumed, 50);
}

#[test]
fn test_queue_consumption_sell_aggressor() {
    let mut book = L1Book::new();
    let q = make_quote(1000, 100, 200, 102, 150);
    book.on_quote(&q);

    let trade = TradeEvent {
        ts: 1100,
        price: 100,
        qty: 75,
        aggressor: Aggressor::Sell,
    };

    let consumed = book.estimate_queue_consumed(&trade);
    assert_eq!(consumed.side, Side::Buy);
    assert_eq!(consumed.price, 100);
    assert_eq!(consumed.qty_consumed, 75);
}

#[test]
fn test_queue_consumption_unknown_aggressor_at_ask() {
    let mut book = L1Book::new();
    let q = make_quote(1000, 100, 200, 102, 150);
    book.on_quote(&q);

    let trade = TradeEvent {
        ts: 1100,
        price: 102,
        qty: 50,
        aggressor: Aggressor::Unknown,
    };

    let consumed = book.estimate_queue_consumed(&trade);
    assert_eq!(consumed.side, Side::Sell);
    assert_eq!(consumed.qty_consumed, 50);
}

#[test]
fn test_queue_consumption_trade_not_at_bbo() {
    let mut book = L1Book::new();
    let q = make_quote(1000, 100, 200, 102, 150);
    book.on_quote(&q);

    // Trade at a price that's not the bid or ask
    let trade = TradeEvent {
        ts: 1100,
        price: 105,
        qty: 50,
        aggressor: Aggressor::Buy,
    };

    let consumed = book.estimate_queue_consumed(&trade);
    assert_eq!(consumed.qty_consumed, 0);
}

#[test]
fn test_bbo_change_any_price_change() {
    let change = BboChange {
        bid_price_changed: true,
        ask_price_changed: false,
        bid_qty_changed: false,
        ask_qty_changed: false,
    };
    assert!(change.any_price_change());

    let no_change = BboChange {
        bid_price_changed: false,
        ask_price_changed: false,
        bid_qty_changed: true,
        ask_qty_changed: true,
    };
    assert!(!no_change.any_price_change());
    assert!(no_change.any_change());
}