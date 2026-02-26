use serde::{Deserialize, Serialize};
use mongodb::bson::oid::ObjectId;

// ═══════════════════════════════════════════════════════════════════
// STRATEGY STATUS
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StrategyStatus {
    Idle,
    Monitoring,
    BuyPending,
    InPosition,
    SellPending,
    Paused,
    Completed,
    Error,
}

impl Default for StrategyStatus {
    fn default() -> Self {
        StrategyStatus::Idle
    }
}

impl std::fmt::Display for StrategyStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StrategyStatus::Idle => write!(f, "idle"),
            StrategyStatus::Monitoring => write!(f, "monitoring"),
            StrategyStatus::BuyPending => write!(f, "buy_pending"),
            StrategyStatus::InPosition => write!(f, "in_position"),
            StrategyStatus::SellPending => write!(f, "sell_pending"),
            StrategyStatus::Paused => write!(f, "paused"),
            StrategyStatus::Completed => write!(f, "completed"),
            StrategyStatus::Error => write!(f, "error"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// CONFIG STRUCTS
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TakeProfitLevel {
    pub percent: f64,
    pub sell_percent: f64,
    #[serde(default)]
    pub executed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executed_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopLossConfig {
    pub enabled: bool,
    pub percent: f64,
    #[serde(default)]
    pub trailing: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trailing_callback: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trailing_distance: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DcaConfig {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_buys: Option<i32>,
    #[serde(default)]
    pub buys_done: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dip_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount_per_buy: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridConfig {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub levels: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spacing_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub center_price: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyConfig {
    #[serde(default)]
    pub take_profit_levels: Vec<TakeProfitLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_loss: Option<StopLossConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dca: Option<DcaConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<GridConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_investment: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_daily_operations: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_close_time: Option<i64>,
    #[serde(default = "default_mode")]
    pub mode: String,
}

fn default_mode() -> String {
    "spot".to_string()
}

impl Default for StrategyConfig {
    fn default() -> Self {
        StrategyConfig {
            take_profit_levels: vec![],
            stop_loss: None,
            dca: None,
            grid: None,
            min_investment: None,
            max_daily_operations: None,
            auto_close_time: None,
            mode: "spot".to_string(),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// EXECUTIONS
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionAction {
    Buy,
    Sell,
    BuyFailed,
    SellFailed,
    DcaBuy,
    GridBuy,
    GridSell,
}

impl std::fmt::Display for ExecutionAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutionAction::Buy => write!(f, "buy"),
            ExecutionAction::Sell => write!(f, "sell"),
            ExecutionAction::BuyFailed => write!(f, "buy_failed"),
            ExecutionAction::SellFailed => write!(f, "sell_failed"),
            ExecutionAction::DcaBuy => write!(f, "dca_buy"),
            ExecutionAction::GridBuy => write!(f, "grid_buy"),
            ExecutionAction::GridSell => write!(f, "grid_sell"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyExecution {
    pub execution_id: String,
    pub action: ExecutionAction,
    pub reason: String,
    pub price: f64,
    pub amount: f64,
    pub total: f64,
    #[serde(default)]
    pub fee: f64,
    #[serde(default)]
    pub pnl_usd: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exchange_order_id: Option<String>,
    pub executed_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

// ═══════════════════════════════════════════════════════════════════
// SIGNALS
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SignalType {
    Buy,
    TakeProfit,
    StopLoss,
    TrailingStop,
    DcaBuy,
    GridTrade,
    Info,
    PriceAlert,
}

impl std::fmt::Display for SignalType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignalType::Buy => write!(f, "buy"),
            SignalType::TakeProfit => write!(f, "take_profit"),
            SignalType::StopLoss => write!(f, "stop_loss"),
            SignalType::TrailingStop => write!(f, "trailing_stop"),
            SignalType::DcaBuy => write!(f, "dca_buy"),
            SignalType::GridTrade => write!(f, "grid_trade"),
            SignalType::Info => write!(f, "info"),
            SignalType::PriceAlert => write!(f, "price_alert"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategySignal {
    pub signal_type: SignalType,
    pub price: f64,
    pub message: String,
    #[serde(default)]
    pub acted: bool,
    #[serde(default)]
    pub price_change_percent: f64,
    pub created_at: i64,
}

// ═══════════════════════════════════════════════════════════════════
// POSITION INFO
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionInfo {
    pub entry_price: f64,
    pub quantity: f64,
    pub total_cost: f64,
    #[serde(default)]
    pub current_price: f64,
    #[serde(default)]
    pub unrealized_pnl: f64,
    #[serde(default)]
    pub unrealized_pnl_percent: f64,
    #[serde(default)]
    pub highest_price: f64,
    #[serde(default)]
    pub lowest_price: f64,
    pub opened_at: i64,
}

// ═══════════════════════════════════════════════════════════════════
// USER STRATEGIES — 1 doc per user (padrão UserExchanges)
// Collection: "user_strategies"
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserStrategies {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub user_id: String,
    #[serde(default)]
    pub strategies: Vec<StrategyItem>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyItem {
    pub strategy_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub strategy_type: String,
    pub symbol: String,
    pub exchange_id: String,
    pub exchange_name: String,
    #[serde(default = "default_true")]
    pub is_active: bool,
    #[serde(default)]
    pub status: StrategyStatus,
    #[serde(default)]
    pub config: StrategyConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_legacy: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<PositionInfo>,
    #[serde(default)]
    pub executions: Vec<StrategyExecution>,
    #[serde(default)]
    pub signals: Vec<StrategySignal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_price: Option<f64>,
    #[serde(default = "default_check_interval")]
    pub check_interval_secs: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default)]
    pub total_pnl_usd: f64,
    #[serde(default)]
    pub total_executions: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

fn default_true() -> bool {
    true
}

fn default_check_interval() -> i64 {
    60
}

// ═══════════════════════════════════════════════════════════════════
// REQUEST / RESPONSE DTOs
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
pub struct CreateStrategyRequest {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub strategy_type: String,
    pub symbol: String,
    pub exchange_id: String,
    pub exchange_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<StrategyConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_legacy: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_interval_secs: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateStrategyRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub strategy_type: Option<String>,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub exchange_id: Option<String>,
    #[serde(default)]
    pub exchange_name: Option<String>,
    #[serde(default)]
    pub is_active: Option<bool>,
    #[serde(default)]
    pub status: Option<StrategyStatus>,
    #[serde(default)]
    pub config: Option<StrategyConfig>,
    #[serde(default)]
    pub config_legacy: Option<serde_json::Value>,
    #[serde(default)]
    pub check_interval_secs: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StrategyStatsResponse {
    pub total_executions: i32,
    pub total_buys: i32,
    pub total_sells: i32,
    pub total_pnl_usd: f64,
    pub win_rate: f64,
    pub avg_profit_per_trade: f64,
    pub total_fees: f64,
    pub last_execution_at: Option<i64>,
    pub last_signal_at: Option<i64>,
    pub days_active: i64,
    pub current_position: Option<PositionInfo>,
}

#[derive(Debug, Serialize)]
pub struct StrategyResponse {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub strategy_type: String,
    pub symbol: String,
    pub exchange_id: String,
    pub exchange_name: String,
    pub is_active: bool,
    pub status: StrategyStatus,
    pub config: StrategyConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_legacy: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<PositionInfo>,
    pub executions: Vec<StrategyExecution>,
    pub signals: Vec<StrategySignal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_price: Option<f64>,
    pub check_interval_secs: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub total_pnl_usd: f64,
    pub total_executions: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<StrategyStatsResponse>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl StrategyItem {
    pub fn compute_stats(&self) -> StrategyStatsResponse {
        let total_buys = self.executions.iter()
            .filter(|e| matches!(e.action, ExecutionAction::Buy | ExecutionAction::DcaBuy | ExecutionAction::GridBuy))
            .count() as i32;
        let total_sells = self.executions.iter()
            .filter(|e| matches!(e.action, ExecutionAction::Sell | ExecutionAction::GridSell))
            .count() as i32;
        let sell_executions: Vec<&StrategyExecution> = self.executions.iter()
            .filter(|e| matches!(e.action, ExecutionAction::Sell | ExecutionAction::GridSell))
            .collect();
        let total_fees: f64 = self.executions.iter().map(|e| e.fee).sum();
        let wins = sell_executions.iter().filter(|e| e.pnl_usd > 0.0).count();
        let win_rate = if sell_executions.is_empty() { 0.0 } else {
            (wins as f64 / sell_executions.len() as f64) * 100.0
        };
        let avg_profit = if sell_executions.is_empty() { 0.0 } else {
            sell_executions.iter().map(|e| e.pnl_usd).sum::<f64>() / sell_executions.len() as f64
        };
        let last_execution_at = self.executions.last().map(|e| e.executed_at);
        let last_signal_at = self.signals.last().map(|s| s.created_at);
        let now = chrono::Utc::now().timestamp();
        let days_active = (now - self.created_at) / 86400;
        StrategyStatsResponse {
            total_executions: self.executions.len() as i32,
            total_buys,
            total_sells,
            total_pnl_usd: self.total_pnl_usd,
            win_rate,
            avg_profit_per_trade: avg_profit,
            total_fees,
            last_execution_at,
            last_signal_at,
            days_active,
            current_position: self.position.clone(),
        }
    }
}

impl From<StrategyItem> for StrategyResponse {
    fn from(item: StrategyItem) -> Self {
        let stats = item.compute_stats();
        StrategyResponse {
            id: item.strategy_id.clone(),
            name: item.name,
            description: item.description,
            strategy_type: item.strategy_type,
            symbol: item.symbol,
            exchange_id: item.exchange_id,
            exchange_name: item.exchange_name,
            is_active: item.is_active,
            status: item.status,
            config: item.config,
            config_legacy: item.config_legacy,
            position: item.position,
            executions: item.executions,
            signals: item.signals,
            last_checked_at: item.last_checked_at,
            last_price: item.last_price,
            check_interval_secs: item.check_interval_secs,
            error_message: item.error_message,
            total_pnl_usd: item.total_pnl_usd,
            total_executions: item.total_executions,
            stats: Some(stats),
            created_at: item.created_at,
            updated_at: item.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct StrategyListItem {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub strategy_type: String,
    pub symbol: String,
    pub exchange_id: String,
    pub exchange_name: String,
    pub is_active: bool,
    pub status: StrategyStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<PositionInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<i64>,
    pub total_pnl_usd: f64,
    pub total_executions: i32,
    pub executions_count: usize,
    pub signals_count: usize,
    pub check_interval_secs: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<StrategyItem> for StrategyListItem {
    fn from(item: StrategyItem) -> Self {
        StrategyListItem {
            id: item.strategy_id.clone(),
            name: item.name,
            description: item.description,
            strategy_type: item.strategy_type,
            symbol: item.symbol,
            exchange_id: item.exchange_id,
            exchange_name: item.exchange_name,
            is_active: item.is_active,
            status: item.status,
            position: item.position,
            last_price: item.last_price,
            last_checked_at: item.last_checked_at,
            total_pnl_usd: item.total_pnl_usd,
            total_executions: item.total_executions,
            executions_count: item.executions.len(),
            signals_count: item.signals.len(),
            check_interval_secs: item.check_interval_secs,
            error_message: item.error_message,
            created_at: item.created_at,
            updated_at: item.updated_at,
        }
    }
}
