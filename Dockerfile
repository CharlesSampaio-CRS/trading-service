# Multi-stage build para otimizar o tamanho final da imagem

# Stage 1: Build do Rust
FROM rust:latest AS rust-builder

# Instalar dependências necessárias para compilação
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    python3 \
    python3-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copiar todo o código-fonte
COPY . .

# Variável de ambiente para compatibilidade PyO3
ENV PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1

# Build da aplicação
RUN cargo build --release

# Stage 2: Runtime com Python
FROM python:3.11-slim

# Instalar dependências do sistema (incluindo curl para healthcheck)
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copiar requirements e instalar dependências Python
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

# Copiar binário compilado do Rust
COPY --from=rust-builder /app/target/release/trading-service /app/trading-service

# Tornar o binário executável
RUN chmod +x /app/trading-service

# Criar usuário não-root para segurança
RUN useradd -m -u 1000 appuser && chown -R appuser:appuser /app
USER appuser

# Expor porta da aplicação (ajustar conforme seu .env)
EXPOSE 3002

# Variáveis de ambiente padrão (sobrescrever no deploy)
ENV RUST_LOG=info
ENV HOST=0.0.0.0
ENV PORT=3002

# Healthcheck
HEALTHCHECK --interval=30s --timeout=5s --start-period=40s --retries=3 \
    CMD curl -f http://localhost:3002/health || exit 1

# Comando para iniciar a aplicação
CMD ["./trading-service"]
