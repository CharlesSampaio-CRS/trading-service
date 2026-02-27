#!/usr/bin/env python3
"""
╔═══════════════════════════════════════════════════════════════════╗
║  STRATEGY SIMULATOR — Swing Trade com Venda Gradual              ║
║  Simula cenários de TP/SL com e sem venda gradual escalonada     ║
║                                                                   ║
║  Config = entry_price + tp% + sl% + fee% + gradual_lots          ║
║  Quantity é parâmetro de simulação (vem da posição real)          ║
╚═══════════════════════════════════════════════════════════════════╝

Uso:
  python strategy_simulator.py
  python strategy_simulator.py --token SOL/USDT --entry-price 36 --quantity 0.44
  python strategy_simulator.py --interactive
"""

import json
import sys
from dataclasses import dataclass, field
from typing import Optional


# ═══════════════════════════════════════════════════════════════════
# MODELS
# ═══════════════════════════════════════════════════════════════════

@dataclass
class GradualLot:
    """Um lote da venda gradual"""
    sell_percent: float       # % da posição a vender (ex: 30)
    tp_percent: float         # TP% para disparar este lote (ex: 10)
    executed: bool = False
    executed_at: Optional[str] = None
    executed_price: Optional[float] = None
    realized_pnl: Optional[float] = None


@dataclass
class StrategyConfig:
    """Configuração da estratégia — somente o que o usuário define"""
    token: str = "SOL/USDT"
    exchange: str = "OKX"
    entry_price: float = 36.0         # Preço unitário de compra
    tp_percent: float = 10.0          # Take Profit %
    sl_percent: float = 5.0           # Stop Loss %
    fee_percent: float = 0.5          # Taxa da exchange %
    gradual_enabled: bool = False
    gradual_lots: list = field(default_factory=list)
    check_interval_min: int = 15      # Intervalo de monitoramento


@dataclass
class LotResult:
    """Resultado de um lote vendido"""
    lot_index: int
    sell_percent: float
    tp_percent: float
    lot_quantity: float
    sell_price: float
    gross_value: float
    cost: float
    gross_profit: float
    fee: float
    net_profit: float
    net_return_pct: float


@dataclass
class SimulationResult:
    """Resultado completo da simulação"""
    scenario: str
    config: StrategyConfig
    quantity: float = 0.0
    total_cost: float = 0.0
    lot_results: list = field(default_factory=list)
    total_gross_profit: float = 0.0
    total_fees: float = 0.0
    total_net_profit: float = 0.0
    total_net_return_pct: float = 0.0
    sl_loss: Optional[float] = None
    sl_return_pct: Optional[float] = None
    risk_reward_ratio: float = 0.0


# ═══════════════════════════════════════════════════════════════════
# SIMULATOR
# ═══════════════════════════════════════════════════════════════════

class StrategySimulator:
    """Simulador de estratégia Swing Trade"""

    def __init__(self, config: StrategyConfig, quantity: float):
        self.config = config
        self.quantity = quantity
        self.total_cost = config.entry_price * quantity

    def simulate_single_sell(self) -> SimulationResult:
        """Simula venda única (sem gradual) no TP"""
        c = self.config
        result = SimulationResult(
            scenario="Venda Única", config=c,
            quantity=self.quantity, total_cost=self.total_cost,
        )

        # TP
        sell_price = c.entry_price * (1 + c.tp_percent / 100)
        gross_value = self.quantity * sell_price
        gross_profit = gross_value - self.total_cost
        fee = gross_value * (c.fee_percent / 100)
        net_profit = gross_profit - fee

        lot = LotResult(
            lot_index=0,
            sell_percent=100.0,
            tp_percent=c.tp_percent,
            lot_quantity=self.quantity,
            sell_price=sell_price,
            gross_value=gross_value,
            cost=self.total_cost,
            gross_profit=gross_profit,
            fee=fee,
            net_profit=net_profit,
            net_return_pct=(net_profit / self.total_cost) * 100,
        )
        result.lot_results.append(lot)
        result.total_gross_profit = gross_profit
        result.total_fees = fee
        result.total_net_profit = net_profit
        result.total_net_return_pct = (net_profit / self.total_cost) * 100

        # SL
        sl_price = c.entry_price * (1 - c.sl_percent / 100)
        sl_value = self.quantity * sl_price
        sl_loss_gross = sl_value - self.total_cost
        sl_fee = sl_value * (c.fee_percent / 100)
        result.sl_loss = sl_loss_gross - sl_fee
        result.sl_return_pct = (result.sl_loss / self.total_cost) * 100

        # Risk/Reward
        if result.sl_loss and abs(result.sl_loss) > 0:
            result.risk_reward_ratio = abs(net_profit / result.sl_loss)

        return result

    def simulate_gradual_sell(self) -> SimulationResult:
        """Simula venda gradual escalonada — todos os lotes atingem TP"""
        c = self.config
        result = SimulationResult(
            scenario="Venda Gradual", config=c,
            quantity=self.quantity, total_cost=self.total_cost,
        )

        if not c.gradual_lots:
            return result

        for i, lot_config in enumerate(c.gradual_lots):
            lot_pct = lot_config.sell_percent / 100
            lot_qty = self.quantity * lot_pct
            lot_cost = self.total_cost * lot_pct

            sell_price = c.entry_price * (1 + lot_config.tp_percent / 100)
            gross_value = lot_qty * sell_price
            gross_profit = gross_value - lot_cost
            fee = gross_value * (c.fee_percent / 100)
            net_profit = gross_profit - fee

            lot = LotResult(
                lot_index=i,
                sell_percent=lot_config.sell_percent,
                tp_percent=lot_config.tp_percent,
                lot_quantity=lot_qty,
                sell_price=sell_price,
                gross_value=gross_value,
                cost=lot_cost,
                gross_profit=gross_profit,
                fee=fee,
                net_profit=net_profit,
                net_return_pct=(net_profit / lot_cost) * 100,
            )
            result.lot_results.append(lot)
            result.total_gross_profit += gross_profit
            result.total_fees += fee
            result.total_net_profit += net_profit

        result.total_net_return_pct = (result.total_net_profit / self.total_cost) * 100

        # SL (perda máxima = tudo bate SL)
        sl_price = c.entry_price * (1 - c.sl_percent / 100)
        sl_value = self.quantity * sl_price
        sl_loss_gross = sl_value - self.total_cost
        sl_fee = sl_value * (c.fee_percent / 100)
        result.sl_loss = sl_loss_gross - sl_fee
        result.sl_return_pct = (result.sl_loss / self.total_cost) * 100

        # Risk/Reward
        if result.sl_loss and abs(result.sl_loss) > 0:
            result.risk_reward_ratio = abs(result.total_net_profit / result.sl_loss)

        return result

    def simulate_partial_exit(self, lots_hit: int) -> SimulationResult:
        """Simula cenário onde apenas N lotes atingem TP e o resto bate SL"""
        c = self.config
        if not c.gradual_lots:
            return SimulationResult(
                scenario=f"Parcial ({lots_hit} lotes)", config=c,
                quantity=self.quantity, total_cost=self.total_cost,
            )

        result = SimulationResult(
            scenario=f"Parcial: {lots_hit} lote(s) TP + SL no resto",
            config=c, quantity=self.quantity, total_cost=self.total_cost,
        )

        sold_pct = 0.0

        # Lotes que atingiram TP
        for i in range(min(lots_hit, len(c.gradual_lots))):
            lot_config = c.gradual_lots[i]
            lot_pct = lot_config.sell_percent / 100
            lot_qty = self.quantity * lot_pct
            lot_cost = self.total_cost * lot_pct

            sell_price = c.entry_price * (1 + lot_config.tp_percent / 100)
            gross_value = lot_qty * sell_price
            gross_profit = gross_value - lot_cost
            fee = gross_value * (c.fee_percent / 100)
            net_profit = gross_profit - fee

            lot = LotResult(
                lot_index=i,
                sell_percent=lot_config.sell_percent,
                tp_percent=lot_config.tp_percent,
                lot_quantity=lot_qty,
                sell_price=sell_price,
                gross_value=gross_value,
                cost=lot_cost,
                gross_profit=gross_profit,
                fee=fee,
                net_profit=net_profit,
                net_return_pct=(net_profit / lot_cost) * 100,
            )
            result.lot_results.append(lot)
            result.total_gross_profit += gross_profit
            result.total_fees += fee
            result.total_net_profit += net_profit
            sold_pct += lot_config.sell_percent

        # Restante bate SL
        remaining_pct = (100 - sold_pct) / 100
        if remaining_pct > 0.001:
            remaining_qty = self.quantity * remaining_pct
            remaining_cost = self.total_cost * remaining_pct

            sl_price = c.entry_price * (1 - c.sl_percent / 100)
            sl_value = remaining_qty * sl_price
            sl_loss_gross = sl_value - remaining_cost
            sl_fee = sl_value * (c.fee_percent / 100)
            sl_net = sl_loss_gross - sl_fee

            lot = LotResult(
                lot_index=lots_hit,
                sell_percent=remaining_pct * 100,
                tp_percent=-c.sl_percent,
                lot_quantity=remaining_qty,
                sell_price=sl_price,
                gross_value=sl_value,
                cost=remaining_cost,
                gross_profit=sl_loss_gross,
                fee=sl_fee,
                net_profit=sl_net,
                net_return_pct=(sl_net / remaining_cost) * 100,
            )
            result.lot_results.append(lot)
            result.total_gross_profit += sl_loss_gross
            result.total_fees += sl_fee
            result.total_net_profit += sl_net

        result.total_net_return_pct = (result.total_net_profit / self.total_cost) * 100

        # SL puro para risk/reward
        sl_price = c.entry_price * (1 - c.sl_percent / 100)
        sl_value = self.quantity * sl_price
        sl_loss_gross = sl_value - self.total_cost
        sl_fee = sl_value * (c.fee_percent / 100)
        result.sl_loss = sl_loss_gross - sl_fee
        result.sl_return_pct = (result.sl_loss / self.total_cost) * 100

        return result

    def simulate_price_series(self, prices: list[float]) -> list[dict]:
        """Simula monitoramento com série de preços (backtesting)"""
        c = self.config
        events = []
        lots = [{"sell_pct": l.sell_percent, "tp_pct": l.tp_percent, "executed": False}
                for l in c.gradual_lots] if c.gradual_lots else []

        remaining_qty = self.quantity
        total_realized = 0.0
        total_fees = 0.0

        for tick, price in enumerate(prices):
            change_pct = ((price - c.entry_price) / c.entry_price) * 100

            # Check SL
            if change_pct <= -c.sl_percent and remaining_qty > 0.0001:
                sl_value = remaining_qty * price
                sl_cost = (remaining_qty / self.quantity) * self.total_cost
                sl_loss = sl_value - sl_cost
                sl_fee = sl_value * (c.fee_percent / 100)
                sl_net = sl_loss - sl_fee

                events.append({
                    "tick": tick,
                    "price": price,
                    "change_pct": round(change_pct, 2),
                    "event": "STOP_LOSS",
                    "sell_qty": round(remaining_qty, 8),
                    "sell_pct": round((remaining_qty / self.quantity) * 100, 1),
                    "gross_pnl": round(sl_loss, 2),
                    "fee": round(sl_fee, 2),
                    "net_pnl": round(sl_net, 2),
                })
                total_realized += sl_net
                total_fees += sl_fee
                remaining_qty = 0
                break

            # Check TP lotes
            if lots:
                for i, lot in enumerate(lots):
                    if lot["executed"]:
                        continue
                    if change_pct >= lot["tp_pct"]:
                        lot_pct = lot["sell_pct"] / 100
                        lot_qty = self.quantity * lot_pct
                        lot_cost = self.total_cost * lot_pct

                        sell_value = lot_qty * price
                        profit = sell_value - lot_cost
                        fee = sell_value * (c.fee_percent / 100)
                        net = profit - fee

                        lot["executed"] = True
                        remaining_qty -= lot_qty
                        total_realized += net
                        total_fees += fee

                        events.append({
                            "tick": tick,
                            "price": price,
                            "change_pct": round(change_pct, 2),
                            "event": f"TP_LOT_{i+1}",
                            "sell_qty": round(lot_qty, 8),
                            "sell_pct": lot["sell_pct"],
                            "tp_pct": lot["tp_pct"],
                            "gross_pnl": round(profit, 2),
                            "fee": round(fee, 2),
                            "net_pnl": round(net, 2),
                        })
                        break  # Um lote por tick

            # Check TP único (sem gradual)
            elif change_pct >= c.tp_percent and remaining_qty > 0.0001:
                sell_value = remaining_qty * price
                profit = sell_value - self.total_cost
                fee = sell_value * (c.fee_percent / 100)
                net = profit - fee

                events.append({
                    "tick": tick,
                    "price": price,
                    "change_pct": round(change_pct, 2),
                    "event": "TAKE_PROFIT",
                    "sell_qty": round(remaining_qty, 8),
                    "sell_pct": 100.0,
                    "gross_pnl": round(profit, 2),
                    "fee": round(fee, 2),
                    "net_pnl": round(net, 2),
                })
                total_realized += net
                total_fees += fee
                remaining_qty = 0
                break

            if remaining_qty <= 0.0001:
                break

        # Summary
        events.append({
            "tick": "SUMMARY",
            "total_realized_pnl": round(total_realized, 2),
            "total_fees": round(total_fees, 2),
            "remaining_qty": round(remaining_qty, 8),
            "remaining_pct": round((remaining_qty / self.quantity) * 100, 1) if self.quantity > 0 else 0,
            "return_pct": round((total_realized / self.total_cost) * 100, 2) if self.total_cost > 0 else 0,
            "status": "COMPLETED" if remaining_qty <= 0.0001 else "OPEN",
        })

        return events

    def generate_api_payload(self) -> dict:
        """Gera o JSON payload para a API Rust do trading-service"""
        c = self.config
        payload = {
            "name": f"{c.token.split('/')[0]} Swing {'Gradual' if c.gradual_enabled else 'Simple'}",
            "strategy_type": "swing_trade",
            "symbol": c.token,
            "exchange_id": "seu_exchange_id",
            "exchange_name": c.exchange,
            "check_interval_secs": c.check_interval_min * 60,
            "config": {
                "fee_percent": c.fee_percent,
                "stop_loss": {
                    "enabled": True,
                    "percent": c.sl_percent,
                    "trailing": False,
                },
                "mode": "spot",
            }
        }

        if c.gradual_enabled and c.gradual_lots:
            payload["config"]["gradual_sell"] = {
                "enabled": True,
                "lots": [
                    {
                        "sell_percent": lot.sell_percent,
                        "tp_percent": lot.tp_percent,
                        "executed": False,
                    }
                    for lot in c.gradual_lots
                ]
            }
        else:
            payload["config"]["take_profit_levels"] = [
                {
                    "percent": c.tp_percent,
                    "sell_percent": 100.0,
                    "executed": False,
                }
            ]

        return payload


# ═══════════════════════════════════════════════════════════════════
# PRINTER
# ═══════════════════════════════════════════════════════════════════

def print_header(config: StrategyConfig, quantity: float):
    total_cost = config.entry_price * quantity
    print("\n" + "═" * 60)
    print("  STRATEGY SIMULATOR — Swing Trade")
    print("═" * 60)
    print(f"  Token:         {config.token}")
    print(f"  Exchange:      {config.exchange}")
    print(f"  Preço entrada: ${config.entry_price:.2f}")
    print(f"  Quantidade:    {quantity:.6f}")
    print(f"  Custo total:   ${total_cost:.2f}")
    print(f"  TP:            +{config.tp_percent}%")
    print(f"  SL:            -{config.sl_percent}%")
    print(f"  Taxa:          {config.fee_percent}%")
    print(f"  Check:         a cada {config.check_interval_min}min")
    if config.gradual_enabled and config.gradual_lots:
        lots_str = "/".join([f"{int(l.sell_percent)}" for l in config.gradual_lots])
        tps_str = "/".join([f"+{l.tp_percent}%" for l in config.gradual_lots])
        print(f"  Gradual:       [{lots_str}] → TPs [{tps_str}]")
    print("═" * 60)


def print_result(result: SimulationResult):
    print(f"\n{'─' * 60}")
    print(f"  📊 {result.scenario}")
    print(f"{'─' * 60}")

    for lot in result.lot_results:
        marker = "✅" if lot.net_profit >= 0 else "🔴"
        tp_label = f"TP +{lot.tp_percent}%" if lot.tp_percent > 0 else f"SL {lot.tp_percent}%"
        print(f"\n  Lote {lot.lot_index + 1} │ {lot.sell_percent:.0f}% │ "
              f"${lot.cost:.2f} │ {tp_label}")
        print(f"  ├ Vende a:    ${lot.sell_price:.2f}")
        print(f"  ├ Recebe:     ${lot.gross_value:.2f}")
        print(f"  ├ Lucro bruto: ${lot.gross_profit:+.2f}")
        print(f"  ├ Taxa {result.config.fee_percent}%:   -${lot.fee:.2f}")
        print(f"  └ Líquido:    ${lot.net_profit:+.2f} {marker} "
              f"({lot.net_return_pct:+.2f}%)")

    print(f"\n  {'━' * 50}")
    marker = "✅" if result.total_net_profit >= 0 else "🔴"
    print(f"  💰 Lucro líquido total: ${result.total_net_profit:+.2f} {marker}")
    print(f"  📈 Rentabilidade:       {result.total_net_return_pct:+.2f}%")
    print(f"  💸 Total em taxas:      ${result.total_fees:.2f}")
    if result.sl_loss is not None:
        print(f"  🛑 Perda máxima (SL):   ${result.sl_loss:.2f} "
              f"({result.sl_return_pct:+.2f}%)")
    print(f"  ⚖️  Risco/Retorno:      1:{result.risk_reward_ratio:.1f}")
    print(f"  {'━' * 50}")


def print_comparison(single: SimulationResult, gradual: SimulationResult):
    print(f"\n{'═' * 60}")
    print("  📊 COMPARAÇÃO")
    print(f"{'═' * 60}")
    print(f"  {'':20} {'Venda Única':>16}  {'Gradual':>16}")
    print(f"  {'─' * 56}")
    print(f"  {'Lucro líquido':20} ${single.total_net_profit:>+12.2f}  "
          f"${gradual.total_net_profit:>+12.2f}")
    print(f"  {'Rentabilidade':20} {single.total_net_return_pct:>+12.2f}%  "
          f"{gradual.total_net_return_pct:>+12.2f}%")
    print(f"  {'Taxas':20} ${single.total_fees:>12.2f}  "
          f"${gradual.total_fees:>12.2f}")
    print(f"  {'Risco/Retorno':20} {'1:' + f'{single.risk_reward_ratio:.1f}':>12}  "
          f"{'1:' + f'{gradual.risk_reward_ratio:.1f}':>12}")

    if single.total_net_profit > 0:
        diff = gradual.total_net_profit - single.total_net_profit
        diff_pct = (diff / single.total_net_profit) * 100
        print(f"\n  ⚡ Gradual rende ${diff:+.2f} ({diff_pct:+.0f}%) a mais!")
    print(f"{'═' * 60}")


def print_price_events(events: list[dict]):
    print(f"\n{'─' * 60}")
    print("  📈 BACKTESTING — Simulação com série de preços")
    print(f"{'─' * 60}")
    for e in events:
        if e.get("tick") == "SUMMARY":
            print(f"\n  {'━' * 50}")
            print(f"  RESUMO:")
            print(f"  ├ PNL Realizado:  ${e['total_realized_pnl']:+.2f}")
            print(f"  ├ Total Taxas:    ${e['total_fees']:.2f}")
            print(f"  ├ Restante:       {e['remaining_pct']:.1f}%")
            print(f"  ├ Retorno:        {e['return_pct']:+.2f}%")
            print(f"  └ Status:         {e['status']}")
        else:
            marker = "✅" if e["net_pnl"] >= 0 else "🔴"
            print(f"  Tick {e['tick']:3d} │ ${e['price']:.2f} ({e['change_pct']:+.1f}%) "
                  f"│ {e['event']:12} │ ${e['net_pnl']:+.2f} {marker}")


# ═══════════════════════════════════════════════════════════════════
# INTERACTIVE MODE
# ═══════════════════════════════════════════════════════════════════

def interactive_mode():
    print("\n🎯 STRATEGY SIMULATOR — Modo Interativo\n")

    token = input("  Token (ex: SOL/USDT): ").strip() or "SOL/USDT"
    exchange = input("  Exchange (ex: OKX): ").strip() or "OKX"
    entry_price = float(input("  Preço de entrada ($): ") or "36")
    quantity = float(input("  Quantidade comprada: ") or "0.44")
    tp = float(input("  Take Profit (%): ") or "10")
    sl = float(input("  Stop Loss (%): ") or "5")
    fee = float(input("  Taxa exchange (%): ") or "0.5")
    check = int(input("  Intervalo de check (min): ") or "15")

    gradual_input = input("  Venda gradual? (ex: 30,30,20,20 ou vazio): ").strip()

    config = StrategyConfig(
        token=token,
        exchange=exchange,
        entry_price=entry_price,
        tp_percent=tp,
        sl_percent=sl,
        fee_percent=fee,
        check_interval_min=check,
    )

    if gradual_input:
        parts = [float(x.strip()) for x in gradual_input.split(",")]
        total = sum(parts)
        if abs(total - 100) > 0.1:
            print(f"\n  ⚠️  Soma dos lotes = {total}% (deve ser 100%)")
            return None, 0

        config.gradual_enabled = True
        config.gradual_lots = []
        for i, pct in enumerate(parts):
            lot_tp = tp + (i * 5)
            config.gradual_lots.append(GradualLot(sell_percent=pct, tp_percent=lot_tp))

        custom_tp = input(f"  TPs escalonados {[l.tp_percent for l in config.gradual_lots]}. "
                          f"Customizar? (ex: 10,15,20,25 ou Enter): ").strip()
        if custom_tp:
            tps = [float(x.strip()) for x in custom_tp.split(",")]
            for i, t in enumerate(tps):
                if i < len(config.gradual_lots):
                    config.gradual_lots[i].tp_percent = t

    return config, quantity


# ═══════════════════════════════════════════════════════════════════
# MAIN
# ═══════════════════════════════════════════════════════════════════

def main():
    args = sys.argv[1:]

    if "--interactive" in args or "-i" in args:
        result = interactive_mode()
        if not result or not result[0]:
            return
        config, quantity = result
    else:
        # Config padrão: SOL/USDT, entry $36
        config = StrategyConfig(
            token="SOL/USDT",
            exchange="OKX",
            entry_price=36.0,
            tp_percent=10.0,
            sl_percent=5.0,
            fee_percent=0.5,
            check_interval_min=15,
            gradual_enabled=True,
            gradual_lots=[
                GradualLot(sell_percent=30, tp_percent=10),
                GradualLot(sell_percent=30, tp_percent=15),
                GradualLot(sell_percent=20, tp_percent=20),
                GradualLot(sell_percent=20, tp_percent=25),
            ],
        )
        quantity = 0.44  # Parâmetro de simulação (não faz parte do config)

        # Override com args
        for i, arg in enumerate(args):
            if arg == "--token" and i + 1 < len(args):
                config.token = args[i + 1]
            elif arg == "--entry-price" and i + 1 < len(args):
                config.entry_price = float(args[i + 1])
            elif arg == "--quantity" and i + 1 < len(args):
                quantity = float(args[i + 1])
            elif arg == "--tp" and i + 1 < len(args):
                config.tp_percent = float(args[i + 1])
            elif arg == "--sl" and i + 1 < len(args):
                config.sl_percent = float(args[i + 1])
            elif arg == "--fee" and i + 1 < len(args):
                config.fee_percent = float(args[i + 1])

    sim = StrategySimulator(config, quantity)

    # ── HEADER ──
    print_header(config, quantity)

    # ── CENÁRIO 1: Venda Única ──
    single = sim.simulate_single_sell()
    print_result(single)

    # ── CENÁRIO 2: Venda Gradual ──
    if config.gradual_enabled and config.gradual_lots:
        gradual = sim.simulate_gradual_sell()
        print_result(gradual)

        # ── COMPARAÇÃO ──
        print_comparison(single, gradual)

        # ── CENÁRIOS PARCIAIS ──
        print(f"\n{'═' * 60}")
        print("  📊 CENÁRIOS PARCIAIS (TP parcial + SL no restante)")
        print(f"{'═' * 60}")

        for n_lots in range(1, len(config.gradual_lots)):
            partial = sim.simulate_partial_exit(n_lots)
            marker = "✅" if partial.total_net_profit >= 0 else "🔴"
            sold_pct = sum(l.sell_percent for l in config.gradual_lots[:n_lots])
            print(f"\n  {n_lots} lote(s) vendido(s) ({sold_pct:.0f}%) + SL no resto:")
            print(f"  └ Resultado: ${partial.total_net_profit:+.2f} "
                  f"({partial.total_net_return_pct:+.2f}%) {marker}")

    # ── BACKTESTING: preço sobe ──
    print(f"\n{'═' * 60}")
    print("  📈 BACKTESTING — Preço sobe gradualmente até +30%")
    print(f"{'═' * 60}")
    prices = [config.entry_price * (1 + i * 0.01) for i in range(35)]
    events = sim.simulate_price_series(prices)
    print_price_events(events)

    # ── BACKTESTING: preço cai ──
    print(f"\n{'═' * 60}")
    print("  📉 BACKTESTING — Preço cai direto até SL")
    print(f"{'═' * 60}")
    prices_down = [config.entry_price * (1 - i * 0.01) for i in range(10)]
    events_down = sim.simulate_price_series(prices_down)
    print_price_events(events_down)

    # ── JSON para API ──
    print(f"\n{'═' * 60}")
    print("  🚀 JSON PAYLOAD para API (POST /strategies)")
    print(f"{'═' * 60}")
    payload = sim.generate_api_payload()
    print(json.dumps(payload, indent=2))

    print()


if __name__ == "__main__":
    main()
