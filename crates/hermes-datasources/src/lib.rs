//! Data source provider factory for Terra.

pub mod akshare_cloud;
pub mod akshare_local;
pub mod registry;
pub mod types;
pub mod user_custom;

pub use akshare_cloud::AkshareCloudDataSource;
pub use akshare_local::AkshareLocalDataSource;
pub use registry::DataSourceRegistry;
pub use types::*;
pub use user_custom::UserCustomDataSource;
