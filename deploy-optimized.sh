#!/bin/bash

# Script OTIMIZADO para deploy (sem Docker, menor custo)

set -e

# Cores
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Ler IP da instância
if [ ! -f ec2-info.txt ]; then
    echo "❌ Arquivo ec2-info.txt não encontrado!"
    echo "Execute ./deploy-ec2.sh primeiro"
    exit 1
fi

SERVER_IP=$(grep "Public IP:" ec2-info.txt | awk '{print $3}')
KEY_FILE="trading-service-key.pem"

echo "🚀 Deploy Otimizado - Trading Service"
echo "====================================="
echo ""
echo "🌐 Servidor: $SERVER_IP"
echo ""

# 1. Preparar arquivos
echo "📦 Preparando aplicação..."
DEPLOY_DIR=$(mktemp -d)

# Copiar código fonte
cp -r src "$DEPLOY_DIR/"
cp Cargo.toml Cargo.lock "$DEPLOY_DIR/"
cp requirements.txt "$DEPLOY_DIR/" 2>/dev/null || echo "# No requirements" > "$DEPLOY_DIR/requirements.txt"

# Criar .env se não existir
if [ -f .env ]; then
    echo "✓ Copiando .env local"
    cp .env "$DEPLOY_DIR/"
else
    echo "⚠️  Criando .env padrão"
    cat > "$DEPLOY_DIR/.env" <<EOF
RUST_LOG=info
HOST=0.0.0.0
PORT=3002
DATABASE_URL=sqlite:/home/ubuntu/trading-service/data/trading.db
EOF
fi

# Compactar
cd "$DEPLOY_DIR"
tar -czf app.tar.gz *
cd - > /dev/null

# 2. Enviar para servidor
echo "📤 Enviando para servidor..."
scp -i "$KEY_FILE" -o StrictHostKeyChecking=no "$DEPLOY_DIR/app.tar.gz" ubuntu@"$SERVER_IP":~/

# 3. Build e deploy no servidor (zero-downtime)
echo "🔧 Compilando e configurando no servidor..."
echo "⚡ Estratégia: build em background enquanto serviço continua rodando"
echo ""
ssh -i "$KEY_FILE" -o StrictHostKeyChecking=no ubuntu@"$SERVER_IP" << 'ENDSSH'
    set -e
    export PATH="$HOME/.cargo/bin:$PATH"
    BUILD_DIR=~/trading-service-build
    LIVE_DIR=~/trading-service

    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "📦 Extraindo nova versão (diretório temporário)..."
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    # Limpar build dir anterior se existir
    rm -rf "$BUILD_DIR"
    mkdir -p "$BUILD_DIR"
    mkdir -p "$LIVE_DIR/data"

    cd "$BUILD_DIR"
    tar -xzf ~/app.tar.gz
    rm ~/app.tar.gz

    # Reutilizar cache de compilação do diretório de produção (acelera builds incrementais)
    if [ -d "$LIVE_DIR/target" ]; then
        echo "♻️  Reaproveitando cache de compilação anterior..."
        ln -s "$LIVE_DIR/target" "$BUILD_DIR/target"
    fi
    echo "✓ Fonte extraída em $BUILD_DIR"
    echo ""

    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "💾 Configurando SWAP (para compilação)..."
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    if [ ! -f /swapfile ]; then
        sudo fallocate -l 4G /swapfile
        sudo chmod 600 /swapfile
        sudo mkswap /swapfile
        sudo swapon /swapfile
        echo "✓ SWAP de 4GB ativado"
    else
        sudo swapon /swapfile 2>/dev/null || true
        echo "✓ SWAP já configurado"
    fi
    echo "   SWAP disponível: $(free -h | awk '/^Swap:/ {print $2}')"
    echo ""

    # Matar processos de compilação travados (sem tocar no serviço em produção)
    if pgrep -f "cargo build|rustc" > /dev/null; then
        echo "⚠️  Processos de compilação travados encontrados, limpando..."
        pkill -9 -f "cargo build|rustc" || true
        sleep 1
        echo "✓ Processos limpos"
    fi

    echo "📊 Recursos disponíveis para compilação:"
    echo "   Memória livre: $(free -h | awk '/^Mem:/ {print $7}')"
    echo "   SWAP em uso:   $(free -h | awk '/^Swap:/ {print $3}')"
    echo "   Disco livre:   $(df -h ~ | awk 'NR==2 {print $4}')"
    echo ""

    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "🦀 Compilando nova versão..."
    echo "   ⚡ Serviço em PRODUÇÃO continua rodando normalmente!"
    echo "   ⏱️  Isso pode demorar alguns minutos"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""

    BUILD_OK=false
    if timeout 1800 cargo build --release -j 1 2>&1 | grep -E "(Compiling|Finished|error\[|^error)" | while read line; do
        echo "   $line"
        if [[ "$line" == *"Compiling"* ]] && (( RANDOM % 10 == 0 )); then
            echo "      [Mem: $(free -h | awk '/^Mem:/ {print $3}') / SWAP: $(free -h | awk '/^Swap:/ {print $3}')]"
        fi
    done; then
        BUILD_OK=true
        echo ""
        echo "✓ Compilação concluída com sucesso!"
    else
        EXIT_CODE=$?
        echo ""
        if [ $EXIT_CODE -eq 124 ]; then
            echo "❌ Timeout na compilação (>30min). Instância sem recursos?"
        else
            echo "❌ Erro na compilação! Serviço em produção NÃO foi afetado."
            cargo build --release -j 1 2>&1 | tail -30
        fi
        rm -rf "$BUILD_DIR"
        exit 1
    fi
    echo ""

    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "🔄 Swap atômico do binário (downtime ~2s)..."
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

    # Copiar novo binário com nome temporário (não interrompe nada ainda)
    NEW_BIN="$BUILD_DIR/target/release/trading-service"
    LIVE_BIN="$LIVE_DIR/target/release/trading-service"
    mkdir -p "$LIVE_DIR/target/release"
    cp "$NEW_BIN" "${LIVE_BIN}.new"

    # Sincronizar código fonte novo no live dir
    rsync -a --exclude='target' "$BUILD_DIR/" "$LIVE_DIR/"

    # Configurar systemd (idempotente)
    sudo tee /etc/systemd/system/trading-service.service > /dev/null <<EOF
[Unit]
Description=Trading Service
After=network.target

[Service]
Type=simple
User=ubuntu
WorkingDirectory=/home/ubuntu/trading-service
Environment="RUST_LOG=info"
EnvironmentFile=/home/ubuntu/trading-service/.env
ExecStart=/home/ubuntu/trading-service/target/release/trading-service
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF
    sudo systemctl daemon-reload
    sudo systemctl enable trading-service

    # === JANELA DE DOWNTIME COMEÇA AQUI (~2-3 segundos) ===
    echo "⏱️  Parando serviço antigo..."
    sudo systemctl stop trading-service 2>/dev/null || true

    # Swap atômico: move o novo binário para o lugar definitivo
    mv "${LIVE_BIN}.new" "$LIVE_BIN"

    echo "🚀 Iniciando nova versão..."
    sudo systemctl start trading-service
    # === JANELA DE DOWNTIME TERMINA AQUI ===

    echo ""
    echo "⏳ Aguardando inicialização..."
    for i in {1..10}; do
        sleep 1
        if sudo systemctl is-active --quiet trading-service; then
            echo "✓ Serviço ativo após ${i}s"
            break
        fi
        echo -n "   ${i}s... "
    done
    echo ""

    # Limpar diretório de build temporário
    # Desanexar symlink do target antes de remover para não apagar o cache
    if [ -L "$BUILD_DIR/target" ]; then
        rm "$BUILD_DIR/target"
    fi
    rm -rf "$BUILD_DIR"
    echo "✓ Diretório temporário removido"
    echo ""

    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "🔍 Status do serviço:"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    sudo systemctl status trading-service --no-pager -l || true
    echo ""

    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "📋 Últimas 15 linhas do log:"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    sudo journalctl -u trading-service -n 15 --no-pager || true

    echo ""
    echo "✅ Deploy zero-downtime concluído!"
ENDSSH

# Limpar
rm -rf "$DEPLOY_DIR"

echo ""
echo -e "${GREEN}✅ Aplicação deployada com sucesso!${NC}"
echo ""
echo -e "${BLUE}📋 Informações:${NC}"
echo "   URL: http://$SERVER_IP:3002"
echo "   Health: http://$SERVER_IP:3002/api/v1/health"
echo ""
echo -e "${YELLOW}🔧 Comandos úteis:${NC}"
echo "   Ver logs: ssh -i $KEY_FILE ubuntu@$SERVER_IP 'sudo journalctl -u trading-service -f'"
echo "   Status: ssh -i $KEY_FILE ubuntu@$SERVER_IP 'sudo systemctl status trading-service'"
echo "   Restart: ssh -i $KEY_FILE ubuntu@$SERVER_IP 'sudo systemctl restart trading-service'"
echo "   Parar: ssh -i $KEY_FILE ubuntu@$SERVER_IP 'sudo systemctl stop trading-service'"
echo ""
echo "🧪 Testar API:"
echo "   curl http://$SERVER_IP:3002/api/v1/health"
echo ""
echo -e "${GREEN}💰 Vantagens desta abordagem:${NC}"
echo "   • Sem Docker = economiza ~500MB de RAM"
echo "   • Binário nativo = melhor performance"
echo "   • Systemd = restart automático em caso de crash"
echo "   • Menor uso de recursos = menor custo"
echo ""
