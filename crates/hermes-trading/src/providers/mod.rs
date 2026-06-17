//! Concrete market data provider implementations.

pub mod binance;
pub mod binance_quote;
pub mod eastmoney;
pub mod eastmoney_quote;
#[cfg(any(test, feature = "test-mock"))]
pub mod mock;
#[cfg(any(test, feature = "test-mock"))]
pub mod quote_mock;
pub mod quote_router;
pub mod router;
pub mod stub;
pub mod yahoo;

pub use binance::BinanceProvider;
pub use binance_quote::BinanceQuoteProvider;
pub use eastmoney::EastmoneyProvider;
pub use eastmoney_quote::EastmoneyQuoteProvider;
#[cfg(any(test, feature = "test-mock"))]
pub use mock::MockProvider;
#[cfg(any(test, feature = "test-mock"))]
pub use quote_mock::MockQuoteProvider;
pub use quote_router::{QuoteRouter, QuoteSource};
pub use router::{AutoRouter, DataSource};
pub use stub::StubProvider;
pub use yahoo::YahooProvider;
