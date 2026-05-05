//! Polars-based feed reader for interleaved trade/quote L1 data.
//!
//! Assumes your data has a message type indicator that distinguishes trade rows
//! from quote rows. All rows have a microsecond timestamp.
//!
//! Supported layouts:
//!   1. "unified" — single file, `msg_type` column = "T" or "Q"
//!   2. "split" — separate trade and quote files merged by timestamp
//!   3. "flat" — every row has all fields, trade fields are 0/null when it's a quote

use polars::prelude::*;

use crate::types::*;
use crate::event::*;

// ─── Configuration ─────────────────────────────────────────────────────────────

/// Column name mapping for your data schema.
#[derive(Debug, Clone)]
pub struct FeedSchema {
    // Required columns
    pub timestamp: String,

    // Message type discriminator
    pub msg_type: MsgTypeSchema,

    // Quote columns
    pub bid_price: String,
    pub bid_qty: String,
    pub ask_price: String,
    pub ask_qty: String,

    // Trade columns
    pub trade_price: String,
    pub trade_qty: String,
    pub trade_side: String,  // column values: 1/-1 or "B"/"S" or "buy"/"sell"

    // Optional
    pub symbol: Option<String>,
}

/// How to distinguish trade vs quote rows.
#[derive(Debug, Clone)]
pub enum MsgTypeSchema {
    /// A column holds "T"/"Q" or "trade"/"quote" etc.
    Column {
        name: String,
        trade_value: String,  // e.g., "T" or "trade"
        quote_value: String,  // e.g., "Q" or "quote"
    },
    /// No discriminator — use presence of trade_qty > 0 to detect trades.
    /// Quote rows have trade_qty = 0 or null.
    InferFromQty,
    /// All rows are the same type.
    AllQuotes,
    AllTrades,
}

#[derive(Debug, Clone)]
pub struct FeedConfig {
    pub schema: FeedSchema,

    /// Price scaling factor. Set to 1.0 if prices are already integers.
    /// If your data has prices like 150.25, set this to 100.0 to get 15025.
    pub price_scale: f64,

    /// Trade side encoding in data.
    pub side_encoding: SideEncoding,

    /// Time filter (microseconds). None = load all.
    pub time_range: Option<(TsMicros, TsMicros)>,

    /// Symbol filter for multi-symbol files.
    pub symbol_filter: Option<String>,
}

#[derive(Debug, Clone)]
pub enum SideEncoding {
    /// Numeric: 1 = buy, -1 = sell, 0 = unknown
    Numeric,
    /// String: "B"/"S" or "buy"/"sell"
    StringBS,
    /// Infer from quote (Lee-Ready)
    Infer,
}

impl Default for FeedSchema {
    fn default() -> Self {
        Self {
            timestamp: "ts_us".into(),
            msg_type: MsgTypeSchema::InferFromQty,
            bid_price: "bid_price".into(),
            bid_qty: "bid_qty".into(),
            ask_price: "ask_price".into(),
            ask_qty: "ask_qty".into(),
            trade_price: "trade_price".into(),
            trade_qty: "trade_qty".into(),
            trade_side: "trade_side".into(),
            symbol: None,
        }
    }
}

impl Default for FeedConfig {
    fn default() -> Self {
        Self {
            schema: FeedSchema::default(),
            price_scale: 1.0,
            side_encoding: SideEncoding::Numeric,
            time_range: None,
            symbol_filter: None,
        }
    }
}

// ─── Feed struct ───────────────────────────────────────────────────────────────

/// The high-performance feed. Data is stored in columnar arrays for cache efficiency.
/// After loading via Polars, iteration is a tight loop over contiguous memory.
pub struct Feed {
    // Per-row data
    pub timestamps: Vec<TsMicros>,
    pub msg_types: Vec<MsgType>,     // discriminator per row

    // Quote fields (valid when msg_type == Quote)
    pub bid_prices: Vec<Price>,
    pub bid_qtys: Vec<Qty>,
    pub ask_prices: Vec<Price>,
    pub ask_qtys: Vec<Qty>,

    // Trade fields (valid when msg_type == Trade)
    pub trade_prices: Vec<Price>,
    pub trade_qtys: Vec<Qty>,
    pub trade_sides: Vec<i8>,        // +1 buy, -1 sell, 0 unknown

    // Metadata
    pub len: usize,
    pub cursor: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MsgType {
    Quote = 0,
    Trade = 1,
}

impl Feed {
    // ─── Constructors ──────────────────────────────────────────────────────

    /// Load from a Parquet file. Fastest path — column pruning + row group skipping.
    pub fn from_parquet(path: &str, config: &FeedConfig) -> Result<Self, FeedError> {
        let lf = LazyFrame::scan_parquet(
            path.into(),
            ScanArgsParquet {
                n_rows: None,
                parallel: polars::io::parquet::read::ParallelStrategy::Auto,
                ..Default::default()
            },
        )
        .map_err(|e| FeedError::Polars(e))?;

        Self::from_lazy(lf, config)
    }

    /// Load from a CSV file.
    pub fn from_csv(path: &str, config: &FeedConfig) -> Result<Self, FeedError> {
        let lf = LazyCsvReader::new(path.into())
            .with_has_header(true)
            .with_try_parse_dates(false)
            .finish()
            .map_err(|e| FeedError::Polars(e))?;

        Self::from_lazy(lf, config)
    }

    /// Load from multiple Parquet files (glob pattern).
    pub fn from_parquet_glob(pattern: &str, config: &FeedConfig) -> Result<Self, FeedError> {
        let lf = LazyFrame::scan_parquet(
            pattern.into(),
            ScanArgsParquet {
                glob: true,
                parallel: polars::io::parquet::read::ParallelStrategy::Auto,
                ..Default::default()
            },
        )
        .map_err(|e| FeedError::Polars(e))?;

        Self::from_lazy(lf, config)
    }

    /// Load from two separate files (trades + quotes) and merge by timestamp.
    pub fn from_split_files(
        quotes_path: &str,
        trades_path: &str,
        config: &FeedConfig,
    ) -> Result<Self, FeedError> {
        let quotes_lf = LazyFrame::scan_parquet(
            quotes_path.into(),
            ScanArgsParquet::default(),
        )
        .map_err(|e| FeedError::Polars(e))?
        .with_column(lit("Q").alias("__msg_type"));

        let trades_lf = LazyFrame::scan_parquet(
            trades_path.into(),
            ScanArgsParquet::default(),
        )
        .map_err(|e| FeedError::Polars(e))?
        .with_column(lit("T").alias("__msg_type"));

        // Concatenate and sort by timestamp
        let lf = concat(
            [quotes_lf, trades_lf],
            UnionArgs {
                parallel: true,
                ..Default::default()
            },
        )
        .map_err(|e| FeedError::Polars(e))?;

        // Use the injected __msg_type column
        let mut config = config.clone();
        config.schema.msg_type = MsgTypeSchema::Column {
            name: "__msg_type".into(),
            trade_value: "T".into(),
            quote_value: "Q".into(),
        };

        Self::from_lazy(lf, &config)
    }

    // ─── Core loading logic ────────────────────────────────────────────────

    fn from_lazy(lf: LazyFrame, config: &FeedConfig) -> Result<Self, FeedError> {
        let schema = &config.schema;
        let mut lf = lf;

        // Apply time filter — pushes down to row group skipping in parquet
        if let Some((start, end)) = config.time_range {
            lf = lf.filter(
                col(&schema.timestamp)
                    .gt_eq(lit(start))
                    .and(col(&schema.timestamp).lt_eq(lit(end))),
            );
        }

        // Apply symbol filter
        if let (Some(sym_col), Some(sym_val)) = (&schema.symbol, &config.symbol_filter) {
            lf = lf.filter(col(sym_col).eq(lit(sym_val.as_str())));
        }

        // Sort by timestamp — ensures correct event ordering
        lf = lf.sort(
            [&schema.timestamp],
            SortMultipleOptions::default(),
        );

        // Collect the DataFrame
        let df = lf.collect().map_err(|e| FeedError::Polars(e))?;
        let len = df.height();

        if len == 0 {
            return Ok(Self::empty());
        }

        // ── Extract timestamps ─────────────────────────────────────────────
        let timestamps = extract_i64_vec(&df, &schema.timestamp)?;

        // ── Determine message types ────────────────────────────────────────
        let msg_types = Self::extract_msg_types(&df, &schema.msg_type, &schema.trade_qty)?;

        // ── Extract quote fields ───────────────────────────────────────────
        let bid_prices = extract_price_vec(&df, &schema.bid_price, config.price_scale)?;
        let bid_qtys = extract_i64_vec_or_zeros(&df, &schema.bid_qty, len);
        let ask_prices = extract_price_vec(&df, &schema.ask_price, config.price_scale)?;
        let ask_qtys = extract_i64_vec_or_zeros(&df, &schema.ask_qty, len);

        // ── Extract trade fields ───────────────────────────────────────────
        let trade_prices = extract_price_vec_or_zeros(&df, &schema.trade_price, config.price_scale, len);
        let trade_qtys = extract_i64_vec_or_zeros(&df, &schema.trade_qty, len);

        // ── Extract or infer trade sides ───────────────────────────────────
        let trade_sides = match &config.side_encoding {
            SideEncoding::Numeric => {
                extract_i8_vec_or_zeros(&df, &schema.trade_side, len)
            }
            SideEncoding::StringBS => {
                Self::parse_string_sides(&df, &schema.trade_side, len)
            }
            SideEncoding::Infer => {
                infer_trade_sides(&bid_prices, &ask_prices, &trade_prices, &trade_qtys)
            }
        };

        Ok(Self {
            timestamps,
            msg_types,
            bid_prices,
            bid_qtys,
            ask_prices,
            ask_qtys,
            trade_prices,
            trade_qtys,
            trade_sides,
            len,
            cursor: 0,
        })
    }

    fn extract_msg_types(
        df: &DataFrame,
        schema: &MsgTypeSchema,
        trade_qty_col: &str,
    ) -> Result<Vec<MsgType>, FeedError> {
        let len = df.height();

        match schema {
            MsgTypeSchema::Column { name, trade_value, .. } => {
                let series = df.column(name).map_err(|e| FeedError::Polars(e))?;
                let ca = series.str().map_err(|e| FeedError::Polars(e))?;
                let mut types = Vec::with_capacity(len);
                for val in ca.into_iter() {
                    let t = match val {
                        Some(s) if s == trade_value => MsgType::Trade,
                        _ => MsgType::Quote,
                    };
                    types.push(t);
                }
                Ok(types)
            }
            MsgTypeSchema::InferFromQty => {
                // If trade_qty > 0 → trade, else → quote
                if let Ok(series) = df.column(trade_qty_col) {
                    let vals = series_to_i64_vec(series.as_materialized_series(), len);
                    Ok(vals.iter().map(|&v| {
                        if v > 0 { MsgType::Trade } else { MsgType::Quote }
                    }).collect())
                } else {
                    // Column doesn't exist → all quotes
                    Ok(vec![MsgType::Quote; len])
                }
            }
            MsgTypeSchema::AllQuotes => Ok(vec![MsgType::Quote; len]),
            MsgTypeSchema::AllTrades => Ok(vec![MsgType::Trade; len]),
        }
    }

    fn parse_string_sides(df: &DataFrame, col_name: &str, len: usize) -> Vec<i8> {
        let Ok(series) = df.column(col_name) else {
            return vec![0i8; len];
        };
        let Ok(ca) = series.str() else {
            return vec![0i8; len];
        };

        ca.into_iter()
            .map(|opt| match opt {
                Some("B") | Some("b") | Some("buy") | Some("Buy") | Some("BUY") => 1i8,
                Some("S") | Some("s") | Some("sell") | Some("Sell") | Some("SELL") => -1i8,
                _ => 0i8,
            })
            .collect()
    }

    fn empty() -> Self {
        Self {
            timestamps: vec![],
            msg_types: vec![],
            bid_prices: vec![],
            bid_qtys: vec![],
            ask_prices: vec![],
            ask_qtys: vec![],
            trade_prices: vec![],
            trade_qtys: vec![],
            trade_sides: vec![],
            len: 0,
            cursor: 0,
        }
    }

    // ─── Iteration ─────────────────────────────────────────────────────────

    /// Get the next market event. This is the hot path — no allocations.
    #[inline(always)]
    pub fn next_event(&mut self) -> Option<MarketEvent> {
        if self.cursor >= self.len {
            return None;
        }

        let i = self.cursor;
        self.cursor += 1;

        // SAFETY: i < self.len, all vecs have length == self.len
        unsafe { Some(self.read_event_unchecked(i)) }
    }

    /// Read event at index without bounds checking.
    #[inline(always)]
    unsafe fn read_event_unchecked(&self, i: usize) -> MarketEvent {
        let ts = *unsafe { self.timestamps.get_unchecked(i) };
        let msg_type = *unsafe { self.msg_types.get_unchecked(i) };

        match msg_type {
            MsgType::Quote => MarketEvent::Quote(QuoteEvent {
                ts,
                bid_price: *unsafe { self.bid_prices.get_unchecked(i) },
                bid_qty: *unsafe { self.bid_qtys.get_unchecked(i) },
                ask_price: *unsafe { self.ask_prices.get_unchecked(i) },
                ask_qty: *unsafe { self.ask_qtys.get_unchecked(i) },
            }),
            MsgType::Trade => MarketEvent::Trade(TradeEvent {
                ts,
                price: *unsafe { self.trade_prices.get_unchecked(i) },
                qty: *unsafe { self.trade_qtys.get_unchecked(i) },
                aggressor: match *unsafe { self.trade_sides.get_unchecked(i) } {
                    1 => Aggressor::Buy,
                    -1 => Aggressor::Sell,
                    _ => Aggressor::Unknown,
                },
            }),
        }
    }

    /// Peek at the next timestamp without advancing.
    #[inline(always)]
    pub fn peek_ts(&self) -> Option<TsMicros> {
        if self.cursor < self.len {
            Some(unsafe { *self.timestamps.get_unchecked(self.cursor) })
        } else {
            None
        }
    }

    /// Peek at the next event without advancing.
    #[inline(always)]
    pub fn peek_event(&self) -> Option<MarketEvent> {
        if self.cursor < self.len {
            Some(unsafe { self.read_event_unchecked(self.cursor) })
        } else {
            None
        }
    }

    /// Reset to beginning for another pass.
    #[inline]
    pub fn reset(&mut self) {
        self.cursor = 0;
    }

    /// Skip forward to the first event at or after `target_ts`.
    /// Uses binary search — O(log n).
    pub fn seek_to(&mut self, target_ts: TsMicros) {
        let pos = self.timestamps.partition_point(|&ts| ts < target_ts);
        self.cursor = pos;
    }

    // ─── Metadata ──────────────────────────────────────────────────────────

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn remaining(&self) -> usize {
        self.len - self.cursor
    }

    pub fn time_range(&self) -> Option<(TsMicros, TsMicros)> {
        if self.is_empty() {
            None
        } else {
            Some((self.timestamps[0], self.timestamps[self.len - 1]))
        }
    }

    pub fn trade_count(&self) -> usize {
        self.msg_types.iter().filter(|&&t| t == MsgType::Trade).count()
    }

    pub fn quote_count(&self) -> usize {
        self.msg_types.iter().filter(|&&t| t == MsgType::Quote).count()
    }

    /// Memory usage in bytes (approximate).
    pub fn memory_bytes(&self) -> usize {
        // 8 bytes each: timestamps, bid_prices, bid_qtys, ask_prices, ask_qtys, trade_prices, trade_qtys
        // 1 byte each: msg_types, trade_sides
        self.len * (7 * 8 + 2 * 1)
    }
}

// ─── Iterator adapter ──────────────────────────────────────────────────────────

impl Iterator for Feed {
    type Item = MarketEvent;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        self.next_event()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.remaining();
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for Feed {}

// ─── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum FeedError {
    #[error("Polars error: {0}")]
    Polars(#[from] PolarsError),

    #[error("Missing column: {0}")]
    MissingColumn(String),

    #[error("Invalid data: {0}")]
    InvalidData(String),
}

// ─── Column extraction helpers ─────────────────────────────────────────────────

fn extract_i64_vec(df: &DataFrame, name: &str) -> Result<Vec<i64>, FeedError> {
    let series = df.column(name).map_err(|_| FeedError::MissingColumn(name.into()))?;
    Ok(series_to_i64_vec(series.as_materialized_series(), df.height()))
}

fn extract_price_vec(df: &DataFrame, name: &str, scale: f64) -> Result<Vec<Price>, FeedError> {
    let series = df.column(name).map_err(|_| FeedError::MissingColumn(name.into()))?;

    if scale == 1.0 {
        return Ok(series_to_i64_vec(series.as_materialized_series(), df.height()));
    }

    let len = df.height();
    match series.dtype() {
        DataType::Float64 => {
            let ca = series.f64().unwrap();
            Ok(ca.into_no_null_iter().map(|v| (v * scale).round() as i64).collect())
        }
        DataType::Float32 => {
            let ca = series.f32().unwrap();
            Ok(ca.into_no_null_iter().map(|v| (v as f64 * scale).round() as i64).collect())
        }
        _ => {
            let casted = series.cast(&DataType::Float64).unwrap_or_else(|_| {
                Column::new(name.into(), vec![0.0f64; len])
            });
            let ca = casted.f64().unwrap();
            Ok(ca.into_no_null_iter().map(|v| (v * scale).round() as i64).collect())
        }
    }
}

fn extract_price_vec_or_zeros(df: &DataFrame, name: &str, scale: f64, len: usize) -> Vec<Price> {
    extract_price_vec(df, name, scale).unwrap_or_else(|_| vec![0; len])
}

fn extract_i64_vec_or_zeros(df: &DataFrame, name: &str, len: usize) -> Vec<i64> {
    extract_i64_vec(df, name).unwrap_or_else(|_| vec![0; len])
}

fn extract_i8_vec_or_zeros(df: &DataFrame, name: &str, len: usize) -> Vec<i8> {
    let Ok(series) = df.column(name) else {
        return vec![0i8; len];
    };

    match series.dtype() {
        DataType::Int8 => {
            series.i8().unwrap().into_no_null_iter().collect()
        }
        DataType::Int16 => {
            series.i16().unwrap().into_no_null_iter().map(|v| v as i8).collect()
        }
        DataType::Int32 => {
            series.i32().unwrap().into_no_null_iter().map(|v| v as i8).collect()
        }
        DataType::Int64 => {
            series.i64().unwrap().into_no_null_iter().map(|v| v as i8).collect()
        }
        _ => {
            let casted = series.cast(&DataType::Int8).unwrap_or_else(|_| {
                Column::new(name.into(), vec![0i8; len])
            });
            casted.i8().unwrap().into_no_null_iter().collect()
        }
    }
}

fn series_to_i64_vec(series: &Series, len: usize) -> Vec<i64> {
    match series.dtype() {
        DataType::Int64 => series.i64().unwrap().into_no_null_iter().collect(),
        DataType::Int32 => series.i32().unwrap().into_no_null_iter().map(|v| v as i64).collect(),
        DataType::UInt64 => series.u64().unwrap().into_no_null_iter().map(|v| v as i64).collect(),
        DataType::UInt32 => series.u32().unwrap().into_no_null_iter().map(|v| v as i64).collect(),
        DataType::Float64 => series.f64().unwrap().into_no_null_iter().map(|v| v as i64).collect(),
        DataType::Float32 => series.f32().unwrap().into_no_null_iter().map(|v| v as i64).collect(),
        _ => {
            let casted = series.cast(&DataType::Int64).unwrap_or_else(|_| {
                Series::new(series.name().clone(), vec![0i64; len])
            });
            casted.i64().unwrap().into_no_null_iter().collect()
        }
    }
}

/// Lee-Ready trade side inference: compare trade price to mid.
fn infer_trade_sides(
    bid_prices: &[Price],
    ask_prices: &[Price],
    trade_prices: &[Price],
    trade_qtys: &[Qty],
) -> Vec<i8> {
    let n = trade_prices.len();
    let mut sides = vec![0i8; n];
    let mut prev_price: Price = 0;

    for i in 0..n {
        if trade_qtys[i] == 0 {
            continue; // quote row, leave as 0
        }

        let tp = trade_prices[i];
        let mid = (bid_prices[i] + ask_prices[i]) / 2;

        sides[i] = if tp > mid {
            1
        } else if tp < mid {
            -1
        } else if tp > prev_price {
            1
        } else if tp < prev_price {
            -1
        } else {
            0
        };

        prev_price = tp;
    }

    sides
}