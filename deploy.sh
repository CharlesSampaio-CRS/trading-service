#!/bin/bash

# Script de deploy automatizado para AWS ECR + ECS
# Uso: ./deploy.sh [tag]

set -e

# Configurações (ajuste conforme necessário)
AWS_REGION="${AWS_REGION:-us-east-1}"
AWS_ACCOUNT_ID="${AWS_ACCOUNT_ID:-$(aws sts get-caller-identity --query Account --output text)}"
ECR_REPO="trading-service"
IMAGE_TAG="${1:-latest}"
CLUSTER_NAME="trading-cluster"
SERVICE_NAME="trading-service"

echo "================================================"
echo "Deploy Trading Service to AWS"
echo "================================================"
echo "Region: $AWS_REGION"
echo "Account ID: $AWS_ACCOUNT_ID"
echo "Image Tag: $IMAGE_TAG"
echo "================================================"

# 1. Build da imagem Docker
echo ""
echo "📦 Building Docker image..."
docker build -t $ECR_REPO:$IMAGE_TAG .

# 2. Login no ECR
echo ""
echo "🔐 Logging into ECR..."
aws ecr get-login-password --region $AWS_REGION | \
    docker login --username AWS --password-stdin \
    $AWS_ACCOUNT_ID.dkr.ecr.$AWS_REGION.amazonaws.com

# 3. Tag da imagem
echo ""
echo "🏷️  Tagging image..."
ECR_IMAGE="$AWS_ACCOUNT_ID.dkr.ecr.$AWS_REGION.amazonaws.com/$ECR_REPO:$IMAGE_TAG"
docker tag $ECR_REPO:$IMAGE_TAG $ECR_IMAGE

# 4. Push para ECR
echo ""
echo "⬆️  Pushing to ECR..."
docker push $ECR_IMAGE

# 5. Atualizar serviço ECS (se existir)
echo ""
echo "🔄 Updating ECS service..."
if aws ecs describe-services --cluster $CLUSTER_NAME --services $SERVICE_NAME --region $AWS_REGION 2>/dev/null | grep -q "ACTIVE"; then
    aws ecs update-service \
        --cluster $CLUSTER_NAME \
        --service $SERVICE_NAME \
        --force-new-deployment \
        --region $AWS_REGION
    
    echo ""
    echo "✅ Service updated! Waiting for deployment..."
    aws ecs wait services-stable \
        --cluster $CLUSTER_NAME \
        --services $SERVICE_NAME \
        --region $AWS_REGION
else
    echo "⚠️  ECS service not found. Please create it manually or using the AWS Console."
fi

echo ""
echo "================================================"
echo "✅ Deploy completed successfully!"
echo "================================================"
echo "Image: $ECR_IMAGE"
echo ""
echo "Next steps:"
echo "  - Check logs: aws logs tail /ecs/trading-service --follow"
echo "  - Monitor service: aws ecs describe-services --cluster $CLUSTER_NAME --services $SERVICE_NAME"
echo ""
