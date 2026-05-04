//! Order types and order book management for the simulator.

use crate::types::*;

/// An order in the simulated book.
#[derive(Debug, Clone)]
pub struct Order {
    pub id: OrderId,
    pub side: Side,
    pub price: Price,
    pub qty: Qty,
    pub filled_qty: Qty,
    pub tif: TimeInForce,
    pub submit_ts: TsMicros,
    pub state: OrderState,
    /// Estimated queue position ahead of us (in qty units).
    pub queue_ahead: Qty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderState {
    /// Pending — submitted but not yet active (latency simulation).
    Pending,
    /// Active — resting in the book.
    Active,
    /// Filled completely.
    Filled,
    /// Partially filled (some qty remains).
    PartialFill,
    /// Cancelled by user or expired.
    Cancelled,
    /// Rejected (e.g., crossed spread with IOC and no fill).
    Rejected,
}

impl Order {
    pub fn new_limit(
        id: OrderId,
        side: Side,
        price: Price,
        qty: Qty,
        tif: TimeInForce,
        submit_ts: TsMicros,
    ) -> Self {
        Self {
            id,
            side,
            price,
            qty,
            filled_qty: 0,
            tif,
            submit_ts,
            state: OrderState::Pending,
            queue_ahead: 0,
        }
    }

    #[inline]
    pub fn remaining_qty(&self) -> Qty {
        self.qty - self.filled_qty
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        matches!(self.state, OrderState::Active | OrderState::PartialFill)
    }

    #[inline]
    pub fn is_terminal(&self) -> bool {
        matches!(self.state, OrderState::Filled | OrderState::Cancelled | OrderState::Rejected)
    }
}

/// A fill event produced by the simulator.
#[derive(Debug, Clone, Copy)]
pub struct Fill {
    pub order_id: OrderId,
    pub fill_ts: TsMicros,
    pub price: Price,
    pub qty: Qty,
    pub side: Side,
    pub is_maker: bool,    // true = filled as resting order (maker), false = crossed (taker)
    pub fee: f64,
}

impl Fill {
    /// Signed notional: positive for buys, negative for sells.
    #[inline]
    pub fn signed_qty(&self) -> i64 {
        self.qty * self.side.sign()
    }
}

/// Request from strategy to the simulator.
#[derive(Debug, Clone)]
pub enum OrderRequest {
    /// Submit a new limit order.
    Submit(Order),
    /// Cancel an existing order.
    Cancel(OrderId),
    /// Modify price/qty of an existing order (cancel-replace).
    Modify {
        id: OrderId,
        new_price: Option<Price>,
        new_qty: Option<Qty>,
    },
}