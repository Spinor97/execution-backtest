//! Strategy trait that users implement.

use crate::types::*;
use crate::event::*;
use crate::book::*;
use crate::order::*;
use crate::sim::Simulator;

/// Implement this trait to define your trading strategy.
pub trait Strategy {
    /// Called once at the start of simulation.
    fn on_start(&mut self, _sim: &Simulator) {}

    /// Called on every quote (BBO) update.
    fn on_quote(&mut self, sim: &Simulator, quote: &QuoteEvent, change: &BboChange);

    /// Called on every trade event.
    fn on_trade(&mut self, sim: &Simulator, trade: &TradeEvent);

    /// Called once at the end of simulation.
    fn on_end(&mut self, _sim: &Simulator) {}

    /// Return pending order requests. Called after every event.
    /// Default implementation returns empty vec (no orders).
    fn pending_requests(&mut self) -> Vec<OrderRequest> {
        Vec::new()
    }
}

// ─── Example strategy: simple market maker ─────────────────────────────────────

/// A basic market-making strategy for testing.
/// Quotes symmetrically around mid with a configurable offset.
pub struct SimpleMarketMaker {
    pub offset_ticks: Price,     // distance from mid to our quotes
    pub qty: Qty,                // size per side
    pub max_position: Qty,       // skew beyond this → stop quoting that side
    pending: Vec<OrderRequest>,
    buy_order_id: Option<OrderId>,
    sell_order_id: Option<OrderId>,
    last_bid: Price,
    last_ask: Price,
}

impl SimpleMarketMaker {
    pub fn new(offset_ticks: Price, qty: Qty, max_position: Qty) -> Self {
        Self {
            offset_ticks,
            qty,
            max_position,
            pending: Vec::new(),
            buy_order_id: None,
            sell_order_id: None,
            last_bid: 0,
            last_ask: 0,
        }
    }
}

impl Strategy for SimpleMarketMaker {
    fn on_quote(&mut self, sim: &Simulator, quote: &QuoteEvent, change: &BboChange) {
        if !change.any_price_change() {
            return;
        }

        let bbo = sim.bbo();
        if !bbo.is_valid() {
            return;
        }

        let mid = bbo.mid_price();
        let desired_bid = mid - self.offset_ticks;
        let desired_ask = mid + self.offset_ticks;

        // Only requote if prices changed
        if desired_bid == self.last_bid && desired_ask == self.last_ask {
            return;
        }

        // Cancel existing orders
        if let Some(id) = self.buy_order_id.take() {
            self.pending.push(OrderRequest::Cancel(id));
        }
        if let Some(id) = self.sell_order_id.take() {
            self.pending.push(OrderRequest::Cancel(id));
        }

        let pos = sim.position();

        // Submit new buy (if not max long)
        if pos < self.max_position {
            let order = Order::new_limit(
                0, // will be assigned by sim
                Side::Buy,
                desired_bid,
                self.qty,
                TimeInForce::Gtc,
                quote.ts,
            );
            self.pending.push(OrderRequest::Submit(order));
            self.last_bid = desired_bid;
        }

        // Submit new sell (if not max short)
        if pos > -self.max_position {
            let order = Order::new_limit(
                0,
                Side::Sell,
                desired_ask,
                self.qty,
                TimeInForce::Gtc,
                quote.ts,
            );
            self.pending.push(OrderRequest::Submit(order));
            self.last_ask = desired_ask;
        }
    }

    fn on_trade(&mut self, _sim: &Simulator, _trade: &TradeEvent) {
        // This simple strategy doesn't react to trades
    }

    fn pending_requests(&mut self) -> Vec<OrderRequest> {
        std::mem::take(&mut self.pending)
    }

    fn on_end(&mut self, sim: &Simulator) {
        let stats = sim.stats();
        tracing::info!(
            fills = stats.total_fills,
            qty = stats.total_qty_filled,
            fees = format!("{:.4}", stats.total_fees),
            "Strategy finished"
        );
    }
}