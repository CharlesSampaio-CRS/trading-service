# 🔐 Fluxos de Autenticação

## Registro de Usuários

O sistema suporta três métodos de registro:

### 1. Registro Local (Email + Senha)

```json
POST /api/v1/auth/register
{
  "email": "user@example.com",
  "password": "securePassword123",
  "name": "John Doe",
  "provider": "local"
}
```

**Validação:**
- ✅ `email` - Obrigatório
- ✅ `password` - Obrigatório para provider "local"
- ℹ️ `name` - Opcional
- ℹ️ `provider` - Padrão: "local"

**Resposta:**
```json
{
  "success": true,
  "token": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
  "refresh_token": "...",
  "user": {
    "id": "507f1f77bcf86cd799439011",
    "email": "user@example.com",
    "name": "John Doe",
    "picture": null,
    "roles": ["user"]
  }
}
```

---

### 2. Registro com Google

```json
POST /api/v1/auth/register
{
  "email": "user@gmail.com",
  "name": "John Doe",
  "google_id": "108123456789012345678",
  "picture": "https://lh3.googleusercontent.com/...",
  "provider": "google"
}
```

**Validação:**
- ✅ `email` - Obrigatório
- ✅ `google_id` - Obrigatório para provider "google"
- ℹ️ `name` - Opcional (recomendado)
- ℹ️ `picture` - Opcional (avatar do Google)
- ⚠️ `password` - **NÃO** necessário

**Comportamento:**
- Cria usuário normalmente
- Sem senha armazenada
- Verificação de duplicatas por `email` ou `google_id`

---

### 3. Registro com Apple

```json
POST /api/v1/auth/register
{
  "email": "user@privaterelay.appleid.com",
  "name": "John Doe",
  "apple_id": "001234.a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6.1234",
  "picture": null,
  "provider": "apple"
}
```

**Validação:**
- ✅ `email` - Obrigatório
- ✅ `apple_id` - Obrigatório para provider "apple"
- ℹ️ `name` - Opcional (Apple pode ocultar)
- ℹ️ `picture` - Opcional (Apple não fornece)
- ⚠️ `password` - **NÃO** necessário

**Nota Apple:**
- Email pode ser relay privado do Apple
- Nome pode ser anônimo se usuário escolher
- Sem foto de perfil disponível

---

## Modelo de Usuário no Banco

```javascript
{
  "_id": ObjectId("..."),
  "user_id": "507f1f77bcf86cd799439011",  // Primary key
  "email": "user@example.com",
  "password": "hashed_password",           // null para OAuth
  "name": "John Doe",
  "picture": "https://...",                // null se não fornecido
  "google_id": "108123...",                // null se não for Google
  "apple_id": "001234...",                 // null se não for Apple
  "provider": "local",                     // "local" | "google" | "apple"
  "roles": ["user"],
  "is_active": true,
  "created_at": ISODate("2026-02-11T..."),
  "updated_at": ISODate("2026-02-11T..."),
  "last_login": ISODate("2026-02-11T...")
}
```

---

## Verificação de Duplicatas

O sistema verifica usuários existentes por:

**Registro Local:**
- Email apenas

**Registro Google:**
- Email OU `google_id`

**Registro Apple:**
- Email OU `apple_id`

---

## Login

### Login Local
```json
POST /api/v1/auth/login
{
  "email": "user@example.com",
  "password": "securePassword123"
}
```

### Login OAuth
Para Google/Apple, use o fluxo de registro novamente. Se o usuário já existe, o endpoint de registro retornará erro "User already exists". O frontend deve então:

1. Tentar registrar
2. Se "User already exists", fazer login OAuth (ou implementar endpoint separado de login OAuth)

---

## Segurança

### Senhas
- Hashing: `bcrypt` com cost 12
- Apenas para provider "local"
- OAuth users: `password = null`

### JWT Token
- Algoritmo: HS256
- Expiração: 24 horas
- Claims: `sub` (user_id), `email`, `name`, `roles`, `is_active`

### Refresh Token
- Expiração: 30 dias
- Usado para renovar access token sem re-login

---

## Exemplos de Frontend

### React Native / Expo

```typescript
// Registro Local
const registerLocal = async (email: string, password: string) => {
  const response = await fetch('http://localhost:3002/api/v1/auth/register', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      email,
      password,
      provider: 'local'
    })
  });
  return response.json();
};

// Registro Google
const registerGoogle = async (googleUser: any) => {
  const response = await fetch('http://localhost:3002/api/v1/auth/register', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      email: googleUser.email,
      name: googleUser.name,
      google_id: googleUser.id,
      picture: googleUser.picture,
      provider: 'google'
    })
  });
  return response.json();
};

// Registro Apple
const registerApple = async (appleUser: any) => {
  const response = await fetch('http://localhost:3002/api/v1/auth/register', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      email: appleUser.email,
      name: appleUser.fullName?.givenName,
      apple_id: appleUser.user,
      provider: 'apple'
    })
  });
  return response.json();
};
```

---

## Erros Comuns

| Erro | Causa | Solução |
|------|-------|---------|
| "Email is required" | Campo email não enviado | Sempre enviar email |
| "Password is required for local registration" | Provider "local" sem senha | Enviar password ou usar OAuth |
| "Google ID is required for Google registration" | Provider "google" sem google_id | Enviar google_id do OAuth |
| "Apple ID is required for Apple registration" | Provider "apple" sem apple_id | Enviar apple_id do OAuth |
| "User already exists" | Email ou OAuth ID já cadastrado | Fazer login ao invés de registro |
| "Invalid provider" | Provider não suportado | Usar: "local", "google" ou "apple" |

---

## Roadmap

- [ ] Endpoint separado de login OAuth
- [ ] Vincular múltiplos providers ao mesmo usuário
- [ ] Email verification
- [ ] Password reset
- [ ] 2FA (Two-Factor Authentication)
