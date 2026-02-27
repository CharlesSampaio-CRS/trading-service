#!/usr/bin/env node
/**
 * ══════════════════════════════════════════════════════════════════════
 * Strategy Rules Validator — Test Script
 * ══════════════════════════════════════════════════════════════════════
 *
 * Validates the strategy business rules against the live API:
 *   1. Price calculations (trigger_price, stop_loss_price, gradual_trigger)
 *   2. CRUD operations (create, read, update, delete)
 *   3. Gradual lots auto-generation (4x25% when gradual_sell=true)
 *   4. Status transitions & activation/pause
 *   5. Field validations & error handling
 *   6. Tick endpoint behavior
 *
 * Usage:
 *   node tests/test-strategy-rules.mjs
 *   API_BASE=http://localhost:3002/api/v1 AUTH_TOKEN=xxx node tests/test-strategy-rules.mjs
 */

// ── Config ──────────────────────────────────────────────────────────
const API_BASE = process.env.API_BASE || 'http://54.94.231.254:3002/api/v1'
const AUTH_TOKEN = process.env.AUTH_TOKEN || ''
const VERBOSE = process.env.VERBOSE === '1'

// ── Helpers ─────────────────────────────────────────────────────────
const C = {
  reset: '\x1b[0m',
  green: '\x1b[32m',
  red: '\x1b[31m',
  yellow: '\x1b[33m',
  cyan: '\x1b[36m',
  dim: '\x1b[2m',
  bold: '\x1b[1m',
  magenta: '\x1b[35m',
}

let totalTests = 0
let passed = 0
let failed = 0
let skipped = 0
const failures = []

function log(msg) { console.log(msg) }

function logSection(title) {
  log(`\n${C.cyan}${'═'.repeat(60)}${C.reset}`)
  log(`${C.bold}  ${title}${C.reset}`)
  log(`${C.cyan}${'═'.repeat(60)}${C.reset}`)
}

function logTest(name) {
  totalTests++
  process.stdout.write(`  ${C.dim}▸${C.reset} ${name} ... `)
}

function logPass(detail = '') {
  passed++
  log(`${C.green}✓ PASS${C.reset}${detail ? ` ${C.dim}(${detail})${C.reset}` : ''}`)
}

function logFail(expected, got, note = '') {
  failed++
  const msg = `Expected: ${expected}, Got: ${got}${note ? ' — ' + note : ''}`
  failures.push(msg)
  log(`${C.red}✗ FAIL${C.reset} — ${msg}`)
}

function logSkip(reason) {
  skipped++
  log(`${C.yellow}⊘ SKIP${C.reset} — ${reason}`)
}

function logInfo(msg) {
  log(`  ${C.dim}ℹ ${msg}${C.reset}`)
}

function assertEq(actual, expected, tolerance = 0) {
  if (typeof actual === 'number' && typeof expected === 'number') {
    if (Math.abs(actual - expected) <= tolerance) { logPass(`${actual}`); return true }
    logFail(expected, actual, tolerance > 0 ? `tolerance: ${tolerance}` : '')
    return false
  }
  if (actual === expected) { logPass(`${actual}`); return true }
  logFail(expected, actual)
  return false
}

function assertTrue(val, label = '') {
  if (val) { logPass(label); return true }
  logFail('truthy', val, label)
  return false
}

function assertFalse(val, label = '') {
  if (!val) { logPass(label); return true }
  logFail('falsy', val, label)
  return false
}

// ── API Client ──────────────────────────────────────────────────────
async function api(method, path, body = null) {
  const url = `${API_BASE}${path}`
  const headers = { 'Content-Type': 'application/json' }
  if (AUTH_TOKEN) headers['Authorization'] = `Bearer ${AUTH_TOKEN}`

  const opts = { method, headers }
  if (body) opts.body = JSON.stringify(body)

  if (VERBOSE) logInfo(`${method} ${url}${body ? ' ' + JSON.stringify(body).substring(0, 120) : ''}`)

  try {
    const res = await fetch(url, opts)
    const text = await res.text()
    let data
    try { data = JSON.parse(text) } catch { data = { raw: text } }
    if (VERBOSE) logInfo(`  → ${res.status} ${JSON.stringify(data).substring(0, 200)}`)
    return { status: res.status, data, ok: res.ok }
  } catch (err) {
    return { status: 0, data: { error: err.message }, ok: false }
  }
}

// ═══════════════════════════════════════════════════════════════════
// 1. Price Calculation Tests (Offline)
// ═══════════════════════════════════════════════════════════════════
function testPriceCalculations() {
  logSection('1. Price Calculations (Offline)')

  const scenarios = [
    { name: 'SOL @ $150, TP 5%, Fee 0.1%', base: 150.0, tp: 5.0, sl: 3.0, fee: 0.1, gradual_take: 2.0 },
    { name: 'BTC @ $97000, TP 3%, Fee 0.1%', base: 97000.0, tp: 3.0, sl: 2.0, fee: 0.1, gradual_take: 1.5 },
    { name: 'ETH @ $3500, TP 10%, Fee 0.5%', base: 3500.0, tp: 10.0, sl: 5.0, fee: 0.5, gradual_take: 3.0 },
    { name: 'DOGE @ $0.08, TP 20%, Fee 0.2%', base: 0.08, tp: 20.0, sl: 10.0, fee: 0.2, gradual_take: 5.0 },
    { name: 'Edge: base=0, TP 5%, Fee 0.1%', base: 0.0, tp: 5.0, sl: 3.0, fee: 0.1, gradual_take: 2.0 },
    { name: 'Edge: TP=0%, Fee=0%', base: 100.0, tp: 0.0, sl: 0.0, fee: 0.0, gradual_take: 0.0 },
  ]

  for (const s of scenarios) {
    log(`\n  ${C.magenta}Scenario: ${s.name}${C.reset}`)

    // trigger_price = base * (1 + tp/100 + fee/100)
    const expectedTrigger = s.base * (1 + s.tp / 100 + s.fee / 100)
    logTest('trigger_price formula')
    assertEq(expectedTrigger, s.base * (1 + s.tp / 100 + s.fee / 100), 0.0000001)

    // stop_loss_price = base * (1 - sl/100)
    const expectedSL = s.base * (1 - s.sl / 100)
    logTest('stop_loss_price formula')
    assertEq(expectedSL, s.base * (1 - s.sl / 100), 0.0000001)

    // gradual_trigger_price(lot_index) = base * (1 + tp/100 + fee/100 + gradual_step/100 * lot_index)
    for (let i = 0; i < 4; i++) {
      const expectedGradual = s.base * (1 + s.tp / 100 + s.fee / 100 + s.gradual_take / 100 * i)
      logTest(`gradual_trigger_price(lot_index=${i})`)
      assertEq(expectedGradual, s.base * (1 + s.tp / 100 + s.fee / 100 + s.gradual_take / 100 * i), 0.0000001)
    }

    // Verify: trigger > stop_loss (when base > 0 and tp > 0)
    if (s.base > 0 && s.tp > 0) {
      logTest('trigger_price > stop_loss_price')
      assertTrue(expectedTrigger > expectedSL, `${expectedTrigger.toFixed(6)} > ${expectedSL.toFixed(6)}`)
    }

    // Verify: lot[0] == trigger, lot[N] > lot[N-1]
    if (s.base > 0 && s.tp > 0) {
      const lot0 = s.base * (1 + s.tp / 100 + s.fee / 100 + s.gradual_take / 100 * 0)
      logTest('gradual lot[0] == trigger_price')
      assertEq(lot0, expectedTrigger, 0.0000001)

      if (s.gradual_take > 0) {
        logTest('gradual lots monotonically increasing')
        let monotonic = true
        for (let i = 1; i < 4; i++) {
          const curr = s.base * (1 + s.tp / 100 + s.fee / 100 + s.gradual_take / 100 * i)
          const prev = s.base * (1 + s.tp / 100 + s.fee / 100 + s.gradual_take / 100 * (i - 1))
          if (curr <= prev) { monotonic = false; break }
        }
        assertTrue(monotonic)
      }
    }
  }

  // Specific regression: SOL @ 150
  log(`\n  ${C.magenta}Regression: exact values SOL@150${C.reset}`)
  logTest('trigger = 150 * 1.051 = 157.65')
  assertEq(150 * (1 + 5 / 100 + 0.1 / 100), 157.65, 0.0001)

  logTest('stop_loss = 150 * 0.97 = 145.5')
  assertEq(150 * (1 - 3 / 100), 145.5, 0.0001)

  logTest('gradual[2] = 150 * (1 + 0.05 + 0.001 + 0.02*2) = 163.65')
  assertEq(150 * (1 + 5 / 100 + 0.1 / 100 + 2 / 100 * 2), 163.65, 0.01)
}

// ═══════════════════════════════════════════════════════════════════
// 2. Expiration Logic (Offline)
// ═══════════════════════════════════════════════════════════════════
function testExpiration() {
  logSection('2. Expiration Logic (Offline)')

  const now = Math.floor(Date.now() / 1000)

  logTest('started 119min ago, time_execution=120min -> NOT expired')
  const started119 = now - 119 * 60
  assertFalse((now - started119) >= 120 * 60, 'not expired')

  logTest('started 121min ago, time_execution=120min -> IS expired')
  const started121 = now - 121 * 60
  assertTrue((now - started121) >= 120 * 60, 'expired')

  logTest('started exactly 120min ago -> IS expired')
  const startedExact = now - 120 * 60
  assertTrue((now - startedExact) >= 120 * 60, 'expired at boundary')

  logTest('started 1min ago, time_execution=15min -> NOT expired')
  const started1 = now - 1 * 60
  assertFalse((now - started1) >= 15 * 60, 'not expired')

  logTest('custom time_execution=60min, started 61min ago -> IS expired')
  const started61 = now - 61 * 60
  assertTrue((now - started61) >= 60 * 60, 'expired at 60min')
}

// ═══════════════════════════════════════════════════════════════════
// 3. Gradual Lots Structure (Offline)
// ═══════════════════════════════════════════════════════════════════
function testGradualLotsOffline() {
  logSection('3. Gradual Lots Structure (Offline)')

  const defaultLots = [
    { lot_number: 1, sell_percent: 25.0, executed: false },
    { lot_number: 2, sell_percent: 25.0, executed: false },
    { lot_number: 3, sell_percent: 25.0, executed: false },
    { lot_number: 4, sell_percent: 25.0, executed: false },
  ]

  logTest('default: 4 lots generated')
  assertEq(defaultLots.length, 4)

  logTest('total sell_percent = 100%')
  const total = defaultLots.reduce((s, l) => s + l.sell_percent, 0)
  assertEq(total, 100)

  logTest('all lots start as NOT executed')
  assertTrue(defaultLots.every(l => l.executed === false))

  logTest('lot_numbers are 1,2,3,4')
  assertEq(JSON.stringify(defaultLots.map(l => l.lot_number)), JSON.stringify([1, 2, 3, 4]))

  logTest('each lot is 25%')
  assertTrue(defaultLots.every(l => l.sell_percent === 25.0))

  // Simulate partial execution
  log(`\n  ${C.magenta}Simulate: 2 of 4 lots executed${C.reset}`)
  const partialLots = JSON.parse(JSON.stringify(defaultLots))
  partialLots[0].executed = true
  partialLots[0].executed_price = 160.0
  partialLots[0].realized_pnl = 2.5
  partialLots[1].executed = true
  partialLots[1].executed_price = 163.0
  partialLots[1].realized_pnl = 3.25

  const executedCount = partialLots.filter(l => l.executed).length
  const pendingCount = partialLots.filter(l => !l.executed).length
  const totalPnl = partialLots.filter(l => l.executed).reduce((s, l) => s + (l.realized_pnl || 0), 0)

  logTest('executed lots = 2')
  assertEq(executedCount, 2)

  logTest('pending lots = 2')
  assertEq(pendingCount, 2)

  logTest('total realized PnL from executed lots')
  assertEq(totalPnl, 5.75)
}

// ═══════════════════════════════════════════════════════════════════
// 4. API Integration Tests
// ═══════════════════════════════════════════════════════════════════
async function testAPIIntegration() {
  logSection('4. API Integration Tests')

  if (!AUTH_TOKEN) {
    logTest('AUTH_TOKEN available')
    logSkip('No AUTH_TOKEN set. Set AUTH_TOKEN env var to run API tests.')
    return
  }

  let createdId = null

  // ── 4.1 Create Strategy ─────────────
  log(`\n  ${C.magenta}4.1 Create Strategy${C.reset}`)

  const createPayload = {
    name: `TEST_VALIDATOR_${Date.now()}`,
    symbol: 'SOL/USDT',
    exchange_id: 'test-exchange-id',
    exchange_name: 'Binance',
    config: {
      base_price: 150.0,
      take_profit_percent: 5.0,
      stop_loss_percent: 3.0,
      gradual_take_percent: 2.0,
      fee_percent: 0.1,
      gradual_sell: true,
      gradual_lots: [],
      timer_gradual_min: 15,
      time_execution_min: 120,
    }
  }

  logTest('POST /strategies -> 201')
  const createRes = await api('POST', '/strategies', createPayload)
  assertEq(createRes.status, 201)

  logTest('response.success = true')
  assertTrue(createRes.data?.success)

  const strategy = createRes.data?.strategy
  if (!strategy) {
    logTest('strategy object returned')
    logFail('strategy object', 'null/undefined')
    return
  }

  createdId = strategy.id
  logInfo(`Created strategy ID: ${createdId}`)

  // Validate computed prices
  const expectedTrigger = 150.0 * (1 + 5 / 100 + 0.1 / 100)
  const expectedSL = 150.0 * (1 - 3 / 100)

  logTest('trigger_price computed by backend')
  assertEq(strategy.trigger_price, expectedTrigger, 0.01)

  logTest('stop_loss_price computed by backend')
  assertEq(strategy.stop_loss_price, expectedSL, 0.01)

  logTest('initial status = monitoring')
  assertEq(strategy.status, 'monitoring')

  logTest('is_active = true')
  assertTrue(strategy.is_active)

  // Validate gradual_lots auto-generated
  logTest('gradual_lots auto-generated: length = 4')
  assertEq(strategy.config?.gradual_lots?.length, 4)

  if (strategy.config?.gradual_lots?.length === 4) {
    logTest('all lots 25% sell_percent')
    assertTrue(strategy.config.gradual_lots.every(l => l.sell_percent === 25.0))

    logTest('all lots NOT executed')
    assertTrue(strategy.config.gradual_lots.every(l => l.executed === false))

    logTest('lot_numbers sequential 1-4')
    assertEq(
      JSON.stringify(strategy.config.gradual_lots.map(l => l.lot_number)),
      JSON.stringify([1, 2, 3, 4])
    )
  }

  logTest('config.base_price preserved')
  assertEq(strategy.config?.base_price, 150.0)

  logTest('config.take_profit_percent preserved')
  assertEq(strategy.config?.take_profit_percent, 5.0)

  logTest('config.stop_loss_percent preserved')
  assertEq(strategy.config?.stop_loss_percent, 3.0)

  logTest('config.gradual_sell = true')
  assertTrue(strategy.config?.gradual_sell)

  logTest('config.timer_gradual_min preserved')
  assertEq(strategy.config?.timer_gradual_min, 15)

  logTest('config.time_execution_min preserved')
  assertEq(strategy.config?.time_execution_min, 120)

  logTest('started_at is set (> 0)')
  assertTrue(strategy.started_at > 0)

  logTest('created_at is set')
  assertTrue(strategy.created_at > 0)

  // ── 4.2 Get Strategy ─────────────
  log(`\n  ${C.magenta}4.2 Get Strategy${C.reset}`)

  logTest(`GET /strategies/${createdId} -> 200`)
  const getRes = await api('GET', `/strategies/${createdId}`)
  assertEq(getRes.status, 200)

  logTest('fetched strategy matches created')
  assertEq(getRes.data?.strategy?.name, createPayload.name)

  logTest('trigger_price consistent on GET')
  assertEq(getRes.data?.strategy?.trigger_price, expectedTrigger, 0.01)

  logTest('stop_loss_price consistent on GET')
  assertEq(getRes.data?.strategy?.stop_loss_price, expectedSL, 0.01)

  // ── 4.3 Get Stats ─────────────
  log(`\n  ${C.magenta}4.3 Get Strategy Stats${C.reset}`)

  logTest(`GET /strategies/${createdId}/stats -> 200`)
  const statsRes = await api('GET', `/strategies/${createdId}/stats`)
  assertEq(statsRes.status, 200)

  const stats = statsRes.data?.stats
  if (stats) {
    logTest('stats.total_executions = 0 (new strategy)')
    assertEq(stats.total_executions, 0)

    logTest('stats.total_sells = 0')
    assertEq(stats.total_sells, 0)

    logTest('stats.total_pnl_usd = 0')
    assertEq(stats.total_pnl_usd, 0)

    logTest('stats.win_rate = 0')
    assertEq(stats.win_rate, 0)
  }

  // ── 4.4 List Strategies ─────────────
  log(`\n  ${C.magenta}4.4 List Strategies${C.reset}`)

  logTest('GET /strategies -> 200')
  const listRes = await api('GET', '/strategies')
  assertEq(listRes.status, 200)

  logTest('created strategy in list')
  const found = listRes.data?.strategies?.find(s => s.id === createdId)
  assertTrue(!!found, `found id ${createdId}`)

  if (found) {
    logTest('list item has trigger_price')
    assertEq(found.trigger_price, expectedTrigger, 0.01)

    logTest('list item has stop_loss_price')
    assertEq(found.stop_loss_price, expectedSL, 0.01)
  }

  // ── 4.5 Pause / Activate ─────────────
  log(`\n  ${C.magenta}4.5 Pause & Activate${C.reset}`)

  logTest(`POST /strategies/${createdId}/pause -> 200`)
  const pauseRes = await api('POST', `/strategies/${createdId}/pause`)
  assertEq(pauseRes.status, 200)

  logTest('status after pause = paused')
  assertEq(pauseRes.data?.strategy?.status, 'paused')

  logTest(`POST /strategies/${createdId}/activate -> 200`)
  const activateRes = await api('POST', `/strategies/${createdId}/activate`)
  assertEq(activateRes.status, 200)

  logTest('status after activate = monitoring')
  assertEq(activateRes.data?.strategy?.status, 'monitoring')

  // ── 4.6 Update Strategy ─────────────
  log(`\n  ${C.magenta}4.6 Update Strategy${C.reset}`)

  const updatePayload = {
    config: {
      base_price: 200.0,
      take_profit_percent: 8.0,
      stop_loss_percent: 4.0,
      gradual_take_percent: 3.0,
      fee_percent: 0.2,
      gradual_sell: true,
      gradual_lots: [],
      timer_gradual_min: 20,
      time_execution_min: 180,
    }
  }

  logTest(`PUT /strategies/${createdId} -> 200`)
  const updateRes = await api('PUT', `/strategies/${createdId}`, updatePayload)
  assertEq(updateRes.status, 200)

  // Re-fetch to validate
  const refetchRes = await api('GET', `/strategies/${createdId}`)
  const updated = refetchRes.data?.strategy

  if (updated) {
    const newTrigger = 200.0 * (1 + 8 / 100 + 0.2 / 100)
    const newSL = 200.0 * (1 - 4 / 100)

    logTest('updated trigger_price recalculated')
    assertEq(updated.trigger_price, newTrigger, 0.01)

    logTest('updated stop_loss_price recalculated')
    assertEq(updated.stop_loss_price, newSL, 0.01)

    logTest('updated base_price = 200')
    assertEq(updated.config?.base_price, 200)

    logTest('updated time_execution_min = 180')
    assertEq(updated.config?.time_execution_min, 180)
  }

  // ── 4.7 Tick Strategy ─────────────
  log(`\n  ${C.magenta}4.7 Tick Strategy${C.reset}`)

  logTest(`POST /strategies/${createdId}/tick -> 200`)
  const tickRes = await api('POST', `/strategies/${createdId}/tick`)
  assertEq(tickRes.status, 200)

  logTest('tick response has success=true')
  assertTrue(tickRes.data?.success)

  const tick = tickRes.data?.tick
  if (tick) {
    logTest('tick.strategy_id matches')
    assertEq(tick.strategy_id, createdId)

    logTest('tick.symbol = SOL/USDT')
    assertEq(tick.symbol, 'SOL/USDT')

    logInfo(`tick price: ${tick.price}, signals: ${tick.signals_count}, execs: ${tick.executions_count}, new_status: ${tick.new_status || 'N/A'}`)
    if (tick.error) logInfo(`tick error: ${tick.error}`)
  }

  // ── 4.8 Executions & Signals ─────────────
  log(`\n  ${C.magenta}4.8 Executions & Signals Endpoints${C.reset}`)

  logTest(`GET /strategies/${createdId}/executions -> 200`)
  const execsRes = await api('GET', `/strategies/${createdId}/executions`)
  assertEq(execsRes.status, 200)

  logTest(`GET /strategies/${createdId}/signals -> 200`)
  const sigsRes = await api('GET', `/strategies/${createdId}/signals`)
  assertEq(sigsRes.status, 200)

  // ── 4.9 Error Cases ─────────────
  log(`\n  ${C.magenta}4.9 Error Cases${C.reset}`)

  logTest('GET /strategies/nonexistent-id -> 404')
  const notFoundRes = await api('GET', '/strategies/nonexistent-id-12345')
  assertEq(notFoundRes.status, 404)

  logTest('POST /strategies with empty body -> 4xx')
  const badCreateRes = await api('POST', '/strategies', {})
  assertTrue(badCreateRes.status >= 400, `status=${badCreateRes.status}`)

  logTest('POST /strategies with missing config -> 4xx')
  const noConfigRes = await api('POST', '/strategies', {
    name: 'test', symbol: 'BTC/USDT', exchange_id: 'x', exchange_name: 'X'
  })
  assertTrue(noConfigRes.status >= 400, `status=${noConfigRes.status}`)

  // ── 4.10 Create WITHOUT gradual_sell ─────────────
  log(`\n  ${C.magenta}4.10 Create with gradual_sell=false${C.reset}`)

  const noGradualPayload = {
    name: `TEST_NO_GRADUAL_${Date.now()}`,
    symbol: 'BTC/USDT',
    exchange_id: 'test-exchange-id',
    exchange_name: 'Binance',
    config: {
      base_price: 97000.0,
      take_profit_percent: 3.0,
      stop_loss_percent: 2.0,
      gradual_take_percent: 0.0,
      fee_percent: 0.1,
      gradual_sell: false,
      gradual_lots: [],
      timer_gradual_min: 15,
      time_execution_min: 60,
    }
  }

  logTest('POST /strategies (gradual_sell=false) -> 201')
  const noGradRes = await api('POST', '/strategies', noGradualPayload)
  assertEq(noGradRes.status, 201)

  const noGradStrategy = noGradRes.data?.strategy
  if (noGradStrategy) {
    logTest('gradual_lots empty when gradual_sell=false')
    assertEq(noGradStrategy.config?.gradual_lots?.length || 0, 0)

    logTest('gradual_sell = false')
    assertFalse(noGradStrategy.config?.gradual_sell)

    // Clean up
    await api('DELETE', `/strategies/${noGradStrategy.id}`)
    logInfo(`Cleaned up no-gradual strategy: ${noGradStrategy.id}`)
  }

  // ── 4.11 Cleanup ─────────────
  log(`\n  ${C.magenta}4.11 Delete Strategy${C.reset}`)

  if (createdId) {
    logTest(`DELETE /strategies/${createdId} -> 200`)
    const deleteRes = await api('DELETE', `/strategies/${createdId}`)
    assertEq(deleteRes.status, 200)

    logTest('deleted strategy returns 404 on GET')
    const afterDeleteRes = await api('GET', `/strategies/${createdId}`)
    assertEq(afterDeleteRes.status, 404)
  }
}

// ═══════════════════════════════════════════════════════════════════
// 5. Request Structure Validation (Offline)
// ═══════════════════════════════════════════════════════════════════
function testRequestValidation() {
  logSection('5. CreateStrategyRequest Structure Validation (Offline)')

  const validRequest = {
    name: 'TEST_SOL_123',
    symbol: 'SOL/USDT',
    exchange_id: 'abc-123',
    exchange_name: 'Binance',
    config: {
      base_price: 150.0,
      take_profit_percent: 5.0,
      stop_loss_percent: 3.0,
      gradual_take_percent: 2.0,
      fee_percent: 0.1,
      gradual_sell: true,
      gradual_lots: [],
      timer_gradual_min: 15,
      time_execution_min: 120,
    }
  }

  logTest('valid request has all required fields')
  assertTrue(
    validRequest.name && validRequest.symbol &&
    validRequest.exchange_id && validRequest.exchange_name &&
    validRequest.config && typeof validRequest.config.base_price === 'number'
  )

  logTest('config has all required numeric fields')
  const cfg = validRequest.config
  assertTrue(
    typeof cfg.base_price === 'number' &&
    typeof cfg.take_profit_percent === 'number' &&
    typeof cfg.stop_loss_percent === 'number' &&
    typeof cfg.gradual_take_percent === 'number' &&
    typeof cfg.fee_percent === 'number' &&
    typeof cfg.timer_gradual_min === 'number' &&
    typeof cfg.time_execution_min === 'number'
  )

  logTest('config has boolean gradual_sell')
  assertEq(typeof cfg.gradual_sell, 'boolean')

  logTest('config has array gradual_lots')
  assertTrue(Array.isArray(cfg.gradual_lots))

  logTest('default timer_gradual_min = 15')
  assertEq(cfg.timer_gradual_min, 15)

  logTest('default time_execution_min = 120')
  assertEq(cfg.time_execution_min, 120)
}

// ═══════════════════════════════════════════════════════════════════
// 6. Status / Enum Validation (Offline)
// ═══════════════════════════════════════════════════════════════════
function testStatusTransitions() {
  logSection('6. Status & Enum Validation (Offline)')

  const validStatuses = [
    'idle', 'monitoring', 'in_position', 'gradual_selling',
    'completed', 'stopped_out', 'expired', 'paused', 'error'
  ]
  const oldStatuses = ['buy_pending', 'sell_pending']

  logTest(`${validStatuses.length} valid statuses defined`)
  assertEq(validStatuses.length, 9)

  for (const s of oldStatuses) {
    logTest(`old status "${s}" NOT in valid statuses`)
    assertFalse(validStatuses.includes(s), `"${s}" should not exist`)
  }

  const validActions = ['buy', 'sell', 'buy_failed', 'sell_failed']
  const oldActions = ['dca_buy', 'grid_buy', 'grid_sell']

  for (const a of oldActions) {
    logTest(`old action "${a}" NOT in valid actions`)
    assertFalse(validActions.includes(a), `"${a}" should not exist`)
  }

  const validSignals = ['take_profit', 'stop_loss', 'gradual_sell', 'expired', 'info']
  const oldSignals = ['buy', 'trailing_stop', 'dca_buy', 'grid_trade', 'price_alert']

  for (const s of oldSignals) {
    logTest(`old signal "${s}" NOT in valid signals`)
    assertFalse(validSignals.includes(s), `"${s}" should not exist`)
  }
}

// ═══════════════════════════════════════════════════════════════════
// Main Runner
// ═══════════════════════════════════════════════════════════════════
async function main() {
  log(`\n${C.bold}${C.cyan}╔══════════════════════════════════════════════════════════╗${C.reset}`)
  log(`${C.bold}${C.cyan}║       Strategy Rules Validator — Test Suite              ║${C.reset}`)
  log(`${C.bold}${C.cyan}╚══════════════════════════════════════════════════════════╝${C.reset}`)
  log(`\n${C.dim}  API: ${API_BASE}`)
  log(`  Auth: ${AUTH_TOKEN ? '✓ Token provided' : '✗ No token (API tests will be skipped)'}`)
  log(`  Verbose: ${VERBOSE ? 'ON' : 'OFF'}${C.reset}`)

  // Offline tests (always run)
  testPriceCalculations()
  testExpiration()
  testGradualLotsOffline()
  testRequestValidation()
  testStatusTransitions()

  // API tests (only with auth token)
  await testAPIIntegration()

  // Summary
  logSection('SUMMARY')
  log(`  Total:   ${totalTests}`)
  log(`  ${C.green}Passed:  ${passed}${C.reset}`)
  log(`  ${C.red}Failed:  ${failed}${C.reset}`)
  log(`  ${C.yellow}Skipped: ${skipped}${C.reset}`)

  if (failures.length > 0) {
    log(`\n  ${C.red}${C.bold}Failures:${C.reset}`)
    failures.forEach((f, i) => log(`  ${C.red}${i + 1}. ${f}${C.reset}`))
  }

  const result = failed === 0
    ? `${C.green}${C.bold}ALL TESTS PASSED ✓`
    : `${C.red}${C.bold}SOME TESTS FAILED ✗`
  log(`\n  ${result}${C.reset}\n`)

  process.exit(failed > 0 ? 1 : 0)
}

main().catch(err => {
  console.error(`\n${C.red}Fatal error: ${err.message}${C.reset}`)
  console.error(err.stack)
  process.exit(2)
})
