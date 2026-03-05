use serde::{Deserialize, Serialize};
use mongodb::bson::oid::ObjectId;

/// Deserializador flexível de timestamps: aceita i64 (unix segundos),
/// BSON DateTime ({ "$date": ms }) ou string numérica.
/// Necessário pois scripts externos (Python) podem gravar datas como
/// objetos DateTime enquanto o código Rust usa timestamps Unix (i64).
mod ts_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use serde::de::{self, Visitor, MapAccess};
    use std::fmt;

    struct TsVisitor;
    impl<'de> Visitor<'de> for TsVisitor {
        type Value = i64;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "i64 timestamp (segundos) ou BSON DateTime")
        }
        fn visit_i8<E: de::Error>(self, v: i8)   -> Result<i64, E> { Ok(v as i64) }
        fn visit_i16<E: de::Error>(self, v: i16) -> Result<i64, E> { Ok(v as i64) }
        fn visit_i32<E: de::Error>(self, v: i32) -> Result<i64, E> { Ok(v as i64) }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<i64, E> { Ok(v) }
        fn visit_u8<E: de::Error>(self, v: u8)   -> Result<i64, E> { Ok(v as i64) }
        fn visit_u16<E: de::Error>(self, v: u16) -> Result<i64, E> { Ok(v as i64) }
        fn visit_u32<E: de::Error>(self, v: u32) -> Result<i64, E> { Ok(v as i64) }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<i64, E> { Ok(v as i64) }
        fn visit_f32<E: de::Error>(self, v: f32) -> Result<i64, E> { Ok(v as i64) }
        fn visit_f64<E: de::Error>(self, v: f64) -> Result<i64, E> { Ok(v as i64) }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<i64, E> {
            v.parse::<i64>().map_err(de::Error::custom)
        }
        /// BSON DateTime chega como mapa: { "$date": { "$numberLong": "ms" } }
        ///                            ou: { "$date": ms_int }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<i64, A::Error> {
            #[derive(Deserialize)]
            #[serde(untagged)]
            enum DateVal {
                Int(i64),
                UInt(u64),
                Str(String),
                NumberLong { #[serde(rename = "$numberLong")] n: String },
            }
            let mut ms: Option<i64> = None;
            while let Some(key) = map.next_key::<String>()? {
                if key == "$date" {
                    let val = map.next_value::<DateVal>()?;
                    ms = Some(match val {
                        DateVal::Int(v)  => v,
                        DateVal::UInt(v) => v as i64,
                        DateVal::Str(s)  => s.parse().map_err(de::Error::custom)?,
                        DateVal::NumberLong { n } => n.parse().map_err(de::Error::custom)?,
                    });
                } else {
                    let _ = map.next_value::<serde::de::IgnoredAny>()?;
                }
            }
            // BSON DateTime está em milissegundos; converter para segundos Unix
            ms.map(|v| v / 1000).ok_or_else(|| de::Error::missing_field("$date"))
        }
    }

    pub fn serialize<S: Serializer>(ts: &i64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_i64(*ts)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
        d.deserialize_any(TsVisitor)
    }

    pub mod opt {
        use serde::{Deserializer, Serializer};
        use serde::de::{self, Visitor};
        use std::fmt;

        struct OptTsVisitor;
        impl<'de> Visitor<'de> for OptTsVisitor {
            type Value = Option<i64>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "optional i64 timestamp ou BSON DateTime")
            }
            fn visit_none<E: de::Error>(self) -> Result<Option<i64>, E> { Ok(None) }
            fn visit_unit<E: de::Error>(self) -> Result<Option<i64>, E> { Ok(None) }
            fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Option<i64>, D2::Error> {
                super::deserialize(d).map(Some)
            }
            // Valores diretos (quando o Option é armazenado sem wrapper Some)
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Option<i64>, E> { Ok(Some(v)) }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Option<i64>, E> { Ok(Some(v as i64)) }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Option<i64>, E> { Ok(Some(v as i64)) }
            fn visit_map<A: de::MapAccess<'de>>(self, map: A) -> Result<Option<i64>, A::Error> {
                super::TsVisitor.visit_map(map).map(Some)
            }
        }

        pub fn serialize<S: Serializer>(ts: &Option<i64>, s: S) -> Result<S::Ok, S::Error> {
            match ts { Some(v) => s.serialize_some(v), None => s.serialize_none() }
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
            d.deserialize_option(OptTsVisitor)
        }
    }
}

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
    Archived,
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
            StrategyStatus::Archived => write!(f, "archived"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GradualLot {
    pub lot_number: i32,
    pub sell_percent: f64,
    #[serde(default)]
    pub executed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "ts_serde::opt::deserialize")]
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
    /// DCA (Dollar Cost Averaging) — compra mais quando o preço cai.
    /// Se true, ao invés de stop loss (vender com prejuízo), compra mais
    /// para baixar o preço médio e facilitar atingir o TP.
    #[serde(default)]
    pub dca_enabled: bool,
    /// Valor em USD para cada compra DCA (ex: 36.0 = comprar +$36)
    #[serde(default)]
    pub dca_buy_amount_usd: f64,
    /// Queda percentual do preço médio para acionar DCA (ex: 5.0 = -5%)
    #[serde(default = "default_dca_trigger")]
    pub dca_trigger_percent: f64,
    /// Máximo de compras DCA extras (proteção contra queda infinita)
    #[serde(default = "default_dca_max")]
    pub dca_max_buys: i32,

    // ── Auto Buy Dip ────────────────────────────────────────────────
    /// Compra automática na queda. Funciona SEM posição aberta — monitora
    /// o preço e compra quando cai `auto_buy_dip_percent`% do base_price.
    /// Verifica saldo USDT antes de comprar.
    #[serde(default)]
    pub auto_buy_dip_enabled: bool,
    /// Queda percentual do base_price para acionar compra (ex: 5.0 = -5%)
    #[serde(default = "default_buy_dip_trigger")]
    pub auto_buy_dip_percent: f64,
    /// Valor em USDT de cada compra (ex: 50.0 = comprar $50)
    #[serde(default)]
    pub auto_buy_dip_amount_usd: f64,
    /// Máximo de compras automáticas (proteção)
    #[serde(default = "default_buy_dip_max")]
    pub auto_buy_dip_max_buys: i32,
}

fn default_timer_gradual() -> i64 { 15 }
fn default_time_execution() -> i64 { 120 }
fn default_dca_trigger() -> f64 { 5.0 }
fn default_dca_max() -> i32 { 3 }
fn default_buy_dip_trigger() -> f64 { 5.0 }
fn default_buy_dip_max() -> i32 { 3 }

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
            dca_enabled: false,
            dca_buy_amount_usd: 0.0,
            dca_trigger_percent: 5.0,
            dca_max_buys: 3,
            auto_buy_dip_enabled: false,
            auto_buy_dip_percent: 5.0,
            auto_buy_dip_amount_usd: 0.0,
            auto_buy_dip_max_buys: 3,
        }
    }
}

impl StrategyConfig {
    /// Preço alvo de take profit considerando AMBAS as taxas (compra + venda).
    ///
    /// Matemática:
    ///   custo_real   = base_price × (1 + fee)   ← você pagou taxa na compra
    ///   receita_venda = tp_price  × (1 - fee)   ← você pagará taxa na venda
    ///   lucro_líquido = tp_pct%   →  tp_price × (1-fee) = base_price × (1+fee) × (1 + tp%)
    ///   tp_price = base_price × (1+fee) × (1+tp%) / (1-fee)
    ///
    /// Exemplo: base=$0.92, TP=6%, fee=0.2%
    ///   tp_price = 0.92 × 1.002 × 1.06 / 0.998 = $0.9789
    ///   (vs. fórmula anterior sem taxa dupla: $0.9770 — subestimava em ~$0.0019)
    pub fn trigger_price(&self) -> f64 {
        let tp_factor  = self.take_profit_percent / 100.0;
        let fee_factor = self.fee_percent / 100.0;
        // Taxa na compra já foi paga → aumenta o custo efetivo
        // Taxa na venda será paga   → reduz a receita efetiva
        self.base_price * (1.0 + fee_factor) * (1.0 + tp_factor) / (1.0 - fee_factor)
    }

    /// Preço alvo de stop loss considerando a taxa que será paga na venda.
    ///
    /// Para limitar a perda líquida a `stop_loss_pct`% levando em conta ambas as taxas:
    ///   sl_price = base_price × (1+fee) × (1 - sl%) / (1-fee)
    ///
    /// Isso faz o stop disparar ligeiramente antes do que sem taxas,
    /// garantindo que a perda real (incluindo fee de venda) não exceda o limite.
    pub fn stop_loss_price(&self) -> f64 {
        let sl_factor  = self.stop_loss_percent / 100.0;
        let fee_factor = self.fee_percent / 100.0;
        self.base_price * (1.0 + fee_factor) * (1.0 - sl_factor) / (1.0 - fee_factor)
    }

    /// Preço que aciona auto-compra na queda. Ex: base_price $100, dip 5% → $95
    pub fn buy_dip_trigger_price(&self) -> f64 {
        self.base_price * (1.0 - self.auto_buy_dip_percent / 100.0)
    }

    pub fn gradual_trigger_price(&self, lot_index: usize) -> f64 {
        let base_tp       = self.take_profit_percent / 100.0;
        let fee           = self.fee_percent / 100.0;
        let gradual_step  = self.gradual_take_percent / 100.0;
        // Mesmo critério da trigger_price: cobre taxa de compra + taxa de venda + lucro desejado
        let effective_tp  = base_tp + gradual_step * lot_index as f64;
        self.base_price * (1.0 + fee) * (1.0 + effective_tp) / (1.0 - fee)
    }

    /// Calcula a quantidade estimada de moedas com base no investimento,
    /// já descontando a taxa de compra (tokens que você realmente recebe).
    /// Ex: invested $15.5, base_price $0.92, fee 0.2% → 16.813 SUI (não 16.847)
    pub fn estimated_quantity(&self) -> f64 {
        if self.invested_amount > 0.0 && self.base_price > 0.0 {
            let gross_qty = self.invested_amount / self.base_price;
            let fee_factor = self.fee_percent / 100.0;
            gross_qty * (1.0 - fee_factor) // desconta taxa de compra paga no token
        } else {
            0.0
        }
    }

    /// Calcula PnL líquido estimado em $ baseado no invested_amount e preço atual,
    /// já considerando as taxas de compra (paga) e venda (a ser paga).
    pub fn estimated_pnl(&self, current_price: f64) -> Option<f64> {
        if self.invested_amount > 0.0 && self.base_price > 0.0 {
            let fee = self.fee_percent / 100.0;
            let qty_received = self.invested_amount / self.base_price * (1.0 - fee);
            let sell_revenue = qty_received * current_price * (1.0 - fee); // descontando fee de venda
            Some(sell_revenue - self.invested_amount)
        } else {
            None
        }
    }

    /// Lucro líquido em % que o usuário verá no bolso ao atingir o TP,
    /// levando em conta as duas taxas.
    pub fn net_profit_percent_at_tp(&self) -> f64 {
        let tp = self.trigger_price();
        let fee = self.fee_percent / 100.0;
        let qty = self.invested_amount / self.base_price * (1.0 - fee);
        let revenue = qty * tp * (1.0 - fee);
        (revenue - self.invested_amount) / self.invested_amount * 100.0
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
    #[serde(deserialize_with = "ts_serde::deserialize")]
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
    DcaBuy,
    BuyDip,
    Expired,
    Info,
}

impl std::fmt::Display for SignalType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignalType::TakeProfit => write!(f, "take_profit"),
            SignalType::StopLoss => write!(f, "stop_loss"),
            SignalType::GradualSell => write!(f, "gradual_sell"),
            SignalType::DcaBuy => write!(f, "dca_buy"),
            SignalType::BuyDip => write!(f, "buy_dip"),
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
    #[serde(deserialize_with = "ts_serde::deserialize")]
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
    #[serde(deserialize_with = "ts_serde::deserialize")]
    pub opened_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserStrategies {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub user_id: String,
    #[serde(default)]
    pub strategies: Vec<StrategyItem>,
    #[serde(deserialize_with = "ts_serde::deserialize")]
    pub created_at: i64,
    #[serde(deserialize_with = "ts_serde::deserialize")]
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
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "ts_serde::opt::deserialize")]
    pub last_checked_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "ts_serde::opt::deserialize")]
    pub last_gradual_sell_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default)]
    pub total_pnl_usd: f64,
    #[serde(default)]
    pub total_executions: i32,
    /// Número de compras DCA já realizadas nesta estratégia.
    #[serde(default)]
    pub dca_buys_done: i32,
    /// Número de compras "Buy the Dip" já realizadas.
    #[serde(default)]
    pub buy_dip_buys_done: i32,
    /// Soft delete: se preenchido, estratégia foi arquivada (não aparece na lista ativa)
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "ts_serde::opt::deserialize")]
    pub deleted_at: Option<i64>,
    #[serde(deserialize_with = "ts_serde::deserialize")]
    pub started_at: i64,
    #[serde(deserialize_with = "ts_serde::deserialize")]
    pub created_at: i64,
    #[serde(deserialize_with = "ts_serde::deserialize")]
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
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "ts_serde::opt::deserialize")]
    pub last_checked_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub total_pnl_usd: f64,
    pub total_executions: i32,
    pub dca_buys_done: i32,
    pub buy_dip_buys_done: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<StrategyStatsResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "ts_serde::opt::deserialize")]
    pub deleted_at: Option<i64>,
    #[serde(deserialize_with = "ts_serde::deserialize")]
    pub started_at: i64,
    #[serde(deserialize_with = "ts_serde::deserialize")]
    pub created_at: i64,
    #[serde(deserialize_with = "ts_serde::deserialize")]
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
            dca_buys_done: item.dca_buys_done,
            buy_dip_buys_done: item.buy_dip_buys_done,
            stats: Some(stats),
            deleted_at: item.deleted_at,
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
    pub dca_buys_done: i32,
    pub buy_dip_buys_done: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<i64>,
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
            dca_buys_done: item.dca_buys_done,
            buy_dip_buys_done: item.buy_dip_buys_done,
            deleted_at: item.deleted_at,
            started_at: item.started_at,
            created_at: item.created_at,
            updated_at: item.updated_at,
        }
    }
}
