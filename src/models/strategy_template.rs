use mongodb::bson::oid::ObjectId;
use serde::{Deserialize, Serialize};

/// Configuração individual de um template
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateConfig {
    pub label: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Nível de risco do template
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskLevel {
    pub label: String,
    pub color: String,
}

/// Template de estratégia (armazenado no MongoDB)
/// Templates são independentes das estratégias — a tela de estratégia
/// apenas consome a lista de templates disponíveis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyTemplate {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,

    /// ID do usuário que criou o template (vazio para templates padrão do sistema)
    pub user_id: String,

    /// Nome do template
    pub name: String,

    /// Ícone (emoji)
    pub icon: String,

    /// Tipo de estratégia (ex: "Grid Trading", "DCA", "Trailing Stop + DCA")
    pub strategy_type: String,

    /// Nível de risco
    pub risk: RiskLevel,

    /// Resumo curto
    pub summary: String,

    /// Lista de configurações do template
    pub configs: Vec<TemplateConfig>,

    /// Passos de "como funciona"
    pub how_it_works: Vec<String>,

    /// Se é template padrão do sistema (não pode ser deletado pelo usuário)
    pub is_default: bool,

    /// Timestamp de criação
    pub created_at: i64,

    /// Timestamp de última atualização
    pub updated_at: i64,
}

/// Request para criar template
#[derive(Debug, Deserialize)]
pub struct CreateTemplateRequest {
    pub name: String,
    pub icon: String,
    pub strategy_type: String,
    pub risk: RiskLevel,
    pub summary: String,
    pub configs: Vec<TemplateConfig>,
    pub how_it_works: Vec<String>,
}

/// Request para atualizar template
#[derive(Debug, Deserialize)]
pub struct UpdateTemplateRequest {
    pub name: Option<String>,
    pub icon: Option<String>,
    pub strategy_type: Option<String>,
    pub risk: Option<RiskLevel>,
    pub summary: Option<String>,
    pub configs: Option<Vec<TemplateConfig>>,
    pub how_it_works: Option<Vec<String>>,
}

/// Response de template
#[derive(Debug, Serialize)]
pub struct StrategyTemplateResponse {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub icon: String,
    pub strategy_type: String,
    pub risk: RiskLevel,
    pub summary: String,
    pub configs: Vec<TemplateConfig>,
    pub how_it_works: Vec<String>,
    pub is_default: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<StrategyTemplate> for StrategyTemplateResponse {
    fn from(t: StrategyTemplate) -> Self {
        StrategyTemplateResponse {
            id: t.id.map(|id| id.to_hex()).unwrap_or_default(),
            user_id: t.user_id,
            name: t.name,
            icon: t.icon,
            strategy_type: t.strategy_type,
            risk: t.risk,
            summary: t.summary,
            configs: t.configs,
            how_it_works: t.how_it_works,
            is_default: t.is_default,
            created_at: t.created_at,
            updated_at: t.updated_at,
        }
    }
}
