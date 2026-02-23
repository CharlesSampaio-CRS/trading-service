# Orders API - Documentação

## 🎯 Arquitetura

**Zero Database Architecture** - Orders são buscadas diretamente das exchanges via CCXT.  
Nenhuma persistência em MongoDB - credenciais vêm do MongoDB (descriptografadas) usando JWT.

## 📡 Endpoints

Todos os endpoints requerem autenticação via JWT token no header:
```
Authorization: Bearer {token}
```

### 1. 📊 Buscar Orders

**Endpoint:** `POST /api/v1/orders/fetch/secure`

**Body:** Vazio (user_id vem do JWT)

**Response:**
```json
{
  "success": true,
  "orders": [
    {
      "id": "order_id_123",
      "exchange_id": "mongodb_id",
      "exchange": "Binance",
      "symbol": "BTC/USDT",
      "type": "limit",
      "side": "buy",
      "price": 50000.0,
      "amount": 0.1,
      "filled": 0.0,
      "remaining": 0.1,
      "status": "open",
      "timestamp": 1234567890
    }
  ],
  "count": 1
}
```

**Fluxo:**
1. Backend extrai `user_id` do JWT
2. Busca exchanges do MongoDB com credenciais descriptografadas
3. Para cada exchange, chama CCXT `fetch_open_orders()`
4. Retorna todas as orders agregadas

**Performance:**
- Timeout por exchange: 10 segundos
- Execução paralela para múltiplas exchanges
- MEXC: tratamento especial (itera por símbolos)

---

### 2. ➕ Criar Order

**Endpoint:** `POST /api/v1/orders/create`

**Body:**
```json
{
  "exchange_id": "65abc123...",
  "symbol": "BTC/USDT",
  "order_type": "limit",
  "side": "buy",
  "amount": 0.1,
  "price": 50000.0
}
```

**Fields:**
- `exchange_id` (string, obrigatório): MongoDB ID da exchange
- `symbol` (string, obrigatório): Par de negociação (ex: "BTC/USDT")
- `order_type` (string, obrigatório): "market" ou "limit"
- `side` (string, obrigatório): "buy" ou "sell"  
- `amount` (float, obrigatório): Quantidade a comprar/vender
- `price` (float, opcional): Preço (obrigatório para orders limit)

**Response:**
```json
{
  "success": true,
  "order": {
    "id": "created_order_id",
    "symbol": "BTC/USDT",
    "type": "limit",
    "side": "buy",
    "price": 50000.0,
    "amount": 0.1,
    "status": "open"
  }
}
```

**Fluxo:**
1. Backend extrai `user_id` do JWT
2. Busca exchanges do MongoDB
3. Encontra exchange pelo `exchange_id`
4. Obtém credenciais descriptografadas
5. Chama CCXT `create_order()`
6. Retorna order criada

---

### 3. ❌ Cancelar Order

**Endpoint:** `POST /api/v1/orders/cancel`

**Body:**
```json
{
  "exchange_id": "65abc123...",
  "symbol": "BTC/USDT",
  "order_id": "order_123"
}
```

**Fields:**
- `exchange_id` (string, obrigatório): MongoDB ID da exchange
- `symbol` (string, obrigatório): Par de negociação
- `order_id` (string, obrigatório): ID da ordem a cancelar

**Response:**
```json
{
  "success": true,
  "message": "Order canceled successfully"
}
```

**Fluxo:**
1. Backend extrai `user_id` do JWT
2. Busca exchanges do MongoDB
3. Encontra exchange pelo `exchange_id`
4. Obtém credenciais descriptografadas
5. Chama CCXT `cancel_order(order_id, symbol)`
6. Retorna resultado

---

## 🔒 Segurança

- ✅ **JWT obrigatório** em todos os endpoints
- ✅ **Credenciais nunca expostas** no frontend
- ✅ **User isolation**: cada usuário só acessa suas próprias exchanges
- ✅ **Credenciais descriptografadas** apenas no backend (Fernet encryption)

## ⚡ Performance

**Fetch Orders:**
- Timeout: 10s por exchange
- Execução paralela
- Tratamento especial MEXC (problema conhecido)

**Create/Cancel Orders:**
- Timeout: 12s (TIMEOUTS.NORMAL)
- Síncrono (espera confirmação da exchange)

## 🐛 Error Handling

**Códigos HTTP:**
- `200 OK`: Operação bem-sucedida
- `400 Bad Request`: Erro na validação ou exchange recusou
- `404 Not Found`: Exchange não encontrada
- `500 Internal Server Error`: Erro no backend ou CCXT

**Response de erro:**
```json
{
  "success": false,
  "error": "Exchange not found: 65abc123..."
}
```

## 🚀 Migração do Frontend

**Antes:**
```typescript
// ❌ REMOVIDO - Endpoints antigos
await apiService.getOrders(userId); // /orders (sem JWT)
await apiService.cancelOrder(ccxt_id, apiKey, apiSecret, symbol, orderId); // /orders/cancel-with-creds
```

**Depois:**
```typescript
// ✅ NOVO - Endpoints seguros
await apiService.getOrdersSecure(); // POST /orders/fetch/secure (com JWT)
await apiService.cancelOrderByExchangeId(exchangeId, symbol, orderId); // POST /orders/cancel (com JWT)
await apiService.createOrder(exchangeId, symbol, type, side, amount, price); // POST /orders/create (com JWT)
```

## 📝 Notas

1. **Exchange ID**: Sempre use o MongoDB `_id` da exchange, não o `ccxt_id`
2. **Symbol Format**: Use formato CCXT (ex: "BTC/USDT", não "BTCUSDT")
3. **MEXC Orders**: Podem ser lentas devido ao algoritmo especial (itera símbolos)
4. **Rate Limits**: CCXT respeita rate limits de cada exchange automaticamente

## 🔄 Changelog

**v2.0 - 23/02/2026:**
- ✅ Simplificação total: removidos endpoints com credenciais do frontend
- ✅ Todos endpoints agora usam JWT + MongoDB
- ✅ Criado endpoint `/create` para criar orders
- ✅ Endpoint `/cancel` simplificado
- ✅ Performance otimizada: fetch paralelo de orders
- ✅ Logs melhorados para debug
