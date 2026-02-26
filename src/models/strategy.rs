use mongodb::bson::oid::ObjectId;
use serde::{Deserialize, Serialize};

// ═══════════════════════════════════════════════════════════════════
// STRATEGY STATUS — Ciclo de vida da estratégia
// ═══════════════════════════════════════════════════════════════════

/// Status operacional da estratégia
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StrategyStatus {
    /// Aguardando ativação (recém-criada, sem entry_price)
    Idle,
    /// Monitorando preço, aguardando sinal
    Monitoring,
    /// Ordem de compra colocada, aguardando execução
    BuyPending,
    /// Posição aberta — monitorando TP/SL
    InPosition,
    /// Ordem de venda colocada, aguardando execução
    SellPending,
    /// Pausada pelo usuário (mantém posição se tiver)
    Paused,
    /// Finalizada (TP/SL atingido ou encerrada manualmente)
    Completed,
    /// Erro na execução — requer atenção
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
// STRATEGY CONFIG — Regras tipadas extraídas do template
// ═══════════════════════════════════════════════════════════════════

/// Nível de Take Profit
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TakeProfitLevel {
    /// % de lucro para acionar (ex: 5.0)
    pub percent: f64,
    /// % da posição para vender neste nível (ex: 50.0)
    pub sell_percent: f64,
    /// Se já foi executado
    #[serde(default)]
    pub executed: bool,
    /// Timestamp de execução (se executado)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executed_at: Option<i64>,
}

/// Configuração de Stop Loss
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopLossConfig {
    /// Se está habilitado
    pub enabled: bool,
    /// % de perda para acionar (ex: 3.0 = -3%)
    pub percent: f64,
    /// Se é trailing stop
    #[serde(default)]
    pub trailing: bool,
    /// Distância do trailing em % (se trailing = true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trailing_distance: Option<f64>,
    /// Preço mais alto atingido (para trailing stop)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highest_price: Option<f64>,
}

/// Configuração de DCA (Dollar Cost Averaging)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DcaConfig {
    /// Se está habilitado
    pub enabled: bool,
    /// Intervalo entre compras em segundos
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_seconds: Option<i64>,
    /// Valor por compra em USDT
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_per_buy: Option<f64>,
    /// Máximo de compras
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_buys: Option<i32>,
    /// Compras já realizadas
    #[serde(default)]
    pub buys_done: i32,
    /// % de queda para comprar mais (buy the dip)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dip_percent: Option<f64>,
}

/// Configuração de Grid Trading
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridConfig {
    /// Se está habilitado
    pub enabled: bool,
    /// Número de níveis do grid
    #[serde(skip_serializing_if = "Option::is_none")]
    pub levels: Option<i32>,
    /// Espaçamento entre níveis em %
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spacing_percent: Option<f64>,
    /// Preço central do grid
    #[serde(skip_serializing_if = "Option::is_none")]
    pub center_price: Option<f64>,
}

/// Configuração completa e tipada de uma estratégia
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyConfig {
    /// Níveis de Take Profit
    #[serde(default)]
    pub take_profit_levels: Vec<TakeProfitLevel>,

    /// Configuração de Stop Loss
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_loss: Option<StopLossConfig>,

    /// Configuração de DCA
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dca: Option<DcaConfig>,

    /// Configuração de Grid
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grid: Option<GridConfig>,

    /// Investimento mínimo em USDT
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_investment: Option<f64>,

    /// Máximo de operações por dia
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_daily_operations: Option<i32>,

    /// Fechamento automático (timestamp hora UTC, ex: 23:00 = 82800)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_close_time: Option<i64>,

    /// Modo de operação (spot, margin, futures)
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
// EXECUTIONS — Histórico de ordens executadas
// ═══════════════════════════════════════════════════════════════════

/// Tipo de ação executada
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

/// Registro de uma execução (ordem que foi colocada/preenchida)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyExecution {
    /// ID único da execução
    pub execution_id: String,

    /// Ação realizada
    pub action: ExecutionAction,

    /// Motivo (ex: "take_profit_1", "stop_loss", "dca_buy_3")
    pub reason: String,

    /// Preço de execução
    pub price: f64,

    /// Quantidade executada
    pub amount: f64,

    /// Custo total (price * amount)
    pub total: f64,

    /// Taxa cobrada pela exchange
    #[serde(default)]
    pub fee: f64,

    /// PNL em USD desta execução (para vendas)
    #[serde(default)]
    pub pnl_usd: f64,

    /// ID da ordem na exchange (para rastreamento)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exchange_order_id: Option<String>,

    /// Timestamp de execução
    pub executed_at: i64,

    /// Mensagem de erro (se failed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

// ═══════════════════════════════════════════════════════════════════
// SIGNALS — Sinais gerados pelo monitor (avaliações de regras)
// ═══════════════════════════════════════════════════════════════════

/// Tipo de sinal gerado
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SignalType {
    /// Sinal de compra
    Buy,
    /// Sinal de venda (Take Profit)
    TakeProfit,
    /// Sinal de venda (Stop Loss)
    StopLoss,
    /// Sinal de venda (Trailing Stop)
    TrailingStop,
    /// Sinal de compra DCA
    DcaBuy,
    /// Grid compra/venda
    GridTrade,
    /// Apenas log informativo (nenhuma ação)
    Info,
    /// Alerta de preço sem ação
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

/// Sinal gerado pelo strategy_monitor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategySignal {
    /// Tipo de sinal
    pub signal_type: SignalType,

    /// Preço no momento do sinal
    pub price: f64,

    /// Descrição legível (ex: "Preço atingiu TP1 (+5.2%)")
    pub message: String,

    /// Se o sinal resultou em execução
    #[serde(default)]
    pub acted: bool,

    /// % de variação do preço em relação ao entry_price
    #[serde(default)]
    pub price_change_percent: f64,

    /// Timestamp do sinal
    pub created_at: i64,
}

// ═══════════════════════════════════════════════════════════════════
// POSITION INFO — Estado atual da posição
// ═══════════════════════════════════════════════════════════════════

/// Informações da posição aberta
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionInfo {
    /// Preço médio de entrada
    pub entry_price: f64,

    /// Quantidade total em posição
    pub quantity: f64,

    /// Custo total da posição
    pub total_cost: f64,

    /// Preço atual (atualizado pelo monitor)
    #[serde(default)]
    pub current_price: f64,

    /// PNL não realizado em USD
    #[serde(default)]
    pub unrealized_pnl: f64,

    /// PNL não realizado em %
    #[serde(default)]
    pub unrealized_pnl_percent: f64,

    /// Preço mais alto desde entrada (para trailing stop)
    #[serde(default)]
    pub highest_price: f64,

    /// Preço mais baixo desde entrada
    #[serde(default)]
    pub lowest_price: f64,

    /// Timestamp de abertura da posição
    pub opened_at: i64,
}

// ═══════════════════════════════════════════════════════════════════
// STRATEGY — Modelo principal (MongoDB)
// ═══════════════════════════════════════════════════════════════════

/// Estratégia de trading (armazenada no MongoDB)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Strategy {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,

    /// ID do usuário dono da estratégia
    pub user_id: String,

    /// Nome da estratégia
    pub name: String,

    /// Descrição opcional
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Tipo de estratégia (ex: "buy_and_hold", "dca", "swing_trade", "day_trade", "scalping", "arbitrage", "grid")
    pub strategy_type: String,

    /// Símbolo (ex: "BTC/USDT")
    pub symbol: String,

    /// ID da exchange (ObjectId como string)
    pub exchange_id: String,

    /// Nome da exchange (para facilitar queries)
    pub exchange_name: String,

    /// Status ativo/inativo (legado, mantido para compatibilidade)
    #[serde(default = "default_true")]
    pub is_active: bool,

    // ──────────── FASE 2: Novos campos ────────────

    /// Status operacional detalhado
    #[serde(default)]
    pub status: StrategyStatus,

    /// Configuração tipada (regras TP/SL/DCA/Grid)
    #[serde(default)]
    pub config: StrategyConfig,

    /// Configuração legada (JSON genérico) - para backward compatibility
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_legacy: Option<serde_json::Value>,

    /// Informações da posição atual (se in_position)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<PositionInfo>,

    /// Histórico de execuções (ordens executadas)
    #[serde(default)]
    pub executions: Vec<StrategyExecution>,

    /// Sinais gerados pelo monitor (últimos 50)
    #[serde(default)]
    pub signals: Vec<StrategySignal>,

    /// Última vez que o monitor checou esta estratégia
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<i64>,

    /// Último preço observado pelo monitor
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_price: Option<f64>,

    /// Intervalo de checagem em segundos (default: 60)
    #[serde(default = "default_check_interval")]
    pub check_interval_secs: i64,

    /// Mensagem de erro (quando status = Error)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,

    /// PNL total realizado em USD
    #[serde(default)]
    pub total_pnl_usd: f64,

    /// Total de operações executadas
    #[serde(default)]
    pub total_executions: i32,

    // ──────────── Timestamps ────────────

    /// Timestamp de criação (Unix timestamp)
    pub created_at: i64,

    /// Timestamp de última atualização
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

/// Request para criar estratégia
#[derive(Debug, Deserialize)]
pub struct CreateStrategyRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub strategy_type: String,
    pub symbol: String,
    pub exchange_id: String,
    pub exchange_name: String,

    /// Config tipada (nova)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<StrategyConfig>,

    /// Config JSON legada (fallback)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_legacy: Option<serde_json::Value>,

    /// Intervalo de checagem em segundos
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_interval_secs: Option<i64>,
}

/// Request para atualizar estratégia
#[derive(Debug, Deserialize)]
pub struct UpdateStrategyRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub strategy_type: Option<String>,
    pub symbol: Option<String>,
    pub exchange_id: Option<String>,
    pub exchange_name: Option<String>,
    pub is_active: Option<bool>,
    pub status: Option<StrategyStatus>,
    pub config: Option<StrategyConfig>,
    pub config_legacy: Option<serde_json::Value>,
    pub check_interval_secs: Option<i64>,
}

/// Estatísticas calculadas da estratégia
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

/// Response de estratégia
#[derive(Debug, Serialize)]
pub struct StrategyResponse {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub description: Option<String>,
    pub strategy_type: String,
    pub symbol: String,
    pub exchange_id: String,
    pub exchange_name: String,
    pub is_active: bool,

    // Fase 2
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

    // Stats calculadas
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<StrategyStatsResponse>,

    pub created_at: i64,
    pub updated_at: i64,
}

impl Strategy {
    /// Calcula estatísticas da estratégia
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
        let win_rate = if sell_executions.is_empty() {
            0.0
        } else {
            (wins as f64 / sell_executions.len() as f64) * 100.0
        };

        let avg_profit = if sell_executions.is_empty() {
            0.0
        } else {
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

impl From<Strategy> for StrategyResponse {
    fn from(strategy: Strategy) -> Self {
        let stats = strategy.compute_stats();
        StrategyResponse {
            id: strategy.id.map(|id| id.to_hex()).unwrap_or_default(),
            user_id: strategy.user_id,
            name: strategy.name,
            description: strategy.description,
            strategy_type: strategy.strategy_type,
            symbol: strategy.symbol,
            exchange_id: strategy.exchange_id,
            exchange_name: strategy.exchange_name,
            is_active: strategy.is_active,
            status: strategy.status,
            config: strategy.config,
            config_legacy: strategy.config_legacy,
            position: strategy.position,
            executions: strategy.executions,
            signals: strategy.signals,
            last_checked_at: strategy.last_checked_at,
            last_price: strategy.last_price,
            check_interval_secs: strategy.check_interval_secs,
            error_message: strategy.error_message,
            total_pnl_usd: strategy.total_pnl_usd,
            total_executions: strategy.total_executions,
            stats: Some(stats),
            created_at: strategy.created_at,
            updated_at: strategy.updated_at,
        }
    }
}

/// Response compacta para listagem (sem executions/signals completos)
#[derive(Debug, Serialize)]
pub struct StrategyListItem {
    pub id: String,
    pub user_id: String,
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

impl From<Strategy> for StrategyListItem {
    fn from(strategy: Strategy) -> Self {
        StrategyListItem {
            id: strategy.id.map(|id| id.to_hex()).unwrap_or_default(),
            user_id: strategy.user_id,
            name: strategy.name,
            description: strategy.description,
            strategy_type: strategy.strategy_type,
            symbol: strategy.symbol,
            exchange_id: strategy.exchange_id,
            exchange_name: strategy.exchange_name,
            is_active: strategy.is_active,
            status: strategy.status,
            position: strategy.position,
            last_price: strategy.last_price,
            last_checked_at: strategy.last_checked_at,
            total_pnl_usd: strategy.total_pnl_usd,
            total_executions: strategy.total_executions,
            executions_count: strategy.executions.len(),
            signals_count: strategy.signals.len(),
            check_interval_secs: strategy.check_interval_secs,
            error_message: strategy.error_message,
            created_at: strategy.created_at,
            updated_at: strategy.updated_at,
        }
    }
}
