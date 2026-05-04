//! Market events — the fundamental messages the feed produces.
//! Your data is an interleaved stream of these two types.

use crate::types::*;

/// A single market event: either a quote update or a trade.
#[derive(Debug, Clone, Copy)]
pub enum MarketEvent {
    Quote(QuoteEvent),
    Trade(TradeEvent),
}

impl MarketEvent {
    #[inline(always)]
    pub fn timestamp(&self) -> TsMicros {
        match self {
            MarketEvent::Quote(q) => q.ts,
            MarketEvent::Trade(t) => t.ts,
        }
    }

    #[inline(always)]
    pub fn is_trade(&self) -> bool {
        matches!(self, MarketEvent::Trade(_))
    }

    #[inline(always)]
    pub fn is_quote(&self) -> bool {
        matches!(self, MarketEvent::Quote(_))
    }
}

/// L1 quote (BBO) update.
#[derive(Debug, Clone, Copy)]
pub struct QuoteEvent {
    pub ts: TsMicros,        // exchange timestamp in microseconds
    pub bid_price: Price,
    pub bid_qty: Qty,
    pub ask_price: Price,
    pub ask_qty: Qty,
}

impl QuoteEvent {
    #[inline(always)]
    pub fn mid_price(&self) -> Price {
        (self.bid_price + self.ask_price) / 2
    }

    #[inline(always)]
    pub fn spread(&self) -> Price {
        self.ask_price - self.bid_price
    }

    #[inline(always)]
    pub fn is_crossed(&self) -> bool {
        self.bid_price >= self.ask_price
    }
}

/// Trade event.
#[derive(Debug, Clone, Copy)]
pub struct TradeEvent {
    pub ts: TsMicros,        // exchange timestamp in microseconds
    pub price: Price,
    pub qty: Qty,
    pub aggressor: Aggressor,
}