//! Centralized Eastmoney / Sina API field labels (Chinese column names).

/// Default industry when basic dim has no sector.
pub const DEFAULT_INDUSTRY: &str = "综合";

/// Sina financial abstract / indicator row names.
pub mod financials {
    pub const ROE: &str = "净资产收益率";
    pub const ROE_PCT: &str = "净资产收益率(%)";
    pub const NET_MARGIN: &str = "销售净利率";
    pub const NET_MARGIN_PCT: &str = "销售净利率(%)";
    pub const GROSS_MARGIN: &str = "销售毛利率";
    pub const REVENUE: &str = "营业总收入";
    pub const NET_PROFIT_PARENT: &str = "归属于母公司所有者的净利润";
    pub const NET_PROFIT: &str = "净利润";
    pub const DEBT_RATIO_PCT: &str = "资产负债率(%)";
    pub const CURRENT_RATIO: &str = "流动比率";
    pub const FCF: &str = "企业自由现金流量";
}

/// Fund holder table column names.
pub mod fund_holders {
    pub const FUND_NAME: &str = "基金名称";
    pub const HOLDER_NAME: &str = "股东名称";
    pub const FLOAT_RATIO: &str = "占流通股比例";
    pub const HOLD_RATIO: &str = "持股比例";
}
