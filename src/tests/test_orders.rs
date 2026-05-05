use crate::order::*;
use crate::types::*;

#[test]
fn test_order_new_limit() {
    let order = Order::new_limit(1, Side::Buy, 15025, 100, TimeInForce::Gtc, 1_000_000);

    assert_eq!(order.id, 1);
    assert_eq!(order.side, Side::Buy);
    assert_eq!(order.price, 15025);
    assert_eq!(order.qty, 100);
    assert_eq!(order.filled_qty, 0);
    assert_eq!(order.state, OrderState::Pending);
    assert_eq!(order.remaining_qty(), 100);
    assert_eq!(order.queue_ahead, 0);
}

#[test]
fn test_order_remaining_qty() {
    let mut order = Order::new_limit(1, Side::Buy, 100, 500, TimeInForce::Gtc, 0);
    assert_eq!(order.remaining_qty(), 500);

    order.filled_qty = 200;
    assert_eq!(order.remaining_qty(), 300);

    order.filled_qty = 500;
    assert_eq!(order.remaining_qty(), 0);
}

#[test]
fn test_order_is_active() {
    let mut order = Order::new_limit(1, Side::Buy, 100, 500, TimeInForce::Gtc, 0);

    order.state = OrderState::Pending;
    assert!(!order.is_active());

    order.state = OrderState::Active;
    assert!(order.is_active());

    order.state = OrderState::PartialFill;
    assert!(order.is_active());

    order.state = OrderState::Filled;
    assert!(!order.is_active());

    order.state = OrderState::Cancelled;
    assert!(!order.is_active());

    order.state = OrderState::Rejected;
    assert!(!order.is_active());
}

#[test]
fn test_order_is_terminal() {
    let mut order = Order::new_limit(1, Side::Buy, 100, 500, TimeInForce::Gtc, 0);

    order.state = OrderState::Active;
    assert!(!order.is_terminal());

    order.state = OrderState::PartialFill;
    assert!(!order.is_terminal());

    order.state = OrderState::Filled;
    assert!(order.is_terminal());

    order.state = OrderState::Cancelled;
    assert!(order.is_terminal());

    order.state = OrderState::Rejected;
    assert!(order.is_terminal());
}

#[test]
fn test_fill_signed_qty() {
    let buy_fill = Fill {
        order_id: 1,
        fill_ts: 1000,
        price: 100,
        qty: 50,
        side: Side::Buy,
        is_maker: true,
        fee: 0.0,
    };
    assert_eq!(buy_fill.signed_qty(), 50);

    let sell_fill = Fill {
        order_id: 2,
        fill_ts: 2000,
        price: 101,
        qty: 30,
        side: Side::Sell,
        is_maker: false,
        fee: 0.0,
    };
    assert_eq!(sell_fill.signed_qty(), -30);
}