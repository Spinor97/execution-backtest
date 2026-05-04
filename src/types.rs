//! Core numeric types used throughout the simulator.
//!
//! All prices are stored as i64 in "price ticks" — the raw integer from your
//! data source (e.g., microsecond-resolution price * some scale factor).
//! This avoids floating point in the hot path entirely.

/// Price in integer ticks. Interpretation depends on your instrument.
/// e.g., if raw data has price 150.2350, and tick_size = 0.0001,
/// then Price = 1_502_350.
pub type Price = i64;

/// Quantity in integer units (shares, contracts, lots).
pub type Qty = i64;

/// Timestamp in microseconds since epoch (Unix micros).
pub type TsMicros = i64;

/// Order identifier.
pub type OrderId = u64;

/// Side of an order or trade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    #[inline(always)]
    pub fn opposite(self) -> Self {
        match self {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        }
    }

    /// Sign: +1 for Buy, -1 for Sell. Useful for PnL calculations.
    #[inline(always)]
    pub fn sign(self) -> i64 {
        match self {
            Side::Buy => 1,
            Side::Sell => -1,
        }
    }
}

/// Aggressor side on a trade message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aggressor {
    Buy,
    Sell,
    Unknown,
}

impl Aggressor {
    #[inline(always)]
    pub fn as_side(self) -> Option<Side> {
        match self {
            Aggressor::Buy => Some(Side::Buy),
            Aggressor::Sell => Some(Side::Sell),
            Aggressor::Unknown => None,
        }
    }
}

/// Time-in-force for orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeInForce {
    /// Good till cancel — stays on book until filled or cancelled.
    Gtc,
    /// Immediate or cancel — fill what you can, cancel the rest.
    Ioc,
    /// Fill or kill — fill entirely or cancel entirely.
    Fok,
    /// Good till time — cancel after specified timestamp.
    Gtt(TsMicros),
}

/// Instrument metadata. Configure once per symbol.
#[derive(Debug, Clone)]
pub struct Instrument {
    pub symbol: String,
    pub tick_size: f64,        // minimum price increment as float (for display)
    pub tick_value: f64,       // dollar value per tick per contract
    pub lot_size: Qty,         // minimum order size
    pub price_scale: f64,      // multiply float price by this to get Price integer
    pub maker_fee_per_unit: f64,
    pub taker_fee_per_unit: f64,
}

impl Instrument {
    /// Convert integer price back to float for display.
    #[inline]
    pub fn price_to_float(&self, p: Price) -> f64 {
        p as f64 / self.price_scale
    }

    /// Convert float price to integer tick price.
    #[inline]
    pub fn float_to_price(&self, f: f64) -> Price {
        (f * self.price_scale).round() as Price
    }
}

impl Default for Instrument {
    fn default() -> Self {
        Self {
            symbol: "UNKNOWN".into(),
            tick_size: 0.01,
            tick_value: 0.01,
            lot_size: 1,
            price_scale: 100.0,
            maker_fee_per_unit: 0.0,
            taker_fee_per_unit: 0.0,
        }
    }
}