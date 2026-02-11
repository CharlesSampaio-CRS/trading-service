# Deploy do Trading Service na AWS

## Opções de Deploy

### Opção 1: AWS ECS (Elastic Container Service) com Fargate

#### 1.1 Configurar AWS CLI
```bash
aws configure
```

#### 1.2 Criar repositório no ECR
```bash
# Criar repositório
aws ecr create-repository --repository-name trading-service --region us-east-1

# Obter URI do repositório
aws ecr describe-repositories --repository-names trading-service --region us-east-1
```

#### 1.3 Build e Push da imagem
```bash
# Login no ECR
aws ecr get-login-password --region us-east-1 | docker login --username AWS --password-stdin <account-id>.dkr.ecr.us-east-1.amazonaws.com

# Build da imagem
docker build -t trading-service .

# Tag da imagem
docker tag trading-service:latest <account-id>.dkr.ecr.us-east-1.amazonaws.com/trading-service:latest

# Push para ECR
docker push <account-id>.dkr.ecr.us-east-1.amazonaws.com/trading-service:latest
```

#### 1.4 Criar Task Definition (task-definition.json)
```json
{
  "family": "trading-service",
  "networkMode": "awsvpc",
  "requiresCompatibilities": ["FARGATE"],
  "cpu": "512",
  "memory": "1024",
  "containerDefinitions": [
    {
      "name": "trading-service",
      "image": "<account-id>.dkr.ecr.us-east-1.amazonaws.com/trading-service:latest",
      "portMappings": [
        {
          "containerPort": 8080,
          "protocol": "tcp"
        }
      ],
      "environment": [
        {
          "name": "RUST_LOG",
          "value": "info"
        },
        {
          "name": "HOST",
          "value": "0.0.0.0"
        },
        {
          "name": "PORT",
          "value": "8080"
        }
      ],
      "secrets": [
        {
          "name": "MONGODB_URI",
          "valueFrom": "arn:aws:secretsmanager:us-east-1:<account-id>:secret:trading-service/mongodb-uri"
        },
        {
          "name": "JWT_SECRET",
          "valueFrom": "arn:aws:secretsmanager:us-east-1:<account-id>:secret:trading-service/jwt-secret"
        }
      ],
      "logConfiguration": {
        "logDriver": "awslogs",
        "options": {
          "awslogs-group": "/ecs/trading-service",
          "awslogs-region": "us-east-1",
          "awslogs-stream-prefix": "ecs"
        }
      },
      "healthCheck": {
        "command": ["CMD-SHELL", "curl -f http://localhost:8080/health || exit 1"],
        "interval": 30,
        "timeout": 5,
        "retries": 3,
        "startPeriod": 60
      }
    }
  ],
  "executionRoleArn": "arn:aws:iam::<account-id>:role/ecsTaskExecutionRole",
  "taskRoleArn": "arn:aws:iam::<account-id>:role/ecsTaskRole"
}
```

#### 1.5 Registrar Task Definition
```bash
aws ecs register-task-definition --cli-input-json file://task-definition.json
```

#### 1.6 Criar cluster e service
```bash
# Criar cluster
aws ecs create-cluster --cluster-name trading-cluster

# Criar service
aws ecs create-service \
  --cluster trading-cluster \
  --service-name trading-service \
  --task-definition trading-service \
  --desired-count 1 \
  --launch-type FARGATE \
  --network-configuration "awsvpcConfiguration={subnets=[subnet-xxxxx],securityGroups=[sg-xxxxx],assignPublicIp=ENABLED}"
```

---

### Opção 2: AWS EC2 com Docker

#### 2.1 Lançar instância EC2
- Escolha uma AMI (Amazon Linux 2023 ou Ubuntu)
- Tipo de instância: t3.small ou maior
- Configure security group: porta 8080, 22 (SSH)

#### 2.2 Conectar via SSH e instalar Docker
```bash
# Amazon Linux 2023
sudo yum update -y
sudo yum install -y docker
sudo systemctl start docker
sudo systemctl enable docker
sudo usermod -a -G docker ec2-user

# Ubuntu
sudo apt-get update
sudo apt-get install -y docker.io docker-compose
sudo systemctl start docker
sudo systemctl enable docker
sudo usermod -a -G docker ubuntu
```

#### 2.3 Transferir código e deploy
```bash
# Clonar repositório ou usar SCP
git clone <seu-repo>
cd trading-service

# Criar arquivo .env com suas variáveis
cp .env.example .env
nano .env

# Build e run
docker-compose up -d
```

---

### Opção 3: AWS App Runner (Mais simples)

#### 3.1 Push para ECR (mesmo processo da opção 1)

#### 3.2 Criar serviço via Console AWS
1. Acesse AWS App Runner no console
2. Clique em "Create service"
3. Escolha "Container registry" → "Amazon ECR"
4. Selecione sua imagem
5. Configure:
   - Port: 8080
   - Environment variables
   - Health check: /health
6. Deploy!

---

## Scripts Auxiliares

### Script de Deploy Automatizado (deploy.sh)
```bash
#!/bin/bash

# Configurações
AWS_REGION="us-east-1"
AWS_ACCOUNT_ID="<seu-account-id>"
ECR_REPO="trading-service"
IMAGE_TAG="latest"

# Build
echo "Building Docker image..."
docker build -t $ECR_REPO:$IMAGE_TAG .

# Login ECR
echo "Logging into ECR..."
aws ecr get-login-password --region $AWS_REGION | docker login --username AWS --password-stdin $AWS_ACCOUNT_ID.dkr.ecr.$AWS_REGION.amazonaws.com

# Tag
echo "Tagging image..."
docker tag $ECR_REPO:$IMAGE_TAG $AWS_ACCOUNT_ID.dkr.ecr.$AWS_REGION.amazonaws.com/$ECR_REPO:$IMAGE_TAG

# Push
echo "Pushing to ECR..."
docker push $AWS_ACCOUNT_ID.dkr.ecr.$AWS_REGION.amazonaws.com/$ECR_REPO:$IMAGE_TAG

echo "Deploy completed!"
```

---

## Variáveis de Ambiente Necessárias

Crie no AWS Secrets Manager:
- `MONGODB_URI`: String de conexão do MongoDB
- `JWT_SECRET`: Secret para tokens JWT
- `DATABASE_NAME`: Nome do banco de dados

---

## Monitoramento

### CloudWatch Logs
```bash
# Criar log group
aws logs create-log-group --log-group-name /ecs/trading-service
```

### Alarmes
- CPU > 80%
- Memory > 80%
- Health check failures

---

## Custos Estimados (us-east-1)

### ECS Fargate (0.5 vCPU, 1GB RAM)
- ~$14/mês (running 24/7)

### EC2 t3.small
- ~$15/mês + storage

### App Runner
- ~$20-30/mês (varia com uso)

---

## Próximos Passos

1. Configure um Load Balancer (ALB) para HTTPS
2. Configure Auto Scaling
3. Implemente CI/CD com GitHub Actions ou AWS CodePipeline
4. Configure backup do MongoDB (se usando DocumentDB)
5. Implemente logs estruturados e métricas
