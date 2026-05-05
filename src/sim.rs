//! The execution simulator engine.
//!
//! Models:
//! - Latency: configurable one-way latency before orders become active.
//! - Queue position: estimates position in queue and simulates FIFO fills.
//! - Trades-through: immediate fill when price trades through our level.
//! - Partial fills: proportional or full depletion models.

use crate::types::*;
use crate::event::*;
use crate::book::*;
use crate::order::*;
use crate::strategy::Strategy;
use crate::stats::SimStats;

/// Simulator configuration.
#[derive(Debug, Clone)]
pub struct SimConfig {
    /// One-way latency in microseconds (order submission → exchange).
    pub latency_us: TsMicros,

    /// Queue position model.
    pub queue_model: QueueModel,

    /// Whether to fill limit orders that would cross the spread immediately (as taker).
    pub allow_immediate_cross: bool,

    /// Maximum position allowed (absolute value). 0 = unlimited.
    pub max_position: Qty,

    /// Instrument metadata for fee calculation.
    pub instrument: Instrument,

    /// Fill size limit per trade event (0 = trade.qty).
    pub max_fill_per_trade: Qty,
}

/// How to model queue position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QueueModel {
    /// Assume we join at the back of the queue when our order arrives.
    /// Fill only after queue_ahead is fully consumed.
    Fifo,

    /// Pro-rata: we get filled proportionally when trades happen at our level.
    /// param = our share multiplier (0.0-1.0, typically ~0.1 for realistic fills).
    ProRata(f64),

    /// Pessimistic: only fill on trade-through (price moves past our level).
    TradeThrough,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            latency_us: 50,  // 50 μs default
            queue_model: QueueModel::Fifo,
            allow_immediate_cross: true,
            max_position: 0,
            instrument: Instrument::default(),
            max_fill_per_trade: 0,
            }
    }
}

/// The simulator engine.
pub struct Simulator {
    config: SimConfig,
    book: L1Book,

    // Order management
    active_orders: Vec<Order>,
    pending_orders: Vec<Order>,  // orders waiting for latency to elapse

    // Fill log
    fills: Vec<Fill>,

    // Position tracking
    position: Qty,  // signed: positive = long, negative = short

    // Stats
    stats: SimStats,

    // Monotonic ID generator
    next_order_id: OrderId,
}

impl Simulator {
    pub fn new(config: SimConfig) -> Self {
        Self {
            config,
            book: L1Book::new(),
            active_orders: Vec::with_capacity(16),
            pending_orders: Vec::with_capacity(16),
            fills: Vec::with_capacity(1024),
            position: 0,
            stats: SimStats::new(),
            next_order_id: 1,
        }
    }

    /// Run the full simulation: feed events → strategy decisions → fills.
    pub fn run<S: Strategy>(&mut self, strategy: &mut S, feed: &mut crate::feed::Feed) {
        // Initial callback
        strategy.on_start(self);

        while let Some(event) = feed.next_event() {
            let ts = event.timestamp();

            // 1. Activate pending orders whose latency has elapsed
            self.activate_pending_orders(ts);

            // 2. Process market event
            match event {
                MarketEvent::Quote(q) => {
                    let change = self.book.on_quote(&q);
                    self.handle_quote_fills(ts, &q);
                    strategy.on_quote(self, &q, &change);
                }
                MarketEvent::Trade(t) => {
                    self.handle_trade_fills(ts, &t);
                    strategy.on_trade(self, &t);
                }
            }

            // 3. Process strategy order requests
            self.process_requests(strategy.pending_requests());

            // 4. Expire GTT orders
            self.expire_orders(ts);
        }

        // Final callback
        strategy.on_end(self);
    }

    // ─── Order management (called by strategy) ─────────────────────────────

    /// Submit a new limit order. Returns the assigned order ID.
    pub fn submit_limit(
        &mut self,
        side: Side,
        price: Price,
        qty: Qty,
        tif: TimeInForce,
        current_ts: TsMicros,
    ) -> OrderId {
        let id = self.next_order_id;
        self.next_order_id += 1;

        let order = Order::new_limit(id, side, price, qty, tif, current_ts);
        self.pending_orders.push(order);
        self.stats.orders_submitted += 1;

        id
    }

    /// Cancel an order. Returns true if found and cancelled.
    pub fn cancel(&mut self, order_id: OrderId) -> bool {
        // Check active orders
        if let Some(pos) = self.active_orders.iter().position(|o| o.id == order_id) {
            self.active_orders[pos].state = OrderState::Cancelled;
            let removed = self.active_orders.swap_remove(pos);
            self.stats.orders_cancelled += 1;
            let _ = removed;
            return true;
        }

        // Check pending orders
        if let Some(pos) = self.pending_orders.iter().position(|o| o.id == order_id) {
            self.pending_orders[pos].state = OrderState::Cancelled;
            self.pending_orders.swap_remove(pos);
            self.stats.orders_cancelled += 1;
            return true;
        }

        false
    }

    /// Cancel all active orders.
    pub fn cancel_all(&mut self) {
        for order in self.active_orders.drain(..) {
            let _ = order;
            self.stats.orders_cancelled += 1;
        }
        for order in self.pending_orders.drain(..) {
            let _ = order;
            self.stats.orders_cancelled += 1;
        }
    }

    // ─── Accessors ─────────────────────────────────────────────────────────

    #[inline]
    pub fn bbo(&self) -> &Bbo {
        &self.book.bbo
    }

    #[inline]
    pub fn position(&self) -> Qty {
        self.position
    }

    #[inline]
    pub fn active_orders(&self) -> &[Order] {
        &self.active_orders
    }

    #[inline]
    pub fn fills(&self) -> &[Fill] {
        &self.fills
    }

    #[inline]
    pub fn stats(&self) -> &SimStats {
        &self.stats
    }

    #[inline]
    pub fn config(&self) -> &SimConfig {
        &self.config
    }

    pub fn active_buy_orders(&self) -> impl Iterator<Item = &Order> {
        self.active_orders.iter().filter(|o| o.side == Side::Buy)
    }

    pub fn active_sell_orders(&self) -> impl Iterator<Item = &Order> {
        self.active_orders.iter().filter(|o| o.side == Side::Sell)
    }

    // ─── Internal simulation logic ────────────────────────────────────────

    fn activate_pending_orders(&mut self, current_ts: TsMicros) {
        let latency = self.config.latency_us;

        let mut i = 0;
        while i < self.pending_orders.len() {
            if current_ts >= self.pending_orders[i].submit_ts + latency {
                let mut order = self.pending_orders.swap_remove(i);
                order.state = OrderState::Active;

                // Estimate initial queue position
                order.queue_ahead = self.estimate_initial_queue(&order);

                // Check for immediate cross (marketable limit)
                if self.config.allow_immediate_cross && self.would_cross(&order) {
                    self.fill_crossing_order(&mut order, current_ts);
                    if order.remaining_qty() > 0 {
                        self.active_orders.push(order);
                    }
                } else {
                    self.active_orders.push(order);
                }
                // Don't increment i — swap_remove moved a new element here
            } else {
                i += 1;
            }
        }
    }

    fn would_cross(&self, order: &Order) -> bool {
        let bbo = &self.book.bbo;
        if !bbo.is_valid() {
            return false;
        }
        match order.side {
            Side::Buy => order.price >= bbo.ask_price,
            Side::Sell => order.price <= bbo.bid_price,
        }
    }

    fn fill_crossing_order(&mut self, order: &mut Order, ts: TsMicros) {
        let bbo = &self.book.bbo;
        let fill_price = match order.side {
            Side::Buy => bbo.ask_price,
            Side::Sell => bbo.bid_price,
        };

        let available = match order.side {
            Side::Buy => bbo.ask_qty,
            Side::Sell => bbo.bid_qty,
        };

        let fill_qty = order.remaining_qty().min(available);
        if fill_qty <= 0 {
            return;
        }

        // Position limit check
        let fill_qty = self.position_limit_qty(order.side, fill_qty);
        if fill_qty <= 0 {
            return;
        }

        self.record_fill(order, ts, fill_price, fill_qty, false);
    }

    fn handle_quote_fills(&mut self, ts: TsMicros, q: &QuoteEvent) {
        // When the BBO price improves through our limit price, we get filled.
        let mut i = 0;
        while i < self.active_orders.len() {
            let order = &self.active_orders[i];
            let filled = match order.side {
                // Buy order: if new ask drops to or below our price → fill
                Side::Buy => q.ask_price <= order.price && q.ask_price > 0,
                // Sell order: if new bid rises to or above our price → fill
                Side::Sell => q.bid_price >= order.price && q.bid_price > 0,
            };

            if filled {
                let mut order = self.active_orders.swap_remove(i);
                let fill_qty = self.position_limit_qty(order.side, order.remaining_qty());
                if fill_qty > 0 {
                    let price = order.price;
                    self.record_fill(&mut order, ts, price, fill_qty, true);
                }
                if order.remaining_qty() > 0 && !order.is_terminal() {
                    self.active_orders.push(order);
                    // Don't increment — check the swapped element
                } else {
                    // Order fully filled or terminal
                }
            } 
            
            i += 1;
            
        }
    }

    fn handle_trade_fills(&mut self, ts: TsMicros, trade: &TradeEvent) {
        let consumption = self.book.estimate_queue_consumed(trade);

        let mut i = 0;
        while i < self.active_orders.len() {
            //let order = &mut self.active_orders[i];

            let side = self.active_orders[i].side;
            let price = self.active_orders[i].price;

            // Only process orders on the same side as the resting liquidity
            let resting_side = match trade.aggressor {
                Aggressor::Buy => Side::Sell,   // aggressor buys → resting are sells
                Aggressor::Sell => Side::Buy,   // aggressor sells → resting are buys
                Aggressor::Unknown => {
                    // Infer from price
                    if trade.price >= self.book.bbo.ask_price {
                        Side::Sell
                    } else {
                        Side::Buy
                    }
                }
            };

            if (side != resting_side || price != trade.price) && self.config.queue_model != QueueModel::TradeThrough {
                i += 1;
                continue;
            }

            // Queue position logic
            match self.config.queue_model {
                QueueModel::Fifo => {
                    // Deplete queue ahead
                    let order = &mut self.active_orders[i];
                    order.queue_ahead -= trade.qty.min(order.queue_ahead);

                    if order.queue_ahead <= 0 {
                        // We're at the front — get filled by remaining trade qty
                        let trade_remaining = trade.qty - consumption.qty_consumed.min(trade.qty);
                        let can_fill = order.remaining_qty().min(trade_remaining);
                        
                        let fill_qty = self.position_limit_qty(side, can_fill);
                        
                        if fill_qty > 0 {
                            let mut order = self.active_orders.swap_remove(i);
                            let price = order.price;
                            self.record_fill(&mut order, ts, price, fill_qty, true);
                            if order.remaining_qty() > 0 && !order.is_terminal() {
                                self.active_orders.push(order);
                            }
                            continue; // don't increment i
                        }
                    }
                }
                QueueModel::ProRata(share) => {
                    let order = &mut self.active_orders[i];
                    let our_fill = ((trade.qty as f64) * share).floor() as Qty;
                    let fill_qty = our_fill
                        .min(order.remaining_qty())
                        .max(0);
                    let fill_qty = self.position_limit_qty(side, fill_qty);

                    if fill_qty > 0 {
                        let mut order = self.active_orders.swap_remove(i);
                        self.record_fill(&mut order, ts, price, fill_qty, true);
                        if order.remaining_qty() > 0 && !order.is_terminal() {
                            self.active_orders.push(order);
                        }
                    }
                }
                QueueModel::TradeThrough => {
                    let order = &mut self.active_orders[i];
                    let remaining_quantity = order.remaining_qty();
                    // Only fill if trade price is strictly through our level
                    let through = match order.side {
                        Side::Buy => trade.price < order.price,
                        Side::Sell => trade.price > order.price,
                    };

                    if through {
                        let fill_qty = self.position_limit_qty(side, remaining_quantity);
                        if fill_qty > 0 {
                            let mut order = self.active_orders.swap_remove(i);
                            self.record_fill(&mut order, ts, price, fill_qty, true);
                            if order.remaining_qty() > 0 && !order.is_terminal() {
                                self.active_orders.push(order);
                            }
                            continue;
                        }
                    }
                }
            }

            i += 1;
        }
    }

    fn estimate_initial_queue(&self, order: &Order) -> Qty {
        let bbo = &self.book.bbo;
        match order.side {
            Side::Buy => {
                if order.price == bbo.bid_price {
                    bbo.bid_qty  // join at back of bid queue
                } else if order.price < bbo.bid_price {
                    0  // deeper in book — assume we're alone
                } else {
                    0  // would cross — handled separately
                }
            }
            Side::Sell => {
                if order.price == bbo.ask_price {
                    bbo.ask_qty  // join at back of ask queue
                } else if order.price > bbo.ask_price {
                    0
                } else {
                    0
                }
            }
        }
    }

    fn record_fill(&mut self, order: &mut Order, ts: TsMicros, price: Price, qty: Qty, is_maker: bool) {
        order.filled_qty += qty;
        if order.filled_qty >= order.qty {
            order.state = OrderState::Filled;
        } else {
            order.state = OrderState::PartialFill;
        }

        let fee_per_unit = if is_maker {
            self.config.instrument.maker_fee_per_unit
        } else {
            self.config.instrument.taker_fee_per_unit
        };

        let fill = Fill {
            order_id: order.id,
            fill_ts: ts,
            price,
            qty,
            side: order.side,
            is_maker,
            fee: fee_per_unit * qty as f64,
        };

        // Update position
        self.position += qty * order.side.sign();

        // Update stats
        self.stats.total_fills += 1;
        self.stats.total_qty_filled += qty;
        self.stats.total_fees += fill.fee;
        if is_maker {
            self.stats.maker_fills += 1;
        } else {
            self.stats.taker_fills += 1;
        }

        self.fills.push(fill);
    }

    fn position_limit_qty(&self, side: Side, desired: Qty) -> Qty {
        if self.config.max_position == 0 {
            return desired; // no limit
        }

        let new_pos = self.position + desired * side.sign();
        if new_pos.abs() > self.config.max_position {
            let allowed = self.config.max_position - self.position.abs();
            allowed.max(0).min(desired)
        } else {
            desired
        }
    }

    fn expire_orders(&mut self, current_ts: TsMicros) {
        let mut i = 0;
        while i < self.active_orders.len() {
            if let TimeInForce::Gtt(expiry) = self.active_orders[i].tif {
                if current_ts >= expiry {
                    self.active_orders[i].state = OrderState::Cancelled;
                    self.active_orders.swap_remove(i);
                    self.stats.orders_cancelled += 1;
                    continue;
                }
            }
            i += 1;
        }
    }

    fn process_requests(&mut self, requests: Vec<OrderRequest>) {
        for req in requests {
            match req {
                OrderRequest::Submit(order) => {
                    self.pending_orders.push(order);
                    self.stats.orders_submitted += 1;
                }
                OrderRequest::Cancel(id) => {
                    self.cancel(id);
                }
                OrderRequest::Modify { id, new_price, new_qty } => {
                    // Cancel and resubmit model
                    if let Some(pos) = self.active_orders.iter().position(|o| o.id == id) {
                        let old = self.active_orders.swap_remove(pos);
                        let price = new_price.unwrap_or(old.price);
                        let qty = new_qty.unwrap_or(old.remaining_qty());
                        let new_order = Order::new_limit(
                            old.id, old.side, price, qty, old.tif, old.submit_ts
                        );
                        self.pending_orders.push(new_order);
                    }
                }
            }
        }
    }
}