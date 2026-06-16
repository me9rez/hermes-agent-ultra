//! Concrete market data provider implementations.

pub mod binance;
pub mod eastmoney;
#[cfg(any(test, feature = "test-mock"))]
pub mod mock;
pub mod router;

pub use binance::BinanceProvider;
pub use eastmoney::EastmoneyProvider;
#[cfg(any(test, feature = "test-mock"))]
pub use mock::MockProvider;
pub use router::{AutoRouter, DataSource};
