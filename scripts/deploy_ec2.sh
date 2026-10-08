#!/usr/bin/env bash
set -euo pipefail

# Usage: ./scripts/deploy_ec2.sh <EC2_PUBLIC_IP_OR_DNS> [KEY_PATH]

if [ $# -lt 1 ]; then
    echo "Usage: $0 <EC2_PUBLIC_IP_OR_DNS> [KEY_PATH]"
    echo "Example: $0 54.210.123.45 snlang.pem"
    exit 1
fi

EC2_HOST="$1"
KEY_PATH="${2:-snlang.pem}"
SSH_USER="ec2-user"
REMOTE_DIR="/home/ec2-user/assembly"

if [ ! -f "$KEY_PATH" ]; then
    echo "Error: Key file '$KEY_PATH' not found."
    exit 1
fi

chmod 400 "$KEY_PATH"

echo "==> Testing SSH connection to ${SSH_USER}@${EC2_HOST}..."
ssh -i "$KEY_PATH" -o StrictHostKeyChecking=no -o ConnectTimeout=10 "${SSH_USER}@${EC2_HOST}" "echo 'Connected successfully to EC2 instance.'"

echo "==> Setting up Swap (2GB) on EC2 to prevent OOM during compilation..."
ssh -i "$KEY_PATH" "${SSH_USER}@${EC2_HOST}" << 'EOF'
if [ ! -f /swapfile ]; then
    sudo fallocate -l 2G /swapfile || sudo dd if=/dev/zero of=/swapfile bs=1M count=2048
    sudo chmod 600 /swapfile
    sudo mkswap /swapfile
    sudo swapon /swapfile
    echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab
    echo "Swap enabled:"
    free -h
fi
EOF

echo "==> Installing system packages (clang, gcc, git, make, sqlite, sqlite-devel)..."
ssh -i "$KEY_PATH" "${SSH_USER}@${EC2_HOST}" << 'EOF'
sudo dnf install -y clang gcc git make sqlite sqlite-devel
clang --version | head -n 1
EOF

echo "==> Ensuring Rust / Cargo is installed..."
ssh -i "$KEY_PATH" "${SSH_USER}@${EC2_HOST}" << 'EOF'
if ! command -v cargo &> /dev/null; then
    echo "Installing Rust toolchain..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
fi
EOF

echo "==> Synchronizing project repository files to EC2..."
ssh -i "$KEY_PATH" "${SSH_USER}@${EC2_HOST}" "mkdir -p ${REMOTE_DIR}"

rsync -avz -e "ssh -i ${KEY_PATH} -o StrictHostKeyChecking=no" \
    --exclude '.git' \
    --exclude 'node_modules' \
    --exclude 'compiler/target' \
    --exclude '*.pem' \
    --exclude '*.o' \
    --exclude '.DS_Store' \
    --exclude 'snc' \
    --exclude 'website_server' \
    ./ "${SSH_USER}@${EC2_HOST}:${REMOTE_DIR}/"

echo "==> Building SNlang compiler (snc) and website server on EC2..."
ssh -i "$KEY_PATH" "${SSH_USER}@${EC2_HOST}" << 'EOF'
source "$HOME/.cargo/env"
cd /home/ec2-user/assembly

# Build snc compiler
echo "Building snc compiler..."
cd compiler
cargo build --release
cp target/release/snc ../snc
chmod +x ../snc
cd ..

# Verify snc
./snc --version || true

# Build website server
echo "Building website server..."
./snc examples/website/server.sn -o website_server
chmod +x website_server
EOF

echo "==> Configuring systemd service for automatic startup and daemonization..."
ssh -i "$KEY_PATH" "${SSH_USER}@${EC2_HOST}" << 'EOF'
sudo tee /etc/systemd/system/snlang-website.service > /dev/null << 'SERVICE'
[Unit]
Description=SNlang Starter and Docs Server
After=network.target

[Service]
Type=simple
User=ec2-user
WorkingDirectory=/home/ec2-user/assembly
ExecStart=/home/ec2-user/assembly/website_server
Restart=always
RestartSec=3
Environment="PATH=/home/ec2-user/.cargo/bin:/usr/local/bin:/usr/bin:/bin"

[Install]
WantedBy=multi-user.target
SERVICE

sudo systemctl daemon-reload
sudo systemctl enable --now snlang-website.service
sudo systemctl restart snlang-website.service
sleep 2
sudo systemctl status snlang-website.service --no-pager
EOF

echo "==> Testing local website response on EC2..."
ssh -i "$KEY_PATH" "${SSH_USER}@${EC2_HOST}" "curl -I http://localhost:8090/ || true"

echo ""
echo "================================================================="
echo " Deployment Complete!"
echo " Access your website at: http://${EC2_HOST}:8090"
echo "================================================================="
