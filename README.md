# Execution Backtest

A high-performance trading execution simulator written in Rust for backtesting execution algorithms and market-making strategies against historical L1 (top-of-book) market data.

## Overview

This crate provides a realistic simulation of order execution in financial markets, modeling:

- **Latency**: Configurable one-way delay between order submission and exchange acceptance
- **Queue Position**: FIFO, Pro-Rata, and Trade-Through queue models for limit order fills
- **Partial Fills**: Realistic modeling of partial executions based on trade events
- **Order Types**: Limit orders with GTC, IOC, FOK, and GTT time-in-force options
- **Fees**: Separate maker/taker fee structures

The simulator is designed for speed, using integer-based price representation to avoid floating-point operations in the hot path, and leveraging Polars for efficient data loading.

## Features

- **High-Performance Feed Reader**: Load interleaved trade/quote data from Parquet or CSV files using Polars
- **L1 Order Book Tracking**: Real-time best bid/offer (BBO) state with queue position estimation
- **Realistic Fill Modeling**: Multiple queue models (FIFO, Pro-Rata, Trade-Through)
- **Position Tracking**: Real-time position and PnL calculation
- **Comprehensive Statistics**: Fill rates, maker ratios, and detailed fill logs
- **Extensible Strategy Interface**: Implement the `Strategy` trait to define custom trading logic

## Project Structure

```
src/
├── lib.rs        # Library root, exports all modules
├── types.rs      # Core numeric types (Price, Qty, Side, etc.)
├── event.rs      # Market events (Quote, Trade)
├── order.rs      # Order types, states, and fill events
├── book.rs       # L1 order book (BBO) tracker
├── feed.rs       # Polars-based data feed reader
├── sim.rs        # Main simulator engine
├── strategy.rs   # Strategy trait and example implementation
└── stats.rs      # Statistics and PnL calculation
```

## Core Components

### Types (`types.rs`)

All prices are stored as `i64` in "price ticks" to avoid floating-point inaccuracies:

- `Price` (i64): Integer price in ticks
- `Qty` (i64): Quantity in shares/contracts
- `TsMicros` (i64): Timestamp in microseconds since epoch
- `Side`: Buy or Sell
- `TimeInForce`: GTC, IOC, FOK, GTT
- `Instrument`: Symbol metadata including tick_size, tick_value, and fee rates

### Events (`event.rs`)

Two types of market events:

- `QuoteEvent`: L1 quote update with bid/ask prices and quantities
- `TradeEvent`: Trade with price, quantity, and aggressor side

### Order Book (`book.rs`)

- `Bbo`: Best Bid/Offer state
- `L1Book`: Tracks BBO changes and estimates queue consumption from trades

### Feed (`feed.rs`)

High-performance data loader supporting multiple data layouts:

- **Unified**: Single file with `msg_type` column ("T" or "Q")
- **Split**: Separate trade and quote files merged by timestamp
- **Flat**: Every row has all fields, trade fields are 0/null for quotes

Supports Parquet and CSV formats with:
- Time range filtering
- Symbol filtering
- Configurable column mappings
- Price scaling
- Trade side inference (Lee-Ready algorithm)

### Simulator (`sim.rs`)

The core execution engine that:

1. Processes market events from the feed
2. Updates the order book state
3. Simulates order fills based on queue position
4. Tracks position and statistics
5. Enforces position limits

Queue models:
- **FIFO**: Order joins at back of queue, fills when queue is consumed
- **ProRata**: Proportional fill based on a share multiplier
- **TradeThrough**: Only fills when price trades through the order level

### Strategy (`strategy.rs`)

Implement the `Strategy` trait to define trading logic:

```rust
pub trait Strategy {
    fn on_start(&mut self, sim: &Simulator) {}
    fn on_quote(&mut self, sim: &Simulator, quote: &QuoteEvent, change: &BboChange);
    fn on_trade(&mut self, sim: &Simulator, trade: &TradeEvent);
    fn on_end(&mut self, sim: &Simulator) {}
    fn pending_requests(&mut self) -> Vec<OrderRequest> { Vec::new() }
}
```

Includes `SimpleMarketMaker` example strategy.

### Statistics (`stats.rs`)

- `SimStats`: Aggregate statistics (fills, fill rate, maker ratio, fees)
- `PnlCalculator`: FIFO-based realized PnL, unrealized PnL, VWAP, implementation shortfall

## Dependencies

- **polars** (0.53.0): Data loading and processing with lazy evaluation
- **serde** (1.0.228): Serialization support
- **thiserror** (2.0.18): Error handling
- **tracing** (0.1.44): Logging and instrumentation

## Usage

```rust
use execution_backtest::*;

// 1. Configure the instrument
let instrument = Instrument {
    symbol: "ES".into(),
    tick_size: 0.25,
    tick_value: 12.50,
    lot_size: 1,
    price_scale: 4.0,  // price * 4 = integer ticks
    maker_fee_per_unit: 0.0,
    taker_fee_per_unit: 0.0,
};

// 2. Configure the simulator
let sim_config = SimConfig {
    latency_us: 50,
    queue_model: QueueModel::Fifo,
    allow_immediate_cross: true,
    max_position: 10,
    instrument: instrument.clone(),
    max_fill_per_trade: 0,
};

// 3. Configure and load the data feed
let feed_config = FeedConfig {
    schema: FeedSchema::default(),
    price_scale: 4.0,
    side_encoding: SideEncoding::Numeric,
    time_range: None,
    symbol_filter: None,
};
let mut feed = Feed::from_parquet("data/es_2024.parquet", &feed_config)?;

// 4. Create strategy and simulator
let mut strategy = SimpleMarketMaker::new(2, 1, 5);
let mut simulator = Simulator::new(sim_config);

// 5. Run simulation
simulator.run(&mut strategy, &mut feed);

// 6. Analyze results
let stats = simulator.stats();
let fills = simulator.fills();
let pnl_calc = PnlCalculator::new(instrument);
let realized_pnl = pnl_calc.realized_pnl(fills);
```

## Data Format

The feed expects data with the following columns (configurable via `FeedSchema`):

| Column | Type | Description |
|--------|------|-------------|
| `ts_us` | i64 | Timestamp in microseconds |
| `bid_price` | f64/i64 | Best bid price |
| `bid_qty` | i64 | Best bid quantity |
| `ask_price` | f64/i64 | Best ask price |
| `ask_qty` | i64 | Best ask quantity |
| `trade_price` | f64/i64 | Trade price (0 for quotes) |
| `trade_qty` | i64 | Trade quantity (0 for quotes) |
| `trade_side` | i8 | 1=buy, -1=sell, 0=unknown |

Trade vs quote rows can be distinguished by:
- A `msg_type` column with "T"/"Q" values
- `trade_qty > 0` indicating a trade (default)
- Separate files for trades and quotes

## License

MIT