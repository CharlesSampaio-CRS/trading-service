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

# 3. Build e deploy no servidor
echo "🔧 Compilando e configurando no servidor..."
echo ""
ssh -i "$KEY_FILE" -o StrictHostKeyChecking=no ubuntu@"$SERVER_IP" << 'ENDSSH'
    set -e
    
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "📦 Extraindo aplicação..."
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    mkdir -p ~/trading-service/data
    cd ~/trading-service
    tar -xzf ~/app.tar.gz
    rm ~/app.tar.gz
    echo "✓ Aplicação extraída"
    echo ""
    
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "💾 Configurando SWAP temporário (para compilação)..."
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    if [ ! -f /swapfile ]; then
        echo "Criando SWAP de 4GB (necessário para compilação Rust)..."
        sudo fallocate -l 4G /swapfile
        sudo chmod 600 /swapfile
        sudo mkswap /swapfile
        sudo swapon /swapfile
        echo "✓ SWAP de 4GB ativado"
    else
        # Garantir que SWAP está ativo
        sudo swapon /swapfile 2>/dev/null || true
        echo "✓ SWAP já configurado e ativo"
    fi
    
    # Mostrar uso de swap
    echo "   SWAP disponível: $(free -h | awk '/^Swap:/ {print $2}')"
    echo ""
    
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "🛑 Parando serviço anterior (se existir)..."
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    if sudo systemctl is-active --quiet trading-service; then
        echo "⏹️  Serviço está rodando, parando..."
        sudo systemctl stop trading-service
        echo "✓ Serviço parado"
        sleep 2
    else
        echo "✓ Nenhum serviço anterior rodando"
    fi
    
    # Matar processos cargo/rustc que possam estar travados
    if pgrep -f "cargo|rustc" > /dev/null; then
        echo "⚠️  Encontrados processos de compilação travados, matando..."
        pkill -9 -f "cargo|rustc" || true
        sleep 1
        echo "✓ Processos limpos"
    fi
    
    # Verificar recursos disponíveis
    echo "📊 Recursos do sistema:"
    echo "   Memória livre: $(free -h | awk '/^Mem:/ {print $7}')"
    echo "   SWAP em uso: $(free -h | awk '/^Swap:/ {print $3}')"
    echo "   Espaço em disco: $(df -h ~ | awk 'NR==2 {print $4}')"
    echo ""
    
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "🧹 Limpando cache de compilação anterior..."
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    cargo clean || true
    echo "✓ Cache limpo"
    echo ""
    
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "🦀 Compilando aplicação Rust..."
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "⏱️  Isso pode demorar 10-15 minutos na primeira vez"
    echo "🔧 Compilando com -j 1 (sequencial) para economizar memória"
    echo "📊 Progresso da compilação:"
    echo ""
    export PATH="$HOME/.cargo/bin:$PATH"
    
    # Compilar sequencialmente para não travar por falta de memória
    # -j 1 força compilação de 1 crate por vez
    if timeout 1800 cargo build --release -j 1 2>&1 | grep -E "(Compiling|Finished|error:)" | while read line; do 
        echo "   $line"
        # Mostrar uso de memória a cada 10 pacotes
        if [[ "$line" == *"Compiling"* ]] && (( RANDOM % 10 == 0 )); then
            echo "      [Mem: $(free -h | awk '/^Mem:/ {print $3}') / SWAP: $(free -h | awk '/^Swap:/ {print $3}')]"
        fi
    done; then
        echo ""
        echo "✓ Compilação concluída com sucesso!"
    else
        EXIT_CODE=$?
        echo ""
        if [ $EXIT_CODE -eq 124 ]; then
            echo "❌ Timeout: Compilação demorou mais de 30 minutos!"
            echo "   A instância pode estar sem recursos."
            echo "   Considere usar uma instância maior temporariamente."
        else
            echo "❌ Erro na compilação!"
            echo "Mostrando últimas linhas do erro..."
            cargo build --release -j 1 2>&1 | tail -50
        fi
        exit 1
    fi
    echo ""
    
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "📝 Criando serviço systemd..."
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
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
RestartSec=10

[Install]
WantedBy=multi-user.target
EOF
    echo "✓ Serviço systemd criado"
    echo ""

    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "🚀 Iniciando serviço..."
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    sudo systemctl daemon-reload
    echo "✓ Systemd recarregado"
    
    sudo systemctl enable trading-service
    echo "✓ Serviço habilitado para iniciar no boot"
    
    sudo systemctl restart trading-service
    echo "✓ Serviço reiniciado"
    echo ""
    
    echo "⏳ Aguardando inicialização (5s)..."
    for i in {5..1}; do
        echo -n "   $i... "
        sleep 1
    done
    echo ""
    echo ""
    
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "🔍 Status do serviço:"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    sudo systemctl status trading-service --no-pager -l || true
    echo ""
    
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "📋 Últimas 10 linhas do log:"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    sudo journalctl -u trading-service -n 10 --no-━━━━━━━━━━━━━━━━━━━━━
    echo "🔍 Status do serviço:"
    sudo systemctl status trading-service --no-pager || true
    
    echo ""
    echo "✅ Deploy concluído!"
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
