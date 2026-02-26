use crate::database::MongoDB;
use crate::models::{StrategyTemplate, RiskLevel, TemplateConfig};
use mongodb::bson::doc;

/// Seed dos 7 templates padrão no MongoDB.
/// Só insere se a collection estiver vazia de defaults.
pub async fn seed_default_templates(db: &MongoDB) {
    let collection = db.collection::<StrategyTemplate>("strategy_templates");

    // Verifica se já existem templates padrão no banco
    let count = collection
        .count_documents(doc! { "is_default": true })
        .await
        .unwrap_or(0);

    if count >= 7 {
        log::info!("📋 Strategy templates: {} defaults already in DB — skipping seed", count);
        return;
    }

    // Se existem menos de 7 (versão antiga ou parcial), remove e recria
    if count > 0 {
        log::info!("📋 Strategy templates: found {} defaults (expected 7) — recreating...", count);
        let _ = collection.delete_many(doc! { "is_default": true }).await;
    }

    log::info!("📋 Strategy templates: seeding 7 default templates into MongoDB...");

    let now = chrono::Utc::now().timestamp();
    let templates = build_default_templates(now);

    match collection.insert_many(&templates).await {
        Ok(result) => {
            log::info!("   ✅ Inserted {} default templates into strategy_templates collection",
                result.inserted_ids.len());
        }
        Err(e) => {
            log::error!("   ❌ Failed to seed default templates: {}", e);
        }
    }
}

/// Constrói os 7 templates padrão
fn build_default_templates(now: i64) -> Vec<StrategyTemplate> {
    vec![
        // ─────────────────────────────────────────────
        // 1. BUY AND HOLD (Longo Prazo)
        // ─────────────────────────────────────────────
        StrategyTemplate {
            id: None,
            user_id: "system".into(),
            name: "Buy and Hold".into(),
            icon: "💎".into(),
            strategy_type: "buy_and_hold".into(),
            risk: RiskLevel { label: "Baixo".into(), color: "#10b981".into() },
            summary: "Compre e segure por meses ou anos. A estratégia mais simples: você compra o ativo e mantém na carteira apostando na valorização de longo prazo, ignorando as oscilações do dia a dia.".into(),
            configs: vec![
                TemplateConfig { label: "Tipo".into(), value: "Buy and Hold (Longo Prazo)".into(), detail: None },
                TemplateConfig { label: "Horizonte".into(), value: "Meses a Anos".into(), detail: Some("Mantenha por pelo menos 6 meses para melhores resultados".into()) },
                TemplateConfig { label: "Take Profit".into(), value: "50%".into(), detail: Some("Alvo de longo prazo — vende quando valorizar 50%".into()) },
                TemplateConfig { label: "Stop Loss".into(), value: "20%".into(), detail: Some("Proteção ampla — aceita volatilidade normal do mercado".into()) },
                TemplateConfig { label: "Investimento mín.".into(), value: "50 USDT".into(), detail: Some("Valor mínimo recomendado para começar".into()) },
                TemplateConfig { label: "Frequência".into(), value: "Compra única".into(), detail: Some("Uma única compra, sem rebalanceamento automático".into()) },
                TemplateConfig { label: "Modo".into(), value: "Spot".into(), detail: Some("Sem alavancagem — apenas compra real do ativo".into()) },
            ],
            how_it_works: vec![
                "1. Você escolhe um token (ex: BTC, ETH) e uma exchange".into(),
                "2. O sistema registra o preço de compra como referência".into(),
                "3. Monitora o preço continuamente em segundo plano".into(),
                "4. Se o preço subir +50%, notifica para realizar o lucro (Take Profit)".into(),
                "5. Se o preço cair -20%, notifica para proteger o capital (Stop Loss)".into(),
                "6. Enquanto estiver entre esses limites, você simplesmente segura".into(),
                "💡 Ideal para: quem acredita no potencial de longo prazo do ativo".into(),
                "⏰ Paciência é a chave — ignore o ruído do dia a dia".into(),
            ],
            is_default: true,
            created_at: now,
            updated_at: now,
        },

        // ─────────────────────────────────────────────
        // 2. DCA — Dollar Cost Averaging (Preço Médio)
        // ─────────────────────────────────────────────
        StrategyTemplate {
            id: None,
            user_id: "system".into(),
            name: "DCA (Preço Médio)".into(),
            icon: "🛡️".into(),
            strategy_type: "dca".into(),
            risk: RiskLevel { label: "Baixo".into(), color: "#10b981".into() },
            summary: "Compras automáticas em intervalos regulares para diluir o preço médio. Você investe sempre o mesmo valor (ex: R$100/semana), reduzindo o impacto da volatilidade ao longo do tempo.".into(),
            configs: vec![
                TemplateConfig { label: "Tipo".into(), value: "DCA — Dollar Cost Averaging".into(), detail: None },
                TemplateConfig { label: "Intervalo DCA".into(), value: "7 dias".into(), detail: Some("Compra automática a cada 7 dias (semanal)".into()) },
                TemplateConfig { label: "Valor por compra".into(), value: "50 USDT".into(), detail: Some("Valor fixo investido em cada compra automática".into()) },
                TemplateConfig { label: "Máx. compras".into(), value: "12".into(), detail: Some("Até 12 compras parceladas (3 meses no semanal)".into()) },
                TemplateConfig { label: "Take Profit".into(), value: "15%".into(), detail: Some("Vende tudo quando o preço médio subir 15%".into()) },
                TemplateConfig { label: "Stop Loss".into(), value: "10%".into(), detail: Some("Para as compras e vende se cair 10% do preço médio".into()) },
                TemplateConfig { label: "Investimento mín.".into(), value: "50 USDT".into(), detail: Some("Por compra — total depende do nº de compras".into()) },
                TemplateConfig { label: "Modo".into(), value: "Spot".into(), detail: Some("Sem alavancagem — compras reais do ativo".into()) },
            ],
            how_it_works: vec![
                "1. Você define o token, exchange e o valor por compra".into(),
                "2. A cada 7 dias, o sistema compra automaticamente o valor definido".into(),
                "3. Se o preço caiu, você compra mais barato — melhora seu preço médio".into(),
                "4. Se o preço subiu, você compra menos unidades — mas ainda acumula".into(),
                "5. Após todas as compras, monitora o preço médio total".into(),
                "6. Take Profit: vende tudo quando subir 15% acima do preço médio".into(),
                "7. Stop Loss: vende tudo se cair 10% abaixo do preço médio".into(),
                "💡 Ideal para: quem quer investir regularmente sem se preocupar com timing".into(),
                "📊 Estatisticamente supera quem tenta acertar o melhor momento de compra".into(),
            ],
            is_default: true,
            created_at: now,
            updated_at: now,
        },

        // ─────────────────────────────────────────────
        // 3. SWING TRADE (Médio Prazo)
        // ─────────────────────────────────────────────
        StrategyTemplate {
            id: None,
            user_id: "system".into(),
            name: "Swing Trade".into(),
            icon: "📈".into(),
            strategy_type: "swing_trade".into(),
            risk: RiskLevel { label: "Médio".into(), color: "#f59e0b".into() },
            summary: "Captura movimentos de preço que duram de dias a semanas. Você compra em suportes e vende em resistências, usando análise técnica para identificar pontos de entrada e saída.".into(),
            configs: vec![
                TemplateConfig { label: "Tipo".into(), value: "Swing Trade (Médio Prazo)".into(), detail: None },
                TemplateConfig { label: "Horizonte".into(), value: "Dias a Semanas".into(), detail: Some("Operações duram de 2 a 30 dias em média".into()) },
                TemplateConfig { label: "Take Profit 1".into(), value: "5%".into(), detail: Some("Realiza 50% da posição no primeiro alvo".into()) },
                TemplateConfig { label: "Take Profit 2".into(), value: "10%".into(), detail: Some("Realiza os 50% restantes no segundo alvo".into()) },
                TemplateConfig { label: "Stop Loss".into(), value: "3%".into(), detail: Some("Sai da operação se cair 3% do preço de entrada".into()) },
                TemplateConfig { label: "Trailing Stop".into(), value: "2%".into(), detail: Some("Protege lucro — sobe junto com o preço, nunca desce".into()) },
                TemplateConfig { label: "Investimento mín.".into(), value: "100 USDT".into(), detail: Some("Valor mínimo para operações com boa margem".into()) },
                TemplateConfig { label: "Modo".into(), value: "Spot".into(), detail: Some("Sem alavancagem para menor risco".into()) },
            ],
            how_it_works: vec![
                "1. Você escolhe o token e exchange, define o preço de entrada".into(),
                "2. O sistema monitora o preço e compra no ponto definido".into(),
                "3. Quando subir 5% (TP1): vende automaticamente 50% — garante lucro parcial".into(),
                "4. Quando subir 10% (TP2): vende os 50% restantes — lucro máximo".into(),
                "5. Se o preço cair 3%: Stop Loss fecha tudo — limita a perda".into(),
                "6. Trailing Stop: após TP1, o stop sobe junto com o preço (2% abaixo do pico)".into(),
                "7. Se o preço voltar a cair após subir, trailing stop protege o lucro".into(),
                "💡 Ideal para: quem acompanha gráficos e quer lucrar com tendências de dias/semanas".into(),
                "📊 Requer atenção moderada — não precisa olhar a cada minuto".into(),
            ],
            is_default: true,
            created_at: now,
            updated_at: now,
        },

        // ─────────────────────────────────────────────
        // 4. DAY TRADE (Curto Prazo)
        // ─────────────────────────────────────────────
        StrategyTemplate {
            id: None,
            user_id: "system".into(),
            name: "Day Trade".into(),
            icon: "⚡".into(),
            strategy_type: "day_trade".into(),
            risk: RiskLevel { label: "Alto".into(), color: "#ef4444".into() },
            summary: "Compra e venda dentro do mesmo dia. Busca lucrar com as oscilações intradiárias do preço, fechando todas as posições antes do fim do dia. Requer atenção constante.".into(),
            configs: vec![
                TemplateConfig { label: "Tipo".into(), value: "Day Trade (Curto Prazo)".into(), detail: None },
                TemplateConfig { label: "Horizonte".into(), value: "Horas (mesmo dia)".into(), detail: Some("Todas as posições são fechadas no mesmo dia".into()) },
                TemplateConfig { label: "Take Profit".into(), value: "2%".into(), detail: Some("Alvo rápido — fecha 100% ao atingir +2%".into()) },
                TemplateConfig { label: "Stop Loss".into(), value: "1%".into(), detail: Some("Stop apertado — limita perda a 1% por operação".into()) },
                TemplateConfig { label: "Trailing Stop".into(), value: "0.5%".into(), detail: Some("Trailing agressivo para travar lucro rápido".into()) },
                TemplateConfig { label: "Máx. operações/dia".into(), value: "5".into(), detail: Some("Limite de 5 operações por dia para controlar risco".into()) },
                TemplateConfig { label: "Investimento mín.".into(), value: "200 USDT".into(), detail: Some("Valor mínimo por operação para cobrir taxas".into()) },
                TemplateConfig { label: "Fechamento auto".into(), value: "23:00 UTC".into(), detail: Some("Fecha todas posições abertas às 23h para não dormir comprado".into()) },
                TemplateConfig { label: "Modo".into(), value: "Spot".into(), detail: Some("Sem alavancagem — reduz risco de liquidação".into()) },
            ],
            how_it_works: vec![
                "1. Você define o token, exchange e preço de entrada desejado".into(),
                "2. O sistema compra quando o preço atinge o ponto de entrada".into(),
                "3. Take Profit em +2%: vende automaticamente com lucro rápido".into(),
                "4. Stop Loss em -1%: vende imediatamente se cair — perda mínima".into(),
                "5. Trailing Stop de 0.5%: se o preço subir além de +2%, acompanha".into(),
                "6. Limite de 5 operações por dia evita overtrading emocional".into(),
                "7. Fechamento automático às 23:00 UTC — nunca dorme posicionado".into(),
                "⚠️ Risco alto: requer experiência e disciplina emocional".into(),
                "💡 Ideal para: traders ativos que podem acompanhar o mercado durante o dia".into(),
                "📊 Proporção ideal: ganhe 2% quando acerta, perca 1% quando erra (2:1)".into(),
            ],
            is_default: true,
            created_at: now,
            updated_at: now,
        },

        // ─────────────────────────────────────────────
        // 5. SCALPING (Ultra Curto Prazo)
        // ─────────────────────────────────────────────
        StrategyTemplate {
            id: None,
            user_id: "system".into(),
            name: "Scalping".into(),
            icon: "🔥".into(),
            strategy_type: "scalping".into(),
            risk: RiskLevel { label: "Alto".into(), color: "#ef4444".into() },
            summary: "Muitas operações rápidas buscando micro-lucros. Entra e sai em minutos, lucrando centavos em cada operação mas com alto volume. Exige mercado líquido e taxas baixas.".into(),
            configs: vec![
                TemplateConfig { label: "Tipo".into(), value: "Scalping (Ultra Curto Prazo)".into(), detail: None },
                TemplateConfig { label: "Horizonte".into(), value: "Segundos a Minutos".into(), detail: Some("Operações duram de 30s a 15 minutos".into()) },
                TemplateConfig { label: "Take Profit".into(), value: "0.5%".into(), detail: Some("Micro-alvo — fecha rápido com +0.5%".into()) },
                TemplateConfig { label: "Stop Loss".into(), value: "0.3%".into(), detail: Some("Stop ultra-apertado — corta perda em -0.3%".into()) },
                TemplateConfig { label: "Máx. operações/dia".into(), value: "20".into(), detail: Some("Alto volume — até 20 operações por dia".into()) },
                TemplateConfig { label: "Intervalo mín.".into(), value: "2 min".into(), detail: Some("Espera pelo menos 2 min entre operações".into()) },
                TemplateConfig { label: "Investimento mín.".into(), value: "500 USDT".into(), detail: Some("Volume alto necessário — lucro vem da quantidade".into()) },
                TemplateConfig { label: "Pares recomendados".into(), value: "BTC, ETH, SOL".into(), detail: Some("Apenas pares com alta liquidez e spread baixo".into()) },
                TemplateConfig { label: "Modo".into(), value: "Spot".into(), detail: Some("Sem alavancagem para reduzir risco de liquidação".into()) },
            ],
            how_it_works: vec![
                "1. O sistema monitora o preço em tempo real (a cada poucos segundos)".into(),
                "2. Identifica micro-movimentos de preço favoráveis".into(),
                "3. Compra rápido e coloca Take Profit em +0.5%".into(),
                "4. Se atingir TP: vende em segundos — lucro pequeno mas rápido".into(),
                "5. Se cair 0.3%: Stop Loss corta a perda imediatamente".into(),
                "6. Repete o processo até 20x por dia".into(),
                "7. Lucro vem do volume: 20 ops × 0.5% = até ~10% no dia (otimista)".into(),
                "⚠️ Risco muito alto: taxas podem comer o lucro se não calcular bem".into(),
                "💡 Ideal para: traders experientes com exchange de taxas baixas (ex: Binance VIP)".into(),
                "🚫 Não recomendado para iniciantes — exige reflexo e disciplina extrema".into(),
            ],
            is_default: true,
            created_at: now,
            updated_at: now,
        },

        // ─────────────────────────────────────────────
        // 6. ARBITRAGEM (Entre Exchanges)
        // ─────────────────────────────────────────────
        StrategyTemplate {
            id: None,
            user_id: "system".into(),
            name: "Arbitragem".into(),
            icon: "🔄".into(),
            strategy_type: "arbitrage".into(),
            risk: RiskLevel { label: "Médio".into(), color: "#f59e0b".into() },
            summary: "Lucra com a diferença de preço do mesmo ativo entre exchanges diferentes. Compra onde está mais barato e vende onde está mais caro, simultaneamente. Risco baixo quando executado rápido.".into(),
            configs: vec![
                TemplateConfig { label: "Tipo".into(), value: "Arbitragem entre Exchanges".into(), detail: None },
                TemplateConfig { label: "Spread mín.".into(), value: "0.5%".into(), detail: Some("Só opera quando a diferença de preço for ≥ 0.5%".into()) },
                TemplateConfig { label: "Exchanges".into(), value: "2 ou mais".into(), detail: Some("Precisa de saldo em pelo menos 2 exchanges diferentes".into()) },
                TemplateConfig { label: "Take Profit".into(), value: "Spread - Taxas".into(), detail: Some("Lucro = diferença de preço menos taxas de ambas exchanges".into()) },
                TemplateConfig { label: "Stop Loss".into(), value: "Automático".into(), detail: Some("Se o spread fechar antes de executar, cancela a operação".into()) },
                TemplateConfig { label: "Tempo máx.".into(), value: "30 seg".into(), detail: Some("Janela de 30s para executar — depois o spread pode sumir".into()) },
                TemplateConfig { label: "Investimento mín.".into(), value: "500 USDT".into(), detail: Some("Valor alto necessário para lucro significativo no spread".into()) },
                TemplateConfig { label: "Modo".into(), value: "Spot".into(), detail: Some("Compra real em uma exchange, venda real na outra".into()) },
            ],
            how_it_works: vec![
                "1. O sistema monitora o preço do token em todas as suas exchanges conectadas".into(),
                "2. Quando detecta diferença de preço ≥ 0.5% entre duas exchanges:".into(),
                "   → Compra na exchange com preço MENOR".into(),
                "   → Vende na exchange com preço MAIOR".into(),
                "3. O lucro é a diferença entre os dois preços, menos as taxas".into(),
                "4. Exemplo: BTC a $95.000 na Binance e $95.600 na Coinbase".into(),
                "   → Spread de 0.63% → Compra Binance, Vende Coinbase → Lucro ~0.4%".into(),
                "5. Se o spread fechar antes de executar, a operação é cancelada (sem perda)".into(),
                "⚠️ Requer saldo em múltiplas exchanges simultaneamente".into(),
                "💡 Ideal para: quem tem contas em várias exchanges e busca lucro de baixo risco".into(),
                "📊 Lucro pequeno por operação, mas praticamente sem risco quando executado rápido".into(),
            ],
            is_default: true,
            created_at: now,
            updated_at: now,
        },

        // ─────────────────────────────────────────────
        // 7. GRID TRADING (Automatizado)
        // ─────────────────────────────────────────────
        StrategyTemplate {
            id: None,
            user_id: "system".into(),
            name: "Grid Trading".into(),
            icon: "🤖".into(),
            strategy_type: "grid".into(),
            risk: RiskLevel { label: "Médio".into(), color: "#f59e0b".into() },
            summary: "Bot automatizado que cria uma grade de ordens de compra e venda em intervalos fixos. Ideal para mercados laterais — lucra com cada oscilação de preço dentro da grade, sem precisar prever a direção.".into(),
            configs: vec![
                TemplateConfig { label: "Tipo".into(), value: "Grid Trading (Automatizado)".into(), detail: None },
                TemplateConfig { label: "Grid Levels".into(), value: "10".into(), detail: Some("10 níveis de preço — 5 de compra abaixo e 5 de venda acima".into()) },
                TemplateConfig { label: "Espaçamento".into(), value: "1%".into(), detail: Some("Cada nível separado por 1% do anterior".into()) },
                TemplateConfig { label: "Take Profit".into(), value: "10%".into(), detail: Some("Fecha todo o grid se o preço subir 10% do centro".into()) },
                TemplateConfig { label: "Stop Loss".into(), value: "5%".into(), detail: Some("Fecha todo o grid se o preço cair 5% do centro".into()) },
                TemplateConfig { label: "Sell Cascade".into(), value: "Sim".into(), detail: Some("Vende em cascata: cada nível acima vende uma parcela".into()) },
                TemplateConfig { label: "Investimento mín.".into(), value: "200 USDT".into(), detail: Some("Dividido entre os 10 níveis do grid (20 USDT cada)".into()) },
                TemplateConfig { label: "Reinício auto".into(), value: "Sim".into(), detail: Some("Quando uma ordem executa, cria nova no próximo nível".into()) },
                TemplateConfig { label: "Modo".into(), value: "Spot".into(), detail: Some("Sem alavancagem — grid de ordens reais".into()) },
            ],
            how_it_works: vec![
                "1. Você define o token e o preço central (ex: BTC a $95.000)".into(),
                "2. O sistema cria 10 ordens em forma de grade:".into(),
                "   → 5 ordens de COMPRA: $94.050, $93.110, $92.179, $91.257, $90.344".into(),
                "   → 5 ordens de VENDA: $95.950, $96.910, $97.879, $98.857, $99.846".into(),
                "3. Quando o preço oscila, ordens são executadas automaticamente".into(),
                "4. Cada vez que uma compra executa → cria uma venda 1% acima".into(),
                "5. Cada vez que uma venda executa → cria uma compra 1% abaixo".into(),
                "6. Lucro vem das oscilações: compra barato, vende caro, repetidamente".into(),
                "7. Stop Loss fecha tudo se sair do range (-5%) — protege o capital".into(),
                "8. Take Profit fecha tudo se romper pra cima (+10%) — garante o lucro".into(),
                "💡 Ideal para: mercados laterais onde o preço oscila sem tendência clara".into(),
                "🤖 100% automático — configure e deixe o bot trabalhar por você".into(),
                "📊 Quanto mais o preço oscila dentro do grid, mais lucro é gerado".into(),
            ],
            is_default: true,
            created_at: now,
            updated_at: now,
        },
    ]
}
