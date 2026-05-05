use execution_backtest::feed::*;
use execution_backtest::sim::*;
use execution_backtest::strategy::SimpleMarketMaker;
use execution_backtest::stats::PnlCalculator;
use execution_backtest::types::*;

use std::time::Instant;

fn main() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    // ─── Configure your data schema ───────────────────────────────────────
    //
    // Adapt these column names to match YOUR data file.
    // Example: interleaved trade/quote CSV or Parquet with microsecond timestamps.
    //
    //   ts_us | msg_type | bid_price | bid_qty | ask_price | ask_qty | trade_price | trade_qty | trade_side
    //   1710504600000000 | Q | 15025 | 300 | 15026 | 200 | 0 | 0 | 0
    //   1710504600000050 | T | 0 | 0 | 0 | 0 | 15026 | 100 | 1
    //

    let feed_config = FeedConfig {
        schema: FeedSchema {
            timestamp: "ts_us".into(),
            msg_type: MsgTypeSchema::Column {
                name: "msg_type".into(),
                trade_value: "T".into(),
                quote_value: "Q".into(),
            },
            bid_price: "bid_price".into(),
            bid_qty: "bid_qty".into(),
            ask_price: "ask_price".into(),
            ask_qty: "ask_qty".into(),
            trade_price: "trade_price".into(),
            trade_qty: "trade_qty".into(),
            trade_side: "trade_side".into(),
            symbol: Some("symbol".into()),
        },
        price_scale: 100.0,  // e.g., 150.25 → 15025
        side_encoding: SideEncoding::Numeric,
        time_range: None,  // load all
        symbol_filter: Some("AAPL".into()),
    };

    // ─── Load feed ────────────────────────────────────────────────────────

    tracing::info!("Loading feed...");
    let load_start = Instant::now();

    // Choose one:
    // let mut feed = Feed::from_parquet("data/l1_20240315.parquet", &feed_config).unwrap();
    // let mut feed = Feed::from_csv("data/l1_20240315.csv", &feed_config).unwrap();
    // let mut feed = Feed::from_parquet_glob("data/l1_202403*.parquet", &feed_config).unwrap();

    // For demo, create a synthetic feed
    let mut feed = create_demo_feed();

    let load_elapsed = load_start.elapsed();
    tracing::info!(
        rows = feed.len(),
        trades = feed.trade_count(),
        quotes = feed.quote_count(),
        load_ms = load_elapsed.as_millis(),
        memory_mb = format!("{:.1}", feed.memory_bytes() as f64 / 1_048_576.0),
        "Feed loaded"
    );

    // ─── Benchmark raw iteration ──────────────────────────────────────────

    let bench_start = Instant::now();
    let mut event_count = 0u64;
    while feed.next_event().is_some() {
        event_count += 1;
    }
    let bench_elapsed = bench_start.elapsed();
    let events_per_sec = event_count as f64 / bench_elapsed.as_secs_f64();
    tracing::info!(
        events = event_count,
        elapsed_us = bench_elapsed.as_micros(),
        throughput = format!("{:.1}M events/sec", events_per_sec / 1e6),
        "Raw iteration benchmark"
    );
    feed.reset();

    // ─── Configure simulator ──────────────────────────────────────────────

    let instrument = Instrument {
        symbol: "AAPL".into(),
        tick_size: 0.01,
        tick_value: 0.01,
        lot_size: 1,
        price_scale: 100.0,
        maker_fee_per_unit: -0.002,  // rebate
        taker_fee_per_unit: 0.003,
    };

    let sim_config = SimConfig {
        latency_us: 50,
        queue_model: QueueModel::Fifo,
        allow_immediate_cross: true,
        max_position: 500,
        instrument: instrument.clone(),
        max_fill_per_trade: 0,
    };

    let mut sim = Simulator::new(sim_config);

    // ─── Create strategy ──────────────────────────────────────────────────

    let mut strategy = SimpleMarketMaker::new(
        5,    // 5 ticks from mid ($0.05)
        100,  // 100 shares per side
        300,  // max 300 position
    );

    // ─── Run simulation ───────────────────────────────────────────────────

    tracing::info!("Running simulation...");
    let sim_start = Instant::now();

    sim.run(&mut strategy, &mut feed);

    let sim_elapsed = sim_start.elapsed();
    let stats = sim.stats();

    tracing::info!(
        elapsed_ms = sim_elapsed.as_millis(),
        orders = stats.orders_submitted,
        fills = stats.total_fills,
        qty_filled = stats.total_qty_filled,
        maker_ratio = format!("{:.1}%", stats.maker_ratio() * 100.0),
        fees = format!("{:.4}", stats.total_fees),
        "Simulation complete"
    );

    // ─── Compute PnL ─────────────────────────────────────────────────────

    let pnl_calc = PnlCalculator::new(instrument);
    let realized = pnl_calc.realized_pnl(sim.fills());
    let position = sim.position();

    tracing::info!(
        realized_pnl = format!("{:.4}", realized),
        final_position = position,
        "PnL summary"
    );

    if let Some(buy_vwap) = pnl_calc.vwap(sim.fills(), Side::Buy) {
        tracing::info!(buy_vwap = format!("{:.4}", buy_vwap), "Buy VWAP");
    }
    if let Some(sell_vwap) = pnl_calc.vwap(sim.fills(), Side::Sell) {
        tracing::info!(sell_vwap = format!("{:.4}", sell_vwap), "Sell VWAP");
    }
}

/// Create a synthetic demo feed for testing without a data file.
fn create_demo_feed() -> execution_backtest::feed::Feed {
    use execution_backtest::feed::MsgType;

    let n_quotes = 100_000;
    let n_trades = 20_000;
    let total = n_quotes + n_trades;

    let mut timestamps = Vec::with_capacity(total);
    let mut msg_types = Vec::with_capacity(total);
    let mut bid_prices = Vec::with_capacity(total);
    let mut bid_qtys = Vec::with_capacity(total);
    let mut ask_prices = Vec::with_capacity(total);
    let mut ask_qtys = Vec::with_capacity(total);
    let mut trade_prices = Vec::with_capacity(total);
    let mut trade_qtys = Vec::with_capacity(total);
    let mut trade_sides = Vec::with_capacity(total);

    let base_ts: i64 = 1_710_504_600_000_000; // some start time in micros
    let mut ts = base_ts;
    let mut mid: i64 = 15000; // $150.00 in cents

    let mut rng_state: u64 = 42;
    let mut cheap_rand = || -> u64 {
        rng_state ^= rng_state << 13;
        rng_state ^= rng_state >> 7;
        rng_state ^= rng_state << 17;
        rng_state
    };

    for _ in 0..total {
        ts += 10 + (cheap_rand() % 100) as i64; // 10-110 μs between events
        timestamps.push(ts);

        // ~17% trades, ~83% quotes (roughly matching real data)
        let is_trade = (cheap_rand() % 6) == 0;

        if is_trade {
            msg_types.push(MsgType::Trade);
            let side: i8 = if cheap_rand() % 2 == 0 { 1 } else { -1 };
            let price = if side == 1 {
                mid + 1 // buy → hits ask
            } else {
                mid - 1 // sell → hits bid
            };

            bid_prices.push(mid - 1);
            bid_qtys.push(0);
            ask_prices.push(mid + 1);
            ask_qtys.push(0);
            trade_prices.push(price);
            trade_qtys.push(50 + (cheap_rand() % 200) as i64);
            trade_sides.push(side);
        } else {
            msg_types.push(MsgType::Quote);
            // Random walk mid price
            let step = (cheap_rand() % 3) as i64 - 1; // -1, 0, +1
            mid += step;
            mid = mid.max(14900).min(15100); // bound it

            let spread = 1 + (cheap_rand() % 3) as i64; // 1-3 ticks
            bid_prices.push(mid - spread);
            bid_qtys.push(100 + (cheap_rand() % 500) as i64);
            ask_prices.push(mid + spread);
            ask_qtys.push(100 + (cheap_rand() % 500) as i64);
            trade_prices.push(0);
            trade_qtys.push(0);
            trade_sides.push(0);
        }
    }

    execution_backtest::feed::Feed {
        timestamps,
        msg_types,
        bid_prices,
        bid_qtys,
        ask_prices,
        ask_qtys,
        trade_prices,
        trade_qtys,
        trade_sides,
        len: total,
        cursor: 0,
    }
}
