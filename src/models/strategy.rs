use serde::{Deserialize, Serialize};
use mongodb::bson::oid::ObjectId;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StrategyStatus {
    Idle,
    Monitoring,
    InPosition,
    GradualSelling,
    Completed,
    StoppedOut,
    Expired,
    Paused,
    Error,
}

impl Default for StrategyStatus {
    fn default() -> Self { StrategyStatus::Idle }
}

impl std::fmt::Display for StrategyStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StrategyStatus::Idle => write!(f, "idle"),
            StrategyStatus::Monitoring => write!(f, "monitoring"),
            StrategyStatus::InPosition => write!(f, "in_position"),
            StrategyStatus::GradualSelling => write!(f, "gradual_selling"),
            StrategyStatus::Completed => write!(f, "completed"),
            StrategyStatus::StoppedOut => write!(f, "stopped_out"),
            StrategyStatus::Expired => write!(f, "expired"),
            StrategyStatus::Paused => write!(f, "paused"),
            StrategyStatus::Error => write!(f, "error"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GradualLot {
    pub lot_number: i32,
    pub sell_percent: f64,
    #[serde(default)]
    pub executed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executed_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executed_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realized_pnl: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyConfig {
    /// Preço unitário da moeda no momento da compra (ex: SOL a $140).
    /// Usado para calcular trigger (TP), stop loss e % de variação.
    pub base_price: f64,
    /// Valor total investido em USDT (ex: $36). Opcional — usado para
    /// calcular PnL estimado em dólares nos sinais e logs.
    #[serde(default)]
    pub invested_amount: f64,
    pub take_profit_percent: f64,
    /// Se false, stop loss é completamente desativado — a estratégia nunca
    /// vende por queda de preço. Útil para hold de longo prazo.
    #[serde(default = "default_true")]
    pub stop_loss_enabled: bool,
    pub stop_loss_percent: f64,
    pub gradual_take_percent: f64,
    pub fee_percent: f64,
    #[serde(default)]
    pub gradual_sell: bool,
    #[serde(default)]
    pub gradual_lots: Vec<GradualLot>,
    #[serde(default = "default_timer_gradual")]
    pub timer_gradual_min: i64,
    #[serde(default = "default_time_execution")]
    pub time_execution_min: i64,
}

fn default_timer_gradual() -> i64 { 15 }
fn default_time_execution() -> i64 { 120 }

impl Default for StrategyConfig {
    fn default() -> Self {
        StrategyConfig {
            base_price: 0.0,
            invested_amount: 0.0,
            take_profit_percent: 10.0,
            stop_loss_enabled: true,
            stop_loss_percent: 5.0,
            gradual_take_percent: 2.0,
            fee_percent: 0.5,
            gradual_sell: false,
            gradual_lots: vec![],
            timer_gradual_min: 15,
            time_execution_min: 120,
        }
    }
}

impl StrategyConfig {
    pub fn trigger_price(&self) -> f64 {
        let tp_factor = self.take_profit_percent / 100.0;
        let fee_factor = self.fee_percent / 100.0;
        self.base_price * (1.0 + tp_factor + fee_factor)
    }

    pub fn stop_loss_price(&self) -> f64 {
        self.base_price * (1.0 - self.stop_loss_percent / 100.0)
    }

    pub fn gradual_trigger_price(&self, lot_index: usize) -> f64 {
        let base_tp = self.take_profit_percent / 100.0;
        let fee = self.fee_percent / 100.0;
        let gradual_step = self.gradual_take_percent / 100.0;
        self.base_price * (1.0 + base_tp + fee + gradual_step * lot_index as f64)
    }

    /// Calcula a quantidade estimada de moedas com base no investimento.
    /// Ex: invested $36, base_price $140 → 0.2571 moedas
    pub fn estimated_quantity(&self) -> f64 {
        if self.invested_amount > 0.0 && self.base_price > 0.0 {
            self.invested_amount / self.base_price
        } else {
            0.0
        }
    }

    /// Calcula PnL estimado em $ baseado no invested_amount e variação de preço.
    /// Ex: invested $36, preço subiu 10% → PnL estimado = +$3.60
    pub fn estimated_pnl(&self, current_price: f64) -> Option<f64> {
        if self.invested_amount > 0.0 && self.base_price > 0.0 {
            let qty = self.invested_amount / self.base_price;
            let current_value = qty * current_price;
            Some(current_value - self.invested_amount)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionAction {
    Buy,
    Sell,
    BuyFailed,
    SellFailed,
}

impl std::fmt::Display for ExecutionAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutionAction::Buy => write!(f, "buy"),
            ExecutionAction::Sell => write!(f, "sell"),
            ExecutionAction::BuyFailed => write!(f, "buy_failed"),
            ExecutionAction::SellFailed => write!(f, "sell_failed"),
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
    /// "system" (monitor automático) ou "user" (tick manual). Preenchido no persist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SignalType {
    TakeProfit,
    StopLoss,
    GradualSell,
    Expired,
    Info,
}

impl std::fmt::Display for SignalType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignalType::TakeProfit => write!(f, "take_profit"),
            SignalType::StopLoss => write!(f, "stop_loss"),
            SignalType::GradualSell => write!(f, "gradual_sell"),
            SignalType::Expired => write!(f, "expired"),
            SignalType::Info => write!(f, "info"),
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
    /// "system" (monitor automático) ou "user" (tick manual). Preenchido no persist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

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
    pub opened_at: i64,
}

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
    pub position: Option<PositionInfo>,
    #[serde(default)]
    pub executions: Vec<StrategyExecution>,
    #[serde(default)]
    pub signals: Vec<StrategySignal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_gradual_sell_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default)]
    pub total_pnl_usd: f64,
    #[serde(default)]
    pub total_executions: i32,
    pub started_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

fn default_true() -> bool { true }

#[derive(Debug, Deserialize)]
pub struct CreateStrategyRequest {
    pub name: String,
    pub symbol: String,
    pub exchange_id: String,
    pub exchange_name: String,
    pub config: StrategyConfig,
}

#[derive(Debug, Deserialize)]
pub struct UpdateStrategyRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub exchange_id: Option<String>,
    #[serde(default)]
    pub exchange_name: Option<String>,
    #[serde(default)]
    pub is_active: Option<bool>,
    #[serde(default)]
    pub config: Option<StrategyConfig>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StrategyStatsResponse {
    pub total_executions: i32,
    pub total_sells: i32,
    pub total_pnl_usd: f64,
    pub total_fees: f64,
    pub win_rate: f64,
    pub current_position: Option<PositionInfo>,
}

#[derive(Debug, Serialize)]
pub struct StrategyResponse {
    pub id: String,
    pub name: String,
    pub symbol: String,
    pub exchange_id: String,
    pub exchange_name: String,
    pub is_active: bool,
    pub status: StrategyStatus,
    pub config: StrategyConfig,
    pub trigger_price: f64,
    pub stop_loss_price: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<PositionInfo>,
    pub executions: Vec<StrategyExecution>,
    pub signals: Vec<StrategySignal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub total_pnl_usd: f64,
    pub total_executions: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<StrategyStatsResponse>,
    pub started_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl StrategyItem {
    pub fn compute_stats(&self) -> StrategyStatsResponse {
        let total_sells = self.executions.iter()
            .filter(|e| e.action == ExecutionAction::Sell)
            .count() as i32;
        let sell_execs: Vec<&StrategyExecution> = self.executions.iter()
            .filter(|e| e.action == ExecutionAction::Sell)
            .collect();
        let total_fees: f64 = self.executions.iter().map(|e| e.fee).sum();
        let wins = sell_execs.iter().filter(|e| e.pnl_usd > 0.0).count();
        let win_rate = if sell_execs.is_empty() { 0.0 } else {
            (wins as f64 / sell_execs.len() as f64) * 100.0
        };
        StrategyStatsResponse {
            total_executions: self.executions.len() as i32,
            total_sells,
            total_pnl_usd: self.total_pnl_usd,
            total_fees,
            win_rate,
            current_position: self.position.clone(),
        }
    }

    pub fn is_expired(&self) -> bool {
        let now = chrono::Utc::now().timestamp();
        let max_secs = self.config.time_execution_min * 60;
        now - self.started_at >= max_secs
    }
}

impl From<StrategyItem> for StrategyResponse {
    fn from(item: StrategyItem) -> Self {
        let stats = item.compute_stats();
        StrategyResponse {
            id: item.strategy_id.clone(),
            name: item.name,
            symbol: item.symbol,
            exchange_id: item.exchange_id,
            exchange_name: item.exchange_name,
            is_active: item.is_active,
            status: item.status,
            trigger_price: item.config.trigger_price(),
            stop_loss_price: item.config.stop_loss_price(),
            config: item.config,
            position: item.position,
            executions: item.executions,
            signals: item.signals,
            last_checked_at: item.last_checked_at,
            last_price: item.last_price,
            error_message: item.error_message,
            total_pnl_usd: item.total_pnl_usd,
            total_executions: item.total_executions,
            stats: Some(stats),
            started_at: item.started_at,
            created_at: item.created_at,
            updated_at: item.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct StrategyListItem {
    pub id: String,
    pub name: String,
    pub symbol: String,
    pub exchange_name: String,
    pub is_active: bool,
    pub status: StrategyStatus,
    pub trigger_price: f64,
    pub stop_loss_price: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<PositionInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_price: Option<f64>,
    pub total_pnl_usd: f64,
    pub total_executions: i32,
    pub started_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<StrategyItem> for StrategyListItem {
    fn from(item: StrategyItem) -> Self {
        StrategyListItem {
            id: item.strategy_id.clone(),
            name: item.name,
            symbol: item.symbol,
            exchange_name: item.exchange_name,
            is_active: item.is_active,
            status: item.status,
            trigger_price: item.config.trigger_price(),
            stop_loss_price: item.config.stop_loss_price(),
            position: item.position,
            last_price: item.last_price,
            total_pnl_usd: item.total_pnl_usd,
            total_executions: item.total_executions,
            started_at: item.started_at,
            created_at: item.created_at,
            updated_at: item.updated_at,
        }
    }
}
