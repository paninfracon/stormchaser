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
    echo "  --mode [k8s|docker]  Select runner environment (default: docker)"
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

if [[ "$MODE" != "k8s" && "$MODE" != "docker" ]]; then
    echo -e "${RED}Error: Invalid mode '$MODE'. Use 'k8s' or 'docker'.${NC}"
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
    docker compose -p stormchaser-k8s --profile "*" down -v || true
    docker compose --profile "*" down -v || true
}

if [ "$CLEANUP" = true ]; then
    echo -e "${BLUE}>>> Performing full cleanup...${NC}"
    cleanup_docker
    cleanup_k8s
fi

# 0. Setup patched ratatui-form locally
"$REPO_ROOT/scripts/patch-ratatui-form.sh"

# 1. Generate TLS Certificates
"$REPO_ROOT/scripts/generate-certs.sh"

# 2. Setup MicroK8s ONLY if in k8s mode
if [ "$MODE" == "k8s" ]; then
    echo -e "${BLUE}>>> Verifying MicroK8s...${NC}"
    if ! command -v microk8s &> /dev/null; then
        echo -e "${RED}Error: microk8s is not installed. Required for --mode k8s.${NC}"
        exit 1
    fi

    if ! microk8s status --wait-ready > /dev/null 2>&1; then
        echo -e "${BLUE}>>> Starting MicroK8s...${NC}"
        run_privileged systemctl restart snap.microk8s.daemon-kubelite.service
        microk8s status --wait-ready
    fi

    # Enable addons
    for addon in dns rbac; do
        if ! microk8s status | sed -n '/enabled:/,/disabled:/p' | grep -q "\b$addon\b"; then
            echo -e "${BLUE}>>> Enabling MicroK8s addon $addon...${NC}"
            microk8s enable "$addon" || run_privileged microk8s enable "$addon" || true
        fi
    done
fi

# Set isolated ports based on the mode
if [ "$MODE" == "k8s" ]; then
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
fi

# 2.5 Generate Dex config if missing or cleanup requested
if [ "$CLEANUP" = true ] || [ ! -f "$REPO_ROOT/deploy/dex/config.generated.yaml" ]; then
    echo -e "${BLUE}>>> Generating random passwords for Dex personas...${NC}"
    export REPO_ROOT
    python3 - << 'EOF'
import os
import stat
import secrets
try:
    from passlib.hash import bcrypt
except ImportError:
    print("\033[0;31mError: passlib is not installed. Please pip install passlib bcrypt.\033[0m")
    exit(1)

def gen_and_hash():
    pw = secrets.token_urlsafe(16)
    return pw, bcrypt.hash(pw)

repo_root = os.environ.get("REPO_ROOT", ".")
template_path = os.path.join(repo_root, "deploy/dex/config.yaml")
out_path = os.path.join(repo_root, "deploy/dex/config.generated.yaml")
cred_path = os.path.join(repo_root, "deploy/dex/credentials.generated")
role_map = {"ADMIN": "admin", "DEV": "dev", "OPS": "ops", "SEC": "sec"}

with open(template_path, "r") as f:
    content = f.read()

with open(cred_path, "w") as cred_file:
    cred_file.write("# Dex persona credentials — keep secret, do not commit\n")
    for role, role_email in role_map.items():
        pw, phash = gen_and_hash()
        cred_file.write(f"stormchaser-{role_email}@paninfracon.net: {pw}\n")
        content = content.replace(f"PASSWORD_HASH_{role}", phash)

os.chmod(cred_path, stat.S_IRUSR | stat.S_IWUSR)

with open(out_path, "w") as f:
    f.write(content)
os.chmod(out_path, stat.S_IRUSR | stat.S_IWUSR)

print(f"Dex credentials written to {cred_path} (mode 0600).")
EOF
fi

# 3. Start Docker Services
echo -e "${BLUE}>>> Starting Stormchaser backend in Docker ($MODE mode)...${NC}"
COMPOSE_PROFILES="$MODE"

# Build and start
docker compose -p "stormchaser-${MODE}" --profile "$COMPOSE_PROFILES" up -d --build
docker compose -p "stormchaser-${MODE}" build stormchaser-agent # Ensure agent is built

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

# 4. Mode-specific Runner Setup
if [ "$MODE" == "k8s" ]; then
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
fi

# 5. Register Storage
echo -e "${BLUE}>>> Registering Local Storage (S3)...${NC}"
if command -v python3 >/dev/null 2>&1 && [ -f "$REPO_ROOT/get_token.py" ]; then
    echo -e "${BLUE}>>> Generating authentication token...${NC}"
    STORMCHASER_TOKEN=$(python3 "$REPO_ROOT/get_token.py")
    export STORMCHASER_TOKEN
    if [ -n "$STORMCHASER_TOKEN" ]; then
        STORMCHASER_API_URL=http://localhost:${PORT_API} "$REPO_ROOT/scripts/register-local-s3.sh"
    else
        echo -e "${RED}Error: Failed to obtain authentication token. Please register storage manually.${NC}"
    fi
else
    echo -e "${RED}Error: python3 or get_token.py not found. Please register storage manually.${NC}"
    echo -e "  stormchaser login --issuer http://localhost:${PORT_DEX}/dex --client-id stormchaser-cli"
    echo -e "  export STORMCHASER_TOKEN=\"<paste-token-here>\""
    echo -e "  STORMCHASER_API_URL=http://localhost:${PORT_API} $REPO_ROOT/scripts/register-local-s3.sh"
fi

echo -e "${GREEN}>>> Stormchaser is UP ($MODE mode)!${NC}"
echo -e "${BLUE}API:${NC} http://localhost:${PORT_API}"
if [ "$MODE" == "k8s" ]; then
    echo -e "${BLUE}Kubeconfig:${NC} .tmp/kubeconfig"
fi
