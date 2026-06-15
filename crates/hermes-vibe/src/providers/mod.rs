//! Concrete market data provider implementations.

pub mod binance;
pub mod eastmoney;
pub mod router;

pub use binance::BinanceProvider;
pub use eastmoney::EastmoneyProvider;
pub use router::AutoRouter;
