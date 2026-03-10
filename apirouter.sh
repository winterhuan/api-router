#!/usr/bin/env bash
# API Router 管理脚本

set -e

# 保证在 sudo / 非登录 shell 中也能找到基础系统命令
export PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:$PATH"

SCRIPT_PATH="${BASH_SOURCE[0]}"
if command -v readlink >/dev/null 2>&1; then
    SCRIPT_PATH="$(readlink -f "$SCRIPT_PATH" 2>/dev/null || echo "$SCRIPT_PATH")"
fi

SCRIPT_DIR="$(cd "$(dirname "$SCRIPT_PATH")" && pwd)"
SERVICE_NAME="apirouter"
SERVICE_FILE="$SCRIPT_DIR/apirouter.service"
BINARY_PATH="$SCRIPT_DIR/target/release/apirouter"
CMD_LINK="/usr/local/bin/apirouter"
DEFAULT_HTTP_PROXY="http://127.0.0.1:10808"
DEFAULT_HTTPS_PROXY="http://127.0.0.1:10808"
DEFAULT_NO_PROXY="localhost,127.0.0.1,::1,evomap.ai,.evomap.ai,.feishu.cn,.larksuite.com,.larkoffice.com,.feishucdn.com,.bytedance.com,.volces.com,.volcengine.com,.aliyuncs.com,.qwen.ai,.minimax.io,.minimaxi.com,.moonshot.ai,.baidu.com,.baidubce.com,.deepseek.com,.nvidia.com,longcat.chat,.longcat.chat,gitcode.com"

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

resolve_user_home() {
    local username="$1"
    local user_home
    user_home="$(getent passwd "$username" 2>/dev/null | cut -d: -f6)"
    if [ -z "$user_home" ]; then
        user_home="/home/$username"
    fi
    echo "$user_home"
}

ensure_cargo() {
    if command -v cargo >/dev/null 2>&1; then
        return 0
    fi

    # sudo 执行时优先尝试原调用用户的 cargo 路径
    if [ -n "${SUDO_USER:-}" ] && [ "$SUDO_USER" != "root" ]; then
        local sudo_home
        sudo_home="$(resolve_user_home "$SUDO_USER")"
        if [ -x "$sudo_home/.cargo/bin/cargo" ]; then
            export PATH="$sudo_home/.cargo/bin:$PATH"
        fi
    fi

    # 再尝试当前用户的 cargo 路径
    if [ -x "$HOME/.cargo/bin/cargo" ]; then
        export PATH="$HOME/.cargo/bin:$PATH"
    fi

    if ! command -v cargo >/dev/null 2>&1; then
        log_error "未找到 cargo。请先安装 Rust，或为当前用户配置 ~/.cargo/bin 到 PATH。"
        exit 1
    fi
}

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

    # 使用 sudo 执行时，尽量以原用户身份编译，避免产物被 root 接管
    if [ "$EUID" -eq 0 ] && [ -n "${SUDO_USER:-}" ] && [ "$SUDO_USER" != "root" ]; then
        local sudo_home
        sudo_home="$(resolve_user_home "$SUDO_USER")"
        sudo -u "$SUDO_USER" env \
            PATH="$sudo_home/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" \
            bash -lc "cd '$SCRIPT_DIR' && cargo build --release"
    else
        ensure_cargo
        cd "$SCRIPT_DIR"
        cargo build --release
    fi

    log_info "编译完成: $BINARY_PATH"
}

normalize_proxy_mode() {
    local mode="${1:-ask}"
    case "$mode" in
        ask|--ask|"")
            echo "ask"
            ;;
        with-proxy|--with-proxy|proxy|--proxy)
            echo "with"
            ;;
        no-proxy|--no-proxy)
            echo "none"
            ;;
        *)
            log_error "无效 install 参数: $mode"
            log_info "可选参数: --with-proxy | --no-proxy"
            exit 1
            ;;
    esac
}

collect_proxy_env() {
    local mode="$1"
    local answer
    local input

    PROXY_ENABLED=0
    PROXY_HTTP="${HTTP_PROXY:-$DEFAULT_HTTP_PROXY}"
    PROXY_HTTPS="${HTTPS_PROXY:-$DEFAULT_HTTPS_PROXY}"
    PROXY_NO_PROXY="${NO_PROXY:-$DEFAULT_NO_PROXY}"

    case "$mode" in
        none)
            return 0
            ;;
        with)
            PROXY_ENABLED=1
            return 0
            ;;
        ask)
            if [ ! -t 0 ]; then
                log_warn "检测到非交互环境，默认不配置代理环境变量。可使用 'install --with-proxy' 强制开启。"
                return 0
            fi

            read -r -p "是否配置服务代理环境变量 (HTTP_PROXY/HTTPS_PROXY/NO_PROXY)? [y/N]: " answer
            case "$answer" in
                y|Y|yes|YES)
                    PROXY_ENABLED=1
                    read -r -p "HTTP_PROXY [$PROXY_HTTP]: " input
                    PROXY_HTTP="${input:-$PROXY_HTTP}"
                    read -r -p "HTTPS_PROXY [$PROXY_HTTPS]: " input
                    PROXY_HTTPS="${input:-$PROXY_HTTPS}"
                    read -r -p "NO_PROXY [$PROXY_NO_PROXY]: " input
                    PROXY_NO_PROXY="${input:-$PROXY_NO_PROXY}"
                    ;;
                *)
                    PROXY_ENABLED=0
                    ;;
            esac
            ;;
    esac
}

write_service_file() {
    local target_file="$1"
    local temp_file
    temp_file="$(mktemp /tmp/apirouter.service.XXXXXX)"

    awk \
        -v proxy_enabled="$PROXY_ENABLED" \
        -v http_proxy="$PROXY_HTTP" \
        -v https_proxy="$PROXY_HTTPS" \
        -v no_proxy="$PROXY_NO_PROXY" '
        /^Environment=(HTTP_PROXY|HTTPS_PROXY|NO_PROXY)=/ { next }
        /^\[Install\]/ && proxy_enabled == "1" {
            print "Environment=HTTP_PROXY=" http_proxy
            print "Environment=HTTPS_PROXY=" https_proxy
            print "Environment=NO_PROXY=" no_proxy
        }
        { print }
    ' "$SERVICE_FILE" > "$temp_file"

    cp "$temp_file" "$target_file"
    rm -f "$temp_file"
}

# 安装 systemd 服务
install() {
    local proxy_mode

    check_root
    proxy_mode="$(normalize_proxy_mode "${1:-ask}")"
    collect_proxy_env "$proxy_mode"
    
    # 确保二进制文件存在
    if [ ! -f "$BINARY_PATH" ]; then
        log_warn "二进制文件不存在，开始编译..."
        build
    fi
    
    # 复制服务文件
    log_info "安装 systemd 服务..."
    write_service_file "/etc/systemd/system/$SERVICE_NAME.service"

    if [ "$PROXY_ENABLED" -eq 1 ]; then
        log_info "已启用代理环境变量"
    else
        log_info "未启用代理环境变量"
    fi
    
    # 重新加载 systemd
    systemctl daemon-reload
    
    # 启用开机自启动
    systemctl enable $SERVICE_NAME

    # 安装全局命令
    install_path
    
    log_info "服务安装完成"
    log_info "使用 'sudo apirouter start' 启动服务"
}

# 安装全局命令到 PATH
install_path() {
    check_root
    chmod +x "$SCRIPT_DIR/apirouter.sh"
    ln -sf "$SCRIPT_DIR/apirouter.sh" "$CMD_LINK"
    log_info "已安装全局命令: $CMD_LINK -> $SCRIPT_DIR/apirouter.sh"
    log_info "现在可在任意目录运行: sudo apirouter status"
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

    log_info "删除全局命令..."
    rm -f "$CMD_LINK"
    
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
    if [ ! -f "$BINARY_PATH" ]; then
        log_warn "二进制文件不存在，开始编译..."
        build
    fi
    RUST_LOG=info exec "$BINARY_PATH" --port 1999
}

# 显示帮助
usage() {
    echo "API Router 管理脚本"
    echo ""
    echo "用法: $0 <命令> [参数]"
    echo ""
    echo "命令:"
    echo "  build     编译项目"
    echo "  path      安装全局命令到 PATH（/usr/local/bin/apirouter）"
    echo "  install   安装 systemd 服务（开机自启动）"
    echo "            参数: --with-proxy | --no-proxy"
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
    path)
        install_path
        ;;
    install)
        install "${2:-ask}"
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
