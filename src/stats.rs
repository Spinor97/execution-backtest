//! Simulation statistics and PnL calculation.

use crate::types::*;
use crate::order::Fill;

/// Aggregate statistics collected during simulation.
#[derive(Debug, Clone, Default)]
pub struct SimStats {
    pub orders_submitted: u64,
    pub orders_cancelled: u64,
    pub total_fills: u64,
    pub total_qty_filled: Qty,
    pub maker_fills: u64,
    pub taker_fills: u64,
    pub total_fees: f64,
}

impl SimStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fill_rate(&self) -> f64 {
        if self.orders_submitted == 0 {
            0.0
        } else {
            self.total_fills as f64 / self.orders_submitted as f64
        }
    }

    pub fn maker_ratio(&self) -> f64 {
        if self.total_fills == 0 {
            0.0
        } else {
            self.maker_fills as f64 / self.total_fills as f64
        }
    }
}

/// PnL calculator from a fill log.
#[derive(Debug, Clone)]
pub struct PnlCalculator {
    pub instrument: crate::types::Instrument,
}

impl PnlCalculator {
    pub fn new(instrument: crate::types::Instrument) -> Self {
        Self { instrument }
    }

    /// Compute realized PnL from a sequence of fills.
    /// Uses FIFO matching: first buy matched with first sell.
    pub fn realized_pnl(&self, fills: &[Fill]) -> f64 {
        let mut buy_queue: Vec<(Price, Qty)> = Vec::new();
        let mut sell_queue: Vec<(Price, Qty)> = Vec::new();
        let mut pnl = 0.0;
        let mut total_fees = 0.0;

        for fill in fills {
            total_fees += fill.fee;

            match fill.side {
                Side::Buy => {
                    // Try to match against existing sells
                    let mut remaining = fill.qty;
                    while remaining > 0 && !sell_queue.is_empty() {
                        let (sell_price, sell_qty) = &mut sell_queue[0];
                        let match_qty = remaining.min(*sell_qty);

                        // PnL = (sell_price - buy_price) * qty * tick_value
                        pnl += (*sell_price - fill.price) as f64
                            * match_qty as f64
                            * self.instrument.tick_value
                            / self.instrument.price_scale;

                        *sell_qty -= match_qty;
                        remaining -= match_qty;

                        if *sell_qty == 0 {
                            sell_queue.remove(0);
                        }
                    }
                    if remaining > 0 {
                        buy_queue.push((fill.price, remaining));
                    }
                }
                Side::Sell => {
                    let mut remaining = fill.qty;
                    while remaining > 0 && !buy_queue.is_empty() {
                        let (buy_price, buy_qty) = &mut buy_queue[0];
                        let match_qty = remaining.min(*buy_qty);

                        pnl += (fill.price - *buy_price) as f64
                            * match_qty as f64
                            * self.instrument.tick_value
                            / self.instrument.price_scale;

                        *buy_qty -= match_qty;
                        remaining -= match_qty;

                        if *buy_qty == 0 {
                            buy_queue.remove(0);
                        }
                    }
                    if remaining > 0 {
                        sell_queue.push((fill.price, remaining));
                    }
                }
            }
        }

        pnl - total_fees
    }

    /// Compute unrealized PnL for open position at a given mark price.
    pub fn unrealized_pnl(&self, fills: &[Fill], mark_price: Price) -> f64 {
        let mut net_position: Qty = 0;
        let mut cost_basis: f64 = 0.0;

        for fill in fills {
            let signed_qty = fill.qty * fill.side.sign();
            cost_basis += fill.price as f64 * signed_qty as f64;
            net_position += signed_qty;
        }

        if net_position == 0 {
            return 0.0;
        }

        let mark_value = mark_price as f64 * net_position as f64;
        (mark_value - cost_basis) * self.instrument.tick_value / self.instrument.price_scale
    }

    /// VWAP of all fills on a given side.
    pub fn vwap(&self, fills: &[Fill], side: Side) -> Option<f64> {
        let mut total_qty: Qty = 0;
        let mut total_value: f64 = 0.0;

        for fill in fills {
            if fill.side == side {
                total_qty += fill.qty;
                total_value += fill.price as f64 * fill.qty as f64;
            }
        }

        if total_qty == 0 {
            None
        } else {
            Some(total_value / total_qty as f64 / self.instrument.price_scale)
        }
    }

    /// Implementation shortfall vs arrival price.
    pub fn implementation_shortfall(
        &self,
        fills: &[Fill],
        arrival_price: Price,
        side: Side,
    ) -> f64 {
        let mut total_qty: Qty = 0;
        let mut total_slippage: f64 = 0.0;

        for fill in fills {
            if fill.side == side {
                let slip = match side {
                    Side::Buy => (fill.price - arrival_price) as f64,
                    Side::Sell => (arrival_price - fill.price) as f64,
                };
                total_slippage += slip * fill.qty as f64;
                total_qty += fill.qty;
            }
        }

        if total_qty == 0 {
            0.0
        } else {
            total_slippage / total_qty as f64
                * self.instrument.tick_value
                / self.instrument.price_scale
        }
    }
}