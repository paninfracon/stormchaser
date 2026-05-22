#!/bin/bash
set -e

# Stormchaser Unified Setup Script
# Orchestrates Local Development Environment (Docker or Hybrid with MicroK8s)

GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m' # No Color

MODE="docker" # Default mode to docker now
CLEANUP=false

usage() {
    echo "Usage: $0 [options]"
    echo "Options:"
    echo "  --mode [docker|hybrid|microk8s]  Select runner environment (default: docker)"
    echo "  --cleanup            Remove existing environment before starting"
    echo "  --help               Show this help message"
    exit 1
}

while [[ $# -gt 0 ]]; do
    case $1 in
        --mode)
            MODE="$2"
            shift 2
            ;;
        --cleanup)
            CLEANUP=true
            shift
            ;;
        --help)
            usage
            ;;
        *)
            usage
            ;;
    esac
done

if [[ "$MODE" != "hybrid" && "$MODE" != "docker" && "$MODE" != "microk8s" ]]; then
    echo -e "${RED}Error: Invalid mode '$MODE'. Use 'docker', 'hybrid', or 'microk8s'.${NC}"
    exit 1
fi

REPO_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." &> /dev/null && pwd)
CERT_DIR="$REPO_ROOT/tests/certs"
export STORMCHASER_CERT_DIR="$CERT_DIR"
export SQLX_OFFLINE=true

# Helper to run commands with privileges using pkexec (desktop-friendly)
run_privileged() {
    if [ "$EUID" -eq 0 ]; then
        "$@"
    elif command -v pkexec &> /dev/null; then
        pkexec "$@"
    else
        echo -e "${BLUE}>>> pkexec not found, falling back to sudo...${NC}"
        sudo "$@"
    fi
}

cleanup_k8s() {
    if command -v helm &> /dev/null; then
        echo -e "${BLUE}>>> Uninstalling K8s components...${NC}"
        helm uninstall alloy stormchaser-runner stormchaser-dogfood 2>/dev/null || true
    fi
}

cleanup_docker() {
    echo -e "${BLUE}>>> Stopping Docker services...${NC}"
    docker compose -p stormchaser-docker --profile "*" down -v || true
    docker compose -p stormchaser-hybrid --profile "*" down -v || true
    docker compose -p stormchaser-k8s --profile "*" down -v || true
    docker compose --profile "*" down -v || true
}

if [ "$CLEANUP" = true ]; then
    echo -e "${BLUE}>>> Performing full cleanup...${NC}"
    cleanup_docker
    cleanup_k8s
fi



# 1. Generate TLS Certificates
"$REPO_ROOT/scripts/generate-certs.sh"

# 2. Setup MicroK8s ONLY if in k8s mode
if [[ "$MODE" == "hybrid" || "$MODE" == "microk8s" ]]; then
    echo -e "${BLUE}>>> Verifying MicroK8s...${NC}"
    if ! command -v microk8s &> /dev/null; then
        echo -e "${RED}Error: microk8s is not installed. Required for --mode k8s.${NC}"
        exit 1
    fi

    if ! microk8s status --wait-ready > /dev/null 2>&1; then
        echo -e "${BLUE}>>> Starting MicroK8s...${NC}"
        run_privileged microk8s start
        microk8s status --wait-ready
    fi

    # Enable addons
    for addon in dns rbac storage ingress; do
        if ! microk8s status | sed -n '/enabled:/,/disabled:/p' | grep -q "\b$addon\b"; then
            echo -e "${BLUE}>>> Enabling MicroK8s addon $addon...${NC}"
            microk8s enable "$addon" || run_privileged microk8s enable "$addon" || true
        fi
    done
fi

# Set isolated ports based on the mode
if [ "$MODE" == "hybrid" ]; then
    export PORT_API=3001
    export PORT_DB=5433
    export PORT_NATS=4223
    export PORT_NATS_MGT=8223
    export PORT_DEX=5557
    export PORT_LOKI=3101
    export PORT_S3=9002
    export PORT_S3_CONS=9003
    export PORT_REG=32001
    export PORT_OPA=8182
    export PORT_WEB=3004
else
    export PORT_API=3000
    export PORT_DB=5432
    export PORT_NATS=4222
    export PORT_NATS_MGT=8222
    export PORT_DEX=5556
    export PORT_LOKI=3100
    export PORT_S3=9000
    export PORT_S3_CONS=9001
    export PORT_REG=32000
    export PORT_OPA=8181
    export PORT_WEB=3003
fi

# 2.25 Generate random dev passwords in .env if missing
ENV_FILE="$REPO_ROOT/.env"
if [ "$CLEANUP" = true ] || [ ! -f "$ENV_FILE" ]; then
    echo -e "${BLUE}>>> Generating random dev passwords in .env...${NC}"
    if ! command -v python3 >/dev/null 2>&1; then
        echo -e "${RED}Error: python3 is required to generate passwords.${NC}" >&2
        exit 1
    fi
    STORMCHASER_DEV_PASSWORD=$(python3 -c 'import secrets; print(secrets.token_urlsafe(16))')
    STORMCHASER_MINIO_PASSWORD=$(python3 -c 'import secrets; print(secrets.token_urlsafe(16))')
    STORMCHASER_CLI_SECRET=$(python3 -c 'import secrets; print(secrets.token_urlsafe(32))')
    if [ -z "$STORMCHASER_DEV_PASSWORD" ] || [ -z "$STORMCHASER_MINIO_PASSWORD" ] || [ -z "$STORMCHASER_CLI_SECRET" ]; then
        echo -e "${RED}Error: Failed to generate random passwords.${NC}" >&2
        exit 1
    fi
    # Upsert keys so that unrelated entries in .env are preserved
    touch "$ENV_FILE"
    for key_val in "STORMCHASER_DEV_PASSWORD=$STORMCHASER_DEV_PASSWORD" "STORMCHASER_MINIO_PASSWORD=$STORMCHASER_MINIO_PASSWORD" "STORMCHASER_CLI_SECRET=$STORMCHASER_CLI_SECRET"; do
        key="${key_val%%=*}"
        val="${key_val#*=}"
        if grep -q "^${key}=" "$ENV_FILE" 2>/dev/null; then
            tmp=$(mktemp)
            sed "s|^${key}=.*|${key}=${val}|" "$ENV_FILE" > "$tmp" && mv "$tmp" "$ENV_FILE"
        else
            echo "${key}=${val}" >> "$ENV_FILE"
        fi
    done
fi

# Load .env into the current shell so subsequent steps can use the passwords
# (handles the case where .env already existed and STORMCHASER_DEV_PASSWORD was not generated above)
if [ -f "$ENV_FILE" ]; then
    set -a
    # shellcheck source=/dev/null
    source "$ENV_FILE"
    set +a
fi

# 2.5 Generate Dex config if missing, cleanup requested, or template changed
DEX_TEMPLATE_PATH="$REPO_ROOT/deploy/dex/config.yaml"
DEX_GENERATED_PATH="$REPO_ROOT/deploy/dex/config.generated.yaml"
REGENERATE_DEX_CONFIG=false

DEX_CREDENTIALS_PATH="$REPO_ROOT/deploy/dex/credentials.generated"

if [ "$CLEANUP" = true ] || [ ! -f "$DEX_GENERATED_PATH" ] || [ ! -f "$DEX_CREDENTIALS_PATH" ]; then
    REGENERATE_DEX_CONFIG=true
elif [ -f "$DEX_TEMPLATE_PATH" ] && [ "$DEX_TEMPLATE_PATH" -nt "$DEX_GENERATED_PATH" ]; then
    REGENERATE_DEX_CONFIG=true
fi

if [ "$REGENERATE_DEX_CONFIG" = true ]; then
    echo -e "${BLUE}>>> Generating random passwords for Dex personas...${NC}"
    export REPO_ROOT
    export STORMCHASER_DEV_PASSWORD
    export STORMCHASER_CLI_SECRET
    if ! command -v python3 >/dev/null 2>&1; then
        echo -e "${RED}Error: python3 is required to generate the Dex config but was not found in PATH.${NC}" >&2
        echo -e "${RED}Please install Python 3 and re-run this script.${NC}" >&2
        exit 1
    fi
    python3 "$REPO_ROOT/scripts/generate_dex_config.py"
fi

# 3. Start Docker Services
if [[ "$MODE" == "docker" || "$MODE" == "hybrid" ]]; then
    echo -e "${BLUE}>>> Starting Stormchaser backend in Docker ($MODE mode)...${NC}"
    if [ "$MODE" == "hybrid" ]; then
        COMPOSE_PROFILES="k8s"
    else
        COMPOSE_PROFILES="$MODE"
    fi

    # Build and start
    docker compose -p "stormchaser-${MODE}" --profile "$COMPOSE_PROFILES" up -d --build
    docker compose -p "stormchaser-${MODE}" build stormchaser-agent # Ensure agent is built
    if [ "$MODE" == "hybrid" ]; then
        docker compose -p "stormchaser-${MODE}" build k8s-runner # Ensure k8s runner is built
    fi

    # Wait for services
    wait_for() {
        local url=$1
        local name=$2
        local timeout=60
        local count=0
        until curl -s "$url" > /dev/null; do
            echo "Waiting for $name..."
            sleep 2
            count=$((count + 2))
            if [ $count -ge $timeout ]; then
                echo -e "${RED}Error: Timeout waiting for $name${NC}"
                exit 1
            fi
        done
    }

    wait_for "http://localhost:${PORT_API}/healthz" "API"
    wait_for "http://localhost:${PORT_S3}/minio/health/live" "S3"
    wait_for "http://localhost:${PORT_DEX}/dex/.well-known/openid-configuration" "Dex"
fi

# 4. Mode-specific Runner Setup
if [ "$MODE" == "hybrid" ]; then
    HOST_IP=$(ip -4 addr show docker0 | grep -Po 'inet \K[\d.]+' || hostname -I | awk '{print $1}')
    echo -e "${BLUE}>>> Importing Runner image to MicroK8s...${NC}"
    docker save stormchaser-runner-k8s:v1 | microk8s images import -

    echo -e "${BLUE}>>> Deploying K8s Runner via Helm...${NC}"
    mkdir -p "$REPO_ROOT/.tmp"
    microk8s config > "$REPO_ROOT/.tmp/kubeconfig"
    export KUBECONFIG="$REPO_ROOT/.tmp/kubeconfig"

    helm repo add grafana https://grafana.github.io/helm-charts 2>/dev/null || true
    helm repo update

    helm upgrade --install alloy grafana/alloy \
      --namespace default \
      -f "$REPO_ROOT/deploy/alloy/values.yaml" \
      --set "alloy.extraEnvVars[0].name=LOKI_HOST,alloy.extraEnvVars[0].value=$HOST_IP" \
      --wait

    helm upgrade --install stormchaser-runner "$REPO_ROOT/deploy/charts/stormchaser-runner-k8s" \
      --namespace default \
      --set config.natsUrl="nats://$HOST_IP:${PORT_NATS}" \
      --set config.rustLog="stormchaser_runner_k8s=debug" \
      --wait
elif [ "$MODE" == "microk8s" ]; then
    NAMESPACE="stormchaser"
    echo -e "${BLUE}>>> Building Stormchaser Docker images...${NC}"
    declare -A COMPONENTS
    COMPONENTS=(
        ["stormchaser-api"]="stormchaser-api"
        ["stormchaser-query"]="stormchaser-query"
        ["stormchaser-engine"]="stormchaser-engine"
        ["stormchaser-runner-k8s"]="stormchaser-runner-k8s"
        ["stormchaser-agent"]="stormchaser-agent"
        ["stormchaser-web"]="stormchaser-web"
    )

    for IMAGE_NAME in "${!COMPONENTS[@]}"; do
        BINARY_NAME=${COMPONENTS[$IMAGE_NAME]}
        echo -e "${BLUE}>>> Building $IMAGE_NAME...${NC}"
        if [ "$IMAGE_NAME" == "stormchaser-web" ]; then
            docker build -t "$IMAGE_NAME:latest" -f "$REPO_ROOT/Dockerfile.web" "$REPO_ROOT"
        else
            docker build -t "$IMAGE_NAME:latest" --build-arg BINARY="$BINARY_NAME" "$REPO_ROOT"
        fi
    done

    echo -e "${BLUE}>>> Importing images to MicroK8s...${NC}"
    for IMAGE_NAME in "${!COMPONENTS[@]}"; do
        docker save "$IMAGE_NAME:latest" | microk8s images import -
    done

    echo -e "${BLUE}>>> Creating namespace and TLS secrets...${NC}"
    microk8s kubectl create namespace "$NAMESPACE" || true
    microk8s kubectl create secret generic stormchaser-tls-certs       --namespace "$NAMESPACE"       --from-file=ca.crt="$CERT_DIR/ca.crt"       --from-file=tls.crt="$CERT_DIR/tls.crt"       --from-file=tls.key="$CERT_DIR/tls.key"       --dry-run=client -o yaml | microk8s kubectl apply -f -

    echo -e "${BLUE}>>> Deploying via Helm...${NC}"
    (
        cd "$REPO_ROOT/deploy/charts/stormchaser"
        rm -f charts/stormchaser-*.tgz
        helm dependency update

        helm upgrade --install stormchaser .           --namespace "$NAMESPACE"           --create-namespace           --set "stormchaser-orchestration.api.image.repository=stormchaser-api"           --set "stormchaser-orchestration.api.image.tag=latest"           --set "stormchaser-orchestration.api.image.pullPolicy=Never"           --set "stormchaser-orchestration.query.image.repository=stormchaser-query"           --set "stormchaser-orchestration.query.image.tag=v2"           --set "stormchaser-orchestration.query.image.pullPolicy=Never"           --set "stormchaser-orchestration.engine.image.repository=stormchaser-engine"           --set "stormchaser-orchestration.engine.image.tag=latest"           --set "stormchaser-orchestration.engine.image.pullPolicy=Never"           --set "stormchaser-runner-k8s.image.repository=stormchaser-runner-k8s"           --set "stormchaser-runner-k8s.image.tag=latest"           --set "stormchaser-runner-k8s.image.pullPolicy=Never"           --set "global.agent.image.repository=stormchaser-agent"           --set "global.agent.image.tag=latest"           --set "global.agent.image.pullPolicy=Never"
    )

    echo -e "${BLUE}>>> Deploying Dex Identity Provider...${NC}"
    if ! python3 -c "from passlib.hash import bcrypt" 2>/dev/null; then
        echo -e "${RED}Error: passlib[bcrypt] is not installed. Please run: pip install passlib[bcrypt]${NC}" >&2
        exit 1
    fi
    for persona in admin dev ops sec; do
        secret_name="dex-${persona}-secret"
        if ! microk8s kubectl get secret "$secret_name" -n "$NAMESPACE" >/dev/null 2>&1; then
            echo -e "${BLUE}>>> Reading password for Dex persona $persona from credentials.generated...${NC}"
            DEX_PASSWORD=$(grep "stormchaser-${persona}@paninfracon.net" "$REPO_ROOT/deploy/dex/credentials.generated" | cut -d' ' -f2)
            DEX_HASH=$(DEX_PASSWORD="$DEX_PASSWORD" python3 -c 'import os; from passlib.hash import bcrypt; print(bcrypt.hash(os.environ["DEX_PASSWORD"]))')
            microk8s kubectl create secret generic "$secret_name" \
                --namespace "$NAMESPACE" \
                --from-literal=password="$DEX_PASSWORD" \
                --from-literal=hash="$DEX_HASH"
            echo -e "${GREEN}>>> Stored Dex credentials for $persona in secret $secret_name.${NC}"
        fi
    done
    microk8s kubectl apply -f "$REPO_ROOT/deploy/dex-k8s/"
fi
# 5. Register Storage
echo -e "${BLUE}>>> Registering Local Storage (S3)...${NC}"
if command -v python3 >/dev/null 2>&1 && [ -f "$REPO_ROOT/scripts/generate_dev_token.py" ]; then
    echo -e "${BLUE}>>> Generating authentication token...${NC}"
    STORMCHASER_TOKEN=$(python3 "$REPO_ROOT/scripts/generate_dev_token.py")
    export STORMCHASER_TOKEN

    if [ -n "$STORMCHASER_TOKEN" ]; then
        if [[ "$MODE" == "docker" ]]; then
            STORMCHASER_API_URL=http://localhost:${PORT_API} "$REPO_ROOT/scripts/register-local-s3.sh"
        elif [[ "$MODE" == "hybrid" ]]; then
            # For hybrid mode, the runner is in K8s and MinIO is in Docker Compose
            # So the runner needs to access MinIO via the host IP.
            # And AWS CLI needs to access Minio via localhost:PORT_S3 (which is 9002 in hybrid)
            AWS_ENDPOINT="http://localhost:${PORT_S3}" \
            S3_ENDPOINT="http://$HOST_IP:${PORT_S3}" \
            STORMCHASER_API_URL="http://localhost:${PORT_API}" \
            "$REPO_ROOT/scripts/register-local-s3.sh"
        else
            # For microk8s, we need the dynamically generated password and cluster-internal DNS
            echo -e "${BLUE}>>> Registering cluster MinIO backend for SFS...${NC}"
            MINIO_PASSWORD=$(microk8s kubectl get secret -n stormchaser stormchaser-minio -o jsonpath='{.data.root-password}' | base64 -d)
            API_IP=$(microk8s kubectl get svc -n stormchaser stormchaser-stormchaser-orchestration-api -o jsonpath='{.spec.clusterIP}')
            API_URL="http://${API_IP}:${PORT_API}"

            # Wait for API to be ready
            echo -e "${BLUE}>>> Waiting for API to become ready at ${API_URL}...${NC}"
            API_READY=false
            for _ in {1..30}; do
                if curl -s -o /dev/null -w "%{http_code}" "$API_URL/api/health" | grep -q "200" || \
                   curl -s -o /dev/null -w "%{http_code}" "$API_URL/healthz" | grep -q "200"; then
                    API_READY=true
                    break
                fi
                sleep 5
            done
            if [[ "$API_READY" != "true" ]]; then
                echo -e "${RED}Error: API did not become ready at ${API_URL} within timeout.${NC}"
                exit 1
            fi

            curl -s -X POST "$API_URL/api/v1/connections" \
              -H "Authorization: Bearer $STORMCHASER_TOKEN" \
              -H "Content-Type: application/json" \
              -d '{
                "name": "local-minio",
                "description": "Local Minio S3-compatible storage for SFS parking",
                "connection_type": "s3",
                "is_default_sfs": true,
                "config": {
                  "endpoint": "http://stormchaser-minio.stormchaser.svc.cluster.local:9000",
                  "bucket": "stormchaser-sfs",
                  "region": "us-east-1",
                  "access_key": "stormchaser",
                  "secret_key": "'"$MINIO_PASSWORD"'",
                  "force_path_style": true
                }
              }' > /dev/null

            echo -e "${GREEN}>>> Storage backend registered successfully.${NC}"
        fi
    else
        echo -e "${RED}Error: Failed to obtain authentication token. Please register storage manually.${NC}"
    fi
else
    echo -e "${RED}Error: python3 or scripts/generate_dev_token.py not found. Please register storage manually.${NC}"
fi

echo -e "${GREEN}>>> Stormchaser is UP ($MODE mode)!${NC}"
echo -e "${BLUE}API:${NC} http://localhost:${PORT_API}"
if [[ -f "$REPO_ROOT/.tmp/kubeconfig" ]]; then
    echo -e "${BLUE}Kubeconfig:${NC} .tmp/kubeconfig"
fi
