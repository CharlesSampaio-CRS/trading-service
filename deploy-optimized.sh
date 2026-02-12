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
ssh -i "$KEY_FILE" -o StrictHostKeyChecking=no ubuntu@"$SERVER_IP" << 'ENDSSH'
    set -e
    
    echo "📦 Extraindo aplicação..."
    mkdir -p ~/trading-service/data
    cd ~/trading-service
    tar -xzf ~/app.tar.gz
    rm ~/app.tar.gz
    
    echo "💾 Configurando SWAP temporário (para compilação)..."
    if [ ! -f /swapfile ]; then
        sudo fallocate -l 2G /swapfile
        sudo chmod 600 /swapfile
        sudo mkswap /swapfile
        sudo swapon /swapfile
        echo "✓ SWAP de 2GB ativado"
    fi
    
    echo "🦀 Compilando aplicação (pode demorar 5-10 min)..."
    export PATH="$HOME/.cargo/bin:$PATH"
    cargo build --release
    
    echo "📝 Criando serviço systemd..."
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

    echo "🚀 Iniciando serviço..."
    sudo systemctl daemon-reload
    sudo systemctl enable trading-service
    sudo systemctl restart trading-service
    
    echo "⏳ Aguardando inicialização..."
    sleep 5
    
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
