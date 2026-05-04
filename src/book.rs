//! L1 order book tracker. Maintains current BBO state from the feed.
//! Also provides queue position estimation for limit orders.

use crate::types::*;
use crate::event::*;

/// Current best bid/offer state.
#[derive(Debug, Clone, Copy, Default)]
pub struct Bbo {
    pub bid_price: Price,
    pub bid_qty: Qty,
    pub ask_price: Price,
    pub ask_qty: Qty,
    pub last_update_ts: TsMicros,
}

impl Bbo {
    #[inline(always)]
    pub fn mid_price(&self) -> Price {
        (self.bid_price + self.ask_price) / 2
    }

    #[inline(always)]
    pub fn spread(&self) -> Price {
        self.ask_price - self.bid_price
    }

    #[inline(always)]
    pub fn is_valid(&self) -> bool {
        self.bid_price > 0 && self.ask_price > 0 && self.bid_price < self.ask_price
    }

    /// Price you'd get for a market buy (take the ask).
    #[inline(always)]
    pub fn market_buy_price(&self) -> Price {
        self.ask_price
    }

    /// Price you'd get for a market sell (hit the bid).
    #[inline(always)]
    pub fn market_sell_price(&self) -> Price {
        self.bid_price
    }

    /// Available liquidity on the given side at the best level.
    #[inline(always)]
    pub fn qty_at_best(&self, side: Side) -> Qty {
        match side {
            Side::Buy => self.bid_qty,
            Side::Sell => self.ask_qty,
        }
    }

    /// Best price on the given side.
    #[inline(always)]
    pub fn best_price(&self, side: Side) -> Price {
        match side {
            Side::Buy => self.bid_price,
            Side::Sell => self.ask_price,
        }
    }
}

/// Tracks BBO state and provides queue position estimation.
#[derive(Debug, Clone)]
pub struct L1Book {
    pub bbo: Bbo,
    prev_bbo: Bbo,  // previous BBO for detecting level changes
}

impl L1Book {
    pub fn new() -> Self {
        Self {
            bbo: Bbo::default(),
            prev_bbo: Bbo::default(),
        }
    }

    /// Update from a quote event. Returns whether the BBO changed.
    #[inline]
    pub fn on_quote(&mut self, q: &QuoteEvent) -> BboChange {
        self.prev_bbo = self.bbo;
        self.bbo = Bbo {
            bid_price: q.bid_price,
            bid_qty: q.bid_qty,
            ask_price: q.ask_price,
            ask_qty: q.ask_qty,
            last_update_ts: q.ts,
        };

        BboChange {
            bid_price_changed: self.bbo.bid_price != self.prev_bbo.bid_price,
            ask_price_changed: self.bbo.ask_price != self.prev_bbo.ask_price,
            bid_qty_changed: self.bbo.bid_qty != self.prev_bbo.bid_qty,
            ask_qty_changed: self.bbo.ask_qty != self.prev_bbo.ask_qty,
        }
    }

    /// Get the previous BBO (before last update).
    #[inline]
    pub fn prev_bbo(&self) -> &Bbo {
        &self.prev_bbo
    }

    /// Estimate queue position depletion from a trade.
    /// Returns how much of the resting quantity was consumed.
    #[inline]
    pub fn estimate_queue_consumed(&self, trade: &TradeEvent) -> QueueConsumption {
        match trade.aggressor {
            Aggressor::Buy => {
                // Buy aggressor hits the ask
                if trade.price == self.bbo.ask_price {
                    QueueConsumption {
                        side: Side::Sell,
                        price: trade.price,
                        qty_consumed: trade.qty,
                    }
                } else {
                    QueueConsumption::none()
                }
            }
            Aggressor::Sell => {
                // Sell aggressor hits the bid
                if trade.price == self.bbo.bid_price {
                    QueueConsumption {
                        side: Side::Buy,
                        price: trade.price,
                        qty_consumed: trade.qty,
                    }
                } else {
                    QueueConsumption::none()
                }
            }
            Aggressor::Unknown => {
                // Try to infer from price
                if trade.price == self.bbo.ask_price {
                    QueueConsumption {
                        side: Side::Sell,
                        price: trade.price,
                        qty_consumed: trade.qty,
                    }
                } else if trade.price == self.bbo.bid_price {
                    QueueConsumption {
                        side: Side::Buy,
                        price: trade.price,
                        qty_consumed: trade.qty,
                    }
                } else {
                    QueueConsumption::none()
                }
            }
        }
    }
}

impl Default for L1Book {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BboChange {
    pub bid_price_changed: bool,
    pub ask_price_changed: bool,
    pub bid_qty_changed: bool,
    pub ask_qty_changed: bool,
}

impl BboChange {
    #[inline(always)]
    pub fn any_price_change(&self) -> bool {
        self.bid_price_changed || self.ask_price_changed
    }

    #[inline(always)]
    pub fn any_change(&self) -> bool {
        self.bid_price_changed || self.ask_price_changed
            || self.bid_qty_changed || self.ask_qty_changed
    }
}

#[derive(Debug, Clone, Copy)]
pub struct QueueConsumption {
    pub side: Side,
    pub price: Price,
    pub qty_consumed: Qty,
}

impl QueueConsumption {
    fn none() -> Self {
        Self {
            side: Side::Buy,
            price: 0,
            qty_consumed: 0,
        }
    }
}