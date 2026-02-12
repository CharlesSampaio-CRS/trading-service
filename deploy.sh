#!/bin/bash

# Deploy rápido: envia código e compila no servidor

set -e

GREEN='\033[0;32m'
NC='\033[0m'

SERVER_IP=$(grep "Public IP:" ec2-info.txt | awk '{print $3}')
KEY_FILE="trading-service-key.pem"

echo "🚀 Deploy Rápido"
echo "================"
echo ""

# Compactar só o código fonte
echo "📦 Compactando código..."
tar -czf /tmp/app-update.tar.gz src/ Cargo.toml Cargo.lock requirements.txt 2>/dev/null

# Enviar
echo "📤 Enviando..."
scp -i "$KEY_FILE" -q /tmp/app-update.tar.gz ubuntu@"$SERVER_IP":~/

# Compilar e reiniciar
echo "🔧 Compilando no servidor..."
ssh -i "$KEY_FILE" ubuntu@"$SERVER_IP" << 'ENDSSH'
    cd ~/trading-service
    tar -xzf ~/app-update.tar.gz
    rm ~/app-update.tar.gz
    
    export PATH="$HOME/.cargo/bin:$PATH"
    cargo build --release 2>&1 | grep -E "Compiling|Finished|error" || true
    
    sudo systemctl restart trading-service
    echo "✅ Serviço reiniciado"
ENDSSH

rm /tmp/app-update.tar.gz

echo ""
echo -e "${GREEN}✅ Deploy concluído!${NC}"
echo "🧪 curl http://$SERVER_IP:3002/api/v1/health"
echo ""
