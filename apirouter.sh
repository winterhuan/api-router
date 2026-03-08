#!/bin/bash
# API Router 管理脚本

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVICE_NAME="apirouter"
SERVICE_FILE="$SCRIPT_DIR/apirouter.service"
BINARY_PATH="$SCRIPT_DIR/target/release/apirouter"

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

# 检查是否为 root 用户
check_root() {
    if [ "$EUID" -ne 0 ]; then
        log_error "请使用 sudo 运行此脚本"
        exit 1
    fi
}

# 编译项目
build() {
    log_info "编译项目..."
    cd "$SCRIPT_DIR"
    source ~/.cargo/env 2>/dev/null || true
    cargo build --release
    log_info "编译完成: $BINARY_PATH"
}

# 安装 systemd 服务
install() {
    check_root
    
    # 确保二进制文件存在
    if [ ! -f "$BINARY_PATH" ]; then
        log_warn "二进制文件不存在，开始编译..."
        build
    fi
    
    # 复制服务文件
    log_info "安装 systemd 服务..."
    cp "$SERVICE_FILE" /etc/systemd/system/$SERVICE_NAME.service
    
    # 重新加载 systemd
    systemctl daemon-reload
    
    # 启用开机自启动
    systemctl enable $SERVICE_NAME
    
    log_info "服务安装完成"
    log_info "使用 'sudo ./apirouter.sh start' 启动服务"
}

# 卸载服务
uninstall() {
    check_root
    
    log_info "停止服务..."
    systemctl stop $SERVICE_NAME 2>/dev/null || true
    
    log_info "禁用开机自启动..."
    systemctl disable $SERVICE_NAME 2>/dev/null || true
    
    log_info "删除服务文件..."
    rm -f /etc/systemd/system/$SERVICE_NAME.service
    
    systemctl daemon-reload
    
    log_info "服务已卸载"
}

# 启动服务
start() {
    check_root
    
    if ! systemctl is-enabled $SERVICE_NAME &>/dev/null; then
        log_warn "服务未安装，正在安装..."
        install
    fi
    
    log_info "启动服务..."
    systemctl start $SERVICE_NAME
    sleep 1
    status
}

# 停止服务
stop() {
    check_root
    log_info "停止服务..."
    systemctl stop $SERVICE_NAME
    log_info "服务已停止"
}

# 重启服务
restart() {
    check_root
    log_info "重启服务..."
    systemctl restart $SERVICE_NAME
    sleep 1
    status
}

# 查看状态
status() {
    echo ""
    echo "========================================="
    echo "         API Router 服务状态"
    echo "========================================="
    
    if systemctl is-active $SERVICE_NAME &>/dev/null; then
        echo -e "状态:     ${GREEN}运行中${NC}"
    else
        echo -e "状态:     ${RED}已停止${NC}"
    fi
    
    if systemctl is-enabled $SERVICE_NAME &>/dev/null; then
        echo -e "自启动:   ${GREEN}已启用${NC}"
    else
        echo -e "自启动:   ${YELLOW}未启用${NC}"
    fi
    
    echo "端口:     1999"
    echo "管理界面: http://localhost:1999/admin-ui"
    echo "API 端点: http://localhost:1999/v1/messages"
    echo ""
    
    # 显示最近日志
    echo "最近日志:"
    echo "-----------------------------------------"
    journalctl -u $SERVICE_NAME -n 10 --no-pager 2>/dev/null || echo "无法读取日志"
    echo ""
}

# 查看日志
logs() {
    echo "查看实时日志 (按 Ctrl+C 退出):"
    journalctl -u $SERVICE_NAME -f
}

# 直接运行（前台模式，用于调试）
run() {
    cd "$SCRIPT_DIR"
    source ~/.cargo/env 2>/dev/null || true
    RUST_LOG=info exec "$BINARY_PATH" --port 1999
}

# 显示帮助
usage() {
    echo "API Router 管理脚本"
    echo ""
    echo "用法: $0 <命令>"
    echo ""
    echo "命令:"
    echo "  build     编译项目"
    echo "  install   安装 systemd 服务（开机自启动）"
    echo "  uninstall 卸载服务"
    echo "  start     启动服务"
    echo "  stop      停止服务"
    echo "  restart   重启服务"
    echo "  status    查看服务状态"
    echo "  logs      查看实时日志"
    echo "  run       前台运行（用于调试）"
    echo ""
}

# 主入口
case "${1:-}" in
    build)
        build
        ;;
    install)
        install
        ;;
    uninstall)
        uninstall
        ;;
    start)
        start
        ;;
    stop)
        stop
        ;;
    restart)
        restart
        ;;
    status)
        status
        ;;
    logs)
        logs
        ;;
    run)
        run
        ;;
    *)
        usage
        exit 1
        ;;
esac
