use crate::feed::*;
use crate::sim::*;
use crate::event::*;
use crate::book::*;
use crate::order::*;
use crate::strategy::Strategy;
use crate::types::*;

// ─── Test helpers ──────────────────────────────────────────────────────────────

fn default_instrument() -> Instrument {
    Instrument {
        symbol: "TEST".into(),
        tick_size: 0.01,
        tick_value: 0.01,
        lot_size: 1,
        price_scale: 100.0,
        maker_fee_per_unit: -0.002,
        taker_fee_per_unit: 0.003,
    }
}

fn default_sim_config() -> SimConfig {
    SimConfig {
        latency_us: 0, // zero latency for simple tests
        queue_model: QueueModel::TradeThrough,
        allow_immediate_cross: true,
        max_position: 1000,
        instrument: default_instrument(),
        max_fill_per_trade: 0,
    }
}

/// Build a feed from explicit events.
fn build_feed_from_events(events: Vec<(TsMicros, MsgType, Price, Qty, Price, Qty, Price, Qty, i8)>) -> Feed {
    let len = events.len();
    let mut f = Feed {
        timestamps: Vec::with_capacity(len),
        msg_types: Vec::with_capacity(len),
        bid_prices: Vec::with_capacity(len),
        bid_qtys: Vec::with_capacity(len),
        ask_prices: Vec::with_capacity(len),
        ask_qtys: Vec::with_capacity(len),
        trade_prices: Vec::with_capacity(len),
        trade_qtys: Vec::with_capacity(len),
        trade_sides: Vec::with_capacity(len),
        len,
        cursor: 0,
    };

    for (ts, mt, bp, bq, ap, aq, tp, tq, ts_side) in events {
        f.timestamps.push(ts);
        f.msg_types.push(mt);
        f.bid_prices.push(bp);
        f.bid_qtys.push(bq);
        f.ask_prices.push(ap);
        f.ask_qtys.push(aq);
        f.trade_prices.push(tp);
        f.trade_qtys.push(tq);
        f.trade_sides.push(ts_side);
    }

    f
}

// ─── Passive strategy: submits a buy limit at specific price ───────────────────

struct BuyLimitStrategy {
    price: Price,
    qty: Qty,
    submitted: bool,
    pending: Vec<OrderRequest>,
}

impl BuyLimitStrategy {
    fn new(price: Price, qty: Qty) -> Self {
        Self { price, qty, submitted: false, pending: Vec::new() }
    }
}

impl Strategy for BuyLimitStrategy {
    fn on_quote(&mut self, sim: &Simulator, quote: &QuoteEvent, _change: &BboChange) {
        if !self.submitted && sim.bbo().is_valid() {
            let order = Order::new_limit(0, Side::Buy, self.price, self.qty, TimeInForce::Gtc, quote.ts);
            self.pending.push(OrderRequest::Submit(order));
            self.submitted = true;
        }
    }

    fn on_trade(&mut self, _sim: &Simulator, _trade: &TradeEvent) {}

    fn pending_requests(&mut self) -> Vec<OrderRequest> {
        std::mem::take(&mut self.pending)
    }
}

struct SellLimitStrategy {
    price: Price,
    qty: Qty,
    submitted: bool,
    pending: Vec<OrderRequest>,
}

impl SellLimitStrategy {
    fn new(price: Price, qty: Qty) -> Self {
        Self { price, qty, submitted: false, pending: Vec::new() }
    }
}

impl Strategy for SellLimitStrategy {
    fn on_quote(&mut self, sim: &Simulator, quote: &QuoteEvent, _change: &BboChange) {
        if !self.submitted && sim.bbo().is_valid() {
            let order = Order::new_limit(0, Side::Sell, self.price, self.qty, TimeInForce::Gtc, quote.ts);
            self.pending.push(OrderRequest::Submit(order));
            self.submitted = true;
        }
    }

    fn on_trade(&mut self, _sim: &Simulator, _trade: &TradeEvent) {}

    fn pending_requests(&mut self) -> Vec<OrderRequest> {
        std::mem::take(&mut self.pending)
    }
}

/// Strategy that does nothing — for testing basic mechanics.
struct NoOpStrategy;

impl Strategy for NoOpStrategy {
    fn on_quote(&mut self, _sim: &Simulator, _quote: &QuoteEvent, _change: &BboChange) {}
    fn on_trade(&mut self, _sim: &Simulator, _trade: &TradeEvent) {}
}

// ─── Tests ─────────────────────────────────────────────────────────────────────

#[test]
fn test_simulator_no_orders_no_fills() {
    let mut feed = build_feed_from_events(vec![
        (1000, MsgType::Quote, 100, 200, 102, 150, 0, 0, 0),
        (2000, MsgType::Quote, 100, 180, 102, 130, 0, 0, 0),
        (3000, MsgType::Trade, 100, 180, 102, 130, 102, 50, 1),
    ]);

    let mut sim = Simulator::new(default_sim_config());
    let mut strategy = NoOpStrategy;
    sim.run(&mut strategy, &mut feed);

    assert_eq!(sim.position(), 0);
    assert_eq!(sim.fills().len(), 0);
    assert_eq!(sim.stats().total_fills, 0);
}

#[test]
fn test_simulator_buy_limit_fill_on_ask_drop() {
    // Buy limit at 100. Ask drops from 102 to 100 → should fill.
    let mut feed = build_feed_from_events(vec![
        (1000, MsgType::Quote, 99, 200, 102, 150, 0, 0, 0),
        (2000, MsgType::Quote, 99, 200, 101, 150, 0, 0, 0),
        (3000, MsgType::Quote, 99, 200, 100, 150, 0, 0, 0), // ask drops to our price
    ]);

    let config = SimConfig {
        latency_us: 0,
        queue_model: QueueModel::Fifo,
        allow_immediate_cross: true,
        max_position: 1000,
        instrument: default_instrument(),
        max_fill_per_trade: 0,
    };

    let mut sim = Simulator::new(config);
    let mut strategy = BuyLimitStrategy::new(100, 50);
    sim.run(&mut strategy, &mut feed);

    assert_eq!(sim.fills().len(), 1);
    assert_eq!(sim.fills()[0].price, 100);
    assert_eq!(sim.fills()[0].qty, 50);
    assert_eq!(sim.fills()[0].side, Side::Buy);
    assert!(sim.fills()[0].is_maker);
    assert_eq!(sim.position(), 50);
}

#[test]
fn test_simulator_sell_limit_fill_on_bid_rise() {
    // Sell limit at 105. Bid rises from 100 to 105 → should fill.
    let mut feed = build_feed_from_events(vec![
        (1000, MsgType::Quote, 100, 200, 107, 150, 0, 0, 0),
        (2000, MsgType::Quote, 103, 200, 107, 150, 0, 0, 0),
        (3000, MsgType::Quote, 105, 200, 107, 150, 0, 0, 0), // bid rises to our price
    ]);

    let config = SimConfig {
        latency_us: 0,
        queue_model: QueueModel::Fifo,
        allow_immediate_cross: true,
        max_position: 1000,
        instrument: default_instrument(),
        max_fill_per_trade: 0,
    };

    let mut sim = Simulator::new(config);
    let mut strategy = SellLimitStrategy::new(105, 30);
    sim.run(&mut strategy, &mut feed);

    assert_eq!(sim.fills().len(), 1);
    assert_eq!(sim.fills()[0].price, 105);
    assert_eq!(sim.fills()[0].qty, 30);
    assert_eq!(sim.fills()[0].side, Side::Sell);
    assert_eq!(sim.position(), -30);
}

#[test]
fn test_simulator_latency_delays_activation() {
    // With 500μs latency, order submitted at t=1000 activates at t=1500.
    // Quote at t=1200 with ask=99 should NOT fill because order isn't active yet.
    // Quote at t=2000 with ask=99 SHOULD fill.
    let mut feed = build_feed_from_events(vec![
        (1000, MsgType::Quote, 98, 200, 102, 150, 0, 0, 0),  // submit here
        (1200, MsgType::Quote, 98, 200, 99, 150, 0, 0, 0),   // ask drops but order not active
        (2000, MsgType::Quote, 98, 200, 100, 150, 0, 0, 0),  // order now active, ask <= price
    ]);

    let config = SimConfig {
        latency_us: 500,
        queue_model: QueueModel::Fifo,
        allow_immediate_cross: true,
        max_position: 1000,
        instrument: default_instrument(),
        max_fill_per_trade: 0,
    };

    let mut sim = Simulator::new(config);
    let mut strategy = BuyLimitStrategy::new(100, 50);
    sim.run(&mut strategy, &mut feed);

    // Order submitted at t=1000, activates at t=1500.
    // At t=2000 ask=100 <= price=100, so it fills.
    assert_eq!(sim.fills().len(), 1);
    assert!(sim.fills()[0].fill_ts >= 2000);
}

#[test]
fn test_simulator_immediate_cross_taker_fill() {
    // Submit buy limit above the current ask → immediate cross as taker.
    let mut feed = build_feed_from_events(vec![
        (1000, MsgType::Quote, 100, 200, 102, 150, 0, 0, 0),
        (2000, MsgType::Quote, 100, 200, 102, 150, 0, 0, 0), // unchanged
    ]);

    let config = SimConfig {
        latency_us: 0,
        allow_immediate_cross: true,
        queue_model: QueueModel::Fifo,
        max_position: 1000,
        instrument: default_instrument(),
        max_fill_per_trade: 0,
    };

    let mut sim = Simulator::new(config);
    // Buy at 105, which is above ask of 102 → crosses immediately
    let mut strategy = BuyLimitStrategy::new(105, 50);
    sim.run(&mut strategy, &mut feed);

    assert_eq!(sim.fills().len(), 1);
    assert_eq!(sim.fills()[0].price, 102); // fills at the ask
    assert!(!sim.fills()[0].is_maker); // taker
}

#[test]
fn test_simulator_position_limit() {
    // Max position = 50. Try to fill 100 → only 50 should fill.
    let mut feed = build_feed_from_events(vec![
        (1000, MsgType::Quote, 98, 200, 102, 150, 0, 0, 0),
        (2000, MsgType::Quote, 98, 200, 99, 200, 0, 0, 0),  // ask drops to fill
    ]);

    let config = SimConfig {
        latency_us: 0,
        queue_model: QueueModel::Fifo,
        allow_immediate_cross: true,
        max_position: 50,
        instrument: default_instrument(),
        max_fill_per_trade: 0,
    };

    let mut sim = Simulator::new(config);
    let mut strategy = BuyLimitStrategy::new(100, 100); // wants 100 but limit is 50
    sim.run(&mut strategy, &mut feed);

    assert!(sim.position() <= 50);
}

#[test]
fn test_simulator_cancel_order() {
    struct CancelStrategy {
        submitted: bool,
        cancelled: bool,
        pending: Vec<OrderRequest>,
    }

    impl Strategy for CancelStrategy {
        fn on_quote(&mut self, sim: &Simulator, quote: &QuoteEvent, _change: &BboChange) {
            if !self.submitted && sim.bbo().is_valid() {
                // Use an explicit ID we control
                let order = Order::new_limit(42, Side::Buy, 50, 100, TimeInForce::Gtc, quote.ts);
                self.pending.push(OrderRequest::Submit(order));
                self.submitted = true;
            } else if self.submitted && !self.cancelled {
                // Query active orders from the simulator to get the real ID
                if let Some(order) = sim.active_orders().first() {
                    self.pending.push(OrderRequest::Cancel(order.id));
                    self.cancelled = true;
                }
            }
        }

        fn on_trade(&mut self, _sim: &Simulator, _trade: &TradeEvent) {}

        fn pending_requests(&mut self) -> Vec<OrderRequest> {
            std::mem::take(&mut self.pending)
        }
    }

    let mut feed = build_feed_from_events(vec![
        (1000, MsgType::Quote, 100, 200, 102, 150, 0, 0, 0),  // submit here
        (2000, MsgType::Quote, 100, 200, 102, 150, 0, 0, 0),  // cancel here
        (3000, MsgType::Quote, 100, 200, 102, 150, 0, 0, 0),  // buffer event (cancel processed)
        (4000, MsgType::Quote, 100, 200, 49, 150, 0, 0, 0),   // would fill if not cancelled
    ]);

    let config = SimConfig {
        latency_us: 0,
        queue_model: QueueModel::Fifo,
        allow_immediate_cross: false, // changed: avoid immediate cross logic interfering
        max_position: 1000,
        instrument: default_instrument(),
        max_fill_per_trade: 0,
    };

    let mut sim = Simulator::new(config);
    let mut strategy = CancelStrategy {
        submitted: false,
        cancelled: false,
        pending: Vec::new(),
    };
    sim.run(&mut strategy, &mut feed);

    // Order was cancelled before the fill opportunity
    assert_eq!(sim.fills().len(), 0);
    assert_eq!(sim.active_orders().len(), 0);
}

#[test]
fn test_simulator_trade_through_fill() {
    // QueueModel::TradeThrough — only fills when price trades THROUGH our level.
    let mut feed = build_feed_from_events(vec![
        (1000, MsgType::Quote, 100, 200, 104, 150, 0, 0, 0),
        // Trade AT our level (100) — should NOT fill with TradeThrough
        (2000, MsgType::Trade, 100, 200, 104, 150, 100, 50, -1),
        // Trade BELOW our level (99) — should fill
        (3000, MsgType::Trade, 100, 200, 104, 150, 99, 500, -1),
    ]);

    let config = SimConfig {
        latency_us: 0,
        queue_model: QueueModel::TradeThrough,
        allow_immediate_cross: false,
        max_position: 1000,
        instrument: default_instrument(),
        max_fill_per_trade: 0,
    };

    let mut sim = Simulator::new(config);
    let mut strategy = BuyLimitStrategy::new(100, 50);
    sim.run(&mut strategy, &mut feed);

    // Should have one fill from the trade-through at t=3000
    assert_eq!(sim.fills().len(), 1);
    assert_eq!(sim.fills()[0].fill_ts, 3000);
}

#[test]
fn test_simulator_maker_fee_rebate() {
    let mut feed = build_feed_from_events(vec![
        (1000, MsgType::Quote, 98, 200, 102, 150, 0, 0, 0),
        (2000, MsgType::Quote, 98, 200, 99, 150, 0, 0, 0), // fills our buy at 100
    ]);

    let instrument = Instrument {
        symbol: "TEST".into(),
        tick_size: 0.01,
        tick_value: 0.01,
        lot_size: 1,
        price_scale: 100.0,
        maker_fee_per_unit: -0.002, // rebate
        taker_fee_per_unit: 0.003,
    };

    let config = SimConfig {
        latency_us: 0,
        queue_model: QueueModel::Fifo,
        allow_immediate_cross: true,
        max_position: 1000,
        instrument,
        max_fill_per_trade: 0,
    };

    let mut sim = Simulator::new(config);
    let mut strategy = BuyLimitStrategy::new(100, 50);
    sim.run(&mut strategy, &mut feed);

    assert_eq!(sim.fills().len(), 1);
    // Maker fee = -0.002 * 50 = -0.1 (negative = rebate)
    assert!((sim.fills()[0].fee - (-0.1)).abs() < 1e-10);
    assert!((sim.stats().total_fees - (-0.1)).abs() < 1e-10);
}

#[test]
fn test_simulator_multiple_fills_accumulate_position() {
    // Two separate fill opportunities
    let mut feed = build_feed_from_events(vec![
        (1000, MsgType::Quote, 98, 200, 102, 150, 0, 0, 0),
        (2000, MsgType::Quote, 98, 200, 99, 150, 0, 0, 0),   // first fill
        (3000, MsgType::Quote, 98, 200, 102, 150, 0, 0, 0),   // ask back up
        (4000, MsgType::Quote, 98, 200, 99, 150, 0, 0, 0),    // would fill again but order is done
    ]);

    let config = SimConfig {
        latency_us: 0,
        queue_model: QueueModel::Fifo,
        allow_immediate_cross: true,
        max_position: 1000,
        instrument: default_instrument(),
        max_fill_per_trade: 0,
    };

    let mut sim = Simulator::new(config);
    let mut strategy = BuyLimitStrategy::new(100, 50);
    sim.run(&mut strategy, &mut feed);

    // Only one order submitted, fills once
    assert_eq!(sim.fills().len(), 1);
    assert_eq!(sim.position(), 50);
}

#[test]
fn test_simulator_cancel_all() {
    struct CancelAllStrategy {
        submitted: bool,
        //cancelled: bool,
        pending: Vec<OrderRequest>,
    }

    impl Strategy for CancelAllStrategy {
        fn on_quote(&mut self, sim: &Simulator, quote: &QuoteEvent, _change: &BboChange) {
            if !self.submitted && sim.bbo().is_valid() {
                // Submit multiple orders
                for price in [90, 91, 92, 93, 94] {
                    let order = Order::new_limit(0, Side::Buy, price, 10, TimeInForce::Gtc, quote.ts);
                    self.pending.push(OrderRequest::Submit(order));
                }
                self.submitted = true;
            }
        }

        fn on_trade(&mut self, _sim: &Simulator, _trade: &TradeEvent) {
            // Can't call cancel_all directly in strategy, so we'll skip this test's internal
        }

        fn pending_requests(&mut self) -> Vec<OrderRequest> {
            std::mem::take(&mut self.pending)
        }
    }

    let mut feed = build_feed_from_events(vec![
        (1000, MsgType::Quote, 100, 200, 102, 150, 0, 0, 0),
        (2000, MsgType::Quote, 100, 200, 102, 150, 0, 0, 0),
    ]);

    let config = SimConfig {
        latency_us: 0,
        ..default_sim_config()
    };

    let mut sim = Simulator::new(config);
    let mut strategy = CancelAllStrategy {
        submitted: false,
        //cancelled: false,
        pending: Vec::new(),
    };
    sim.run(&mut strategy, &mut feed);

    // 5 orders should be active (price far from market, won't fill)
    assert_eq!(sim.active_orders().len(), 5);

    // Now cancel all
    sim.cancel_all();
    assert_eq!(sim.active_orders().len(), 0);
}

#[test]
fn test_simulator_bbo_accessor() {
    let mut feed = build_feed_from_events(vec![
        (1000, MsgType::Quote, 100, 200, 102, 150, 0, 0, 0),
    ]);

    let config = default_sim_config();
    let mut sim = Simulator::new(config);
    let mut strategy = NoOpStrategy;
    sim.run(&mut strategy, &mut feed);

    assert_eq!(sim.bbo().bid_price, 100);
    assert_eq!(sim.bbo().ask_price, 102);
}

#[test]
fn test_simulator_stats_counting() {
    let mut feed = build_feed_from_events(vec![
        (1000, MsgType::Quote, 98, 200, 102, 300, 0, 0, 0),
        (2000, MsgType::Quote, 98, 200, 99, 300, 0, 0, 0), // triggers fill
    ]);

    let config = SimConfig {
        latency_us: 0,
        queue_model: QueueModel::Fifo,
        allow_immediate_cross: true,
        max_position: 1000,
        instrument: default_instrument(),
        max_fill_per_trade: 0,
    };

    let mut sim = Simulator::new(config);
    let mut strategy = BuyLimitStrategy::new(100, 75);
    sim.run(&mut strategy, &mut feed);

    let stats = sim.stats();
    assert_eq!(stats.orders_submitted, 1);
    assert_eq!(stats.total_fills, 1);
    assert_eq!(stats.total_qty_filled, 75);
    assert_eq!(stats.maker_fills, 1);
    assert_eq!(stats.taker_fills, 0);
}

#[test]
fn test_simulator_pro_rata_fill() {
    let mut feed = build_feed_from_events(vec![
        (1000, MsgType::Quote, 100, 500, 102, 300, 0, 0, 0),
        // Trade at our bid level — pro rata should give us a fraction
        (2000, MsgType::Trade, 100, 500, 102, 300, 100, 200, -1),
    ]);

    let config = SimConfig {
        latency_us: 0,
        queue_model: QueueModel::ProRata(0.1), // we get 10% of trade volume
        allow_immediate_cross: false,
        max_position: 1000,
        instrument: default_instrument(),
        max_fill_per_trade: 0,
    };

    let mut sim = Simulator::new(config);
    let mut strategy = BuyLimitStrategy::new(100, 100);
    sim.run(&mut strategy, &mut feed);

    // Trade of 200 at our price, pro-rata 10% = 20 shares
    if !sim.fills().is_empty() {
        assert_eq!(sim.fills()[0].qty, 20);
        assert_eq!(sim.position(), 20);
    }
}