# 📚 Swagger/OpenAPI Documentation

## Acesso à Documentação

Após iniciar o servidor, a documentação Swagger estará disponível em:

- **Swagger UI**: http://localhost:3002/swagger-ui/
- **OpenAPI Spec (JSON)**: http://localhost:3002/api-docs/openapi.json

## 📋 Endpoints Documentados no Swagger

### Authentication (Auth)
- `POST /api/v1/auth/login` - Login with email/password
- `POST /api/v1/auth/register` - Register new user (local, Google, Apple)

### Health & Metrics
- `GET /health` - Health check
- `GET /metrics` - System metrics (Prometheus format)

---

## 🔒 Endpoints **NÃO** Documentados (Por Design)

Os seguintes endpoints **NÃO aparecem no Swagger** por razões de segurança e arquitetura:

### CCXT Integration (Zero-Database Architecture)
- `/api/v1/balances/*` - Real-time balance fetching
- `/api/v1/orders/*` - Order creation and management
- `/api/v1/tickers/*` - Real-time price tickers

### External APIs
- `/api/v1/external/token/*` - CoinGecko token info
- `/api/v1/external/exchange-rate` - Currency conversion
- `/api/v1/external/convert` - Currency converter
- `/api/v1/external/rates` - All exchange rates

### Catalog Data
- `/api/v1/exchanges/*` - Exchange catalog
- `/api/v1/tokens/*` - Token catalog

**Razão:** Estes endpoints requerem credenciais dinâmicas e operam em arquitetura Zero-Database, onde:
- Credenciais são enviadas pelo frontend em cada request
- Sem armazenamento persistente de dados sensíveis
- Documentação Swagger seria confusa e potencialmente insegura

---

## 🔐 Autenticação no Swagger

### 1. Fazer Login

Primeiro, faça login usando o endpoint `/api/v1/auth/login` ou `/api/v1/auth/register`:

```json
POST /api/v1/auth/login
{
  "email": "user@example.com",
  "password": "yourpassword"
}
```

### 2. Copiar o Token JWT

Na resposta, copie o valor do campo `token`:

```json
{
  "success": true,
  "token": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
  "user": {
    "id": "...",
    "email": "user@example.com"
  }
}
```

### 3. Autenticar no Swagger UI

1. No topo da página do Swagger UI, clique no botão **"Authorize"** 🔓
2. No campo "Value", digite: `Bearer seu_token_aqui`
   - Exemplo: `Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...`
3. Clique em **"Authorize"**
4. Clique em **"Close"**

Agora todos os endpoints protegidos podem ser testados diretamente no Swagger UI!

---

## 🚀 Adicionar Novos Endpoints ao Swagger

Para adicionar um novo endpoint à documentação:

### 1. Adicionar anotação ao handler

```rust
#[utoipa::path(
    get,
    path = "/api/v1/exemplo",
    tag = "ExemploTag",
    responses(
        (status = 200, description = "Success", body = ExemploResponse),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])  // Para endpoints que requerem autenticação
    )
)]
pub async fn exemplo_handler() -> HttpResponse {
    // ...
}
```

### 2. Adicionar schemas com ToSchema

```rust
#[derive(Serialize, Deserialize, utoipa::ToSchema)]
pub struct ExemploResponse {
    pub success: bool,
    pub data: String,
}
```

### 3. Registrar no swagger.rs

```rust
#[derive(OpenApi)]
#[openapi(
    paths(
        crate::api::exemplo::exemplo_handler,
    ),
    components(
        schemas(
            crate::services::exemplo::ExemploResponse,
        )
    ),
    tags(
        (name = "ExemploTag", description = "Descrição da tag"),
    )
)]
pub struct ApiDoc;
```

---

## 🎯 Filosofia de Documentação

### O que documentar no Swagger:
✅ Endpoints de autenticação  
✅ Endpoints de sistema (health, metrics)  
✅ Endpoints de gerenciamento de usuários  
✅ Endpoints que usam apenas JWT para autenticação  

### O que NÃO documentar:
❌ Endpoints que requerem credenciais de exchange  
❌ Endpoints de proxy para APIs externas (CCXT)  
❌ Endpoints de arquitetura Zero-Database  
❌ Endpoints com lógica complexa de credenciais dinâmicas  

**Motivo:** Swagger é ideal para APIs tradicionais REST com autenticação simples. Para arquiteturas Zero-Database e proxy CCXT, documentação em Markdown é mais apropriada.

---

## 🛠️ Recursos do Swagger

### Segurança
- **JWT Bearer Authentication** - Todos os endpoints protegidos requerem token JWT
- Schema de segurança configurado globalmente
- Headers de segurança aplicados automaticamente

### Schemas
- Todos os request/response bodies documentados
- Validação de tipos automática
- Exemplos gerados automaticamente

### Tags
- Endpoints organizados por categoria
- Fácil navegação e descoberta

---

## 📦 Dependências

```toml
utoipa = { version = "5", features = ["actix_extras"] }
utoipa-swagger-ui = { version = "8", features = ["actix-web"] }
```

---

## 🔗 Links Úteis

- [utoipa Documentation](https://docs.rs/utoipa/)
- [OpenAPI Specification](https://swagger.io/specification/)
- [Swagger UI](https://swagger.io/tools/swagger-ui/)
- [AUTH_FLOWS.md](./AUTH_FLOWS.md) - Documentação de autenticação completa
