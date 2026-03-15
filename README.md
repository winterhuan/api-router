# APIRouter

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)

> 🚀 **APIRouter** — 多上游 API 代理服务，支持故障切换、熔断器，统一转换为 Anthropic 格式

## ✨ 功能特性

- **多上游故障切换**：配置多个 API 提供商，自动故障转移
- **熔断器保护**：连续失败 3 次后自动跳过故障上游 60 秒
- **格式转换**：支持 OpenAI、Gemini 等格式统一转换为 Anthropic 格式
- **请求日志**：记录最近 100 条请求详情
- **访问控制**：支持客户端 API Key 验证
- **Web 管理界面**：可视化配置管理

## 🚀 快速开始

### 方式一：管理脚本（推荐）

```bash
# 克隆项目
git clone https://github.com/winterhuan/api-router.git
cd api-router

# 一键启动（自动编译、安装服务、启动）
./apirouter.sh start
```

**首次运行会自动完成：**
1. 检测并编译项目
2. 安装用户级 systemd 服务
3. 启动服务

**后续使用：**
```bash
./apirouter.sh status    # 查看状态
./apirouter.sh restart   # 重启服务
./apirouter.sh logs      # 查看日志
./apirouter.sh stop      # 停止服务
```

**或者使用 systemctl：**
```bash
systemctl --user start apirouter
systemctl --user status apirouter
systemctl --user restart apirouter
```

### 方式二：手动编译运行

```bash
# 编译
cargo build --release

# 运行
./target/release/apirouter
```

### 运行参数

```bash
API Router - Multi-upstream proxy with failover

Usage: apirouter [OPTIONS]

Options:
  -p, --port <PORT>          端口号 [默认: 1999]
      --host <HOST>          绑定地址 [默认: 0.0.0.0]
      --data-dir <DATA_DIR>  数据存储目录 [默认: ./data]
  -h, --help                 显示帮助
  -V, --version              显示版本
```

### 代码更新后应用

```bash
# 拉取最新代码
git pull

# 重启服务（start 会自动检测并编译）
./apirouter.sh build && ./apirouter.sh restart

# 或者分步操作
./apirouter.sh build      # 重新编译
./apirouter.sh restart    # 重启服务
```

### 安装命令到 PATH

```bash
./apirouter.sh path    # 安装到 ~/.local/bin/apirouter

# 确保 PATH 包含 ~/.local/bin，添加到 ~/.bashrc：
# export PATH="$HOME/.local/bin:$PATH"

# 之后可在任意目录使用
apirouter status
apirouter restart
```

### 访问服务

- **API 端点**: `http://localhost:1999/v1/messages`
- **管理后台**: `http://localhost:1999/admin-ui`
- **默认密码**: `admin` (⚠️ 登录后请立即修改)

## 📖 使用方法

### 接入第三方客户端

在 ChatBox、NextChat、LobeHub 等客户端中：

- **API 地址**: `http://localhost:1999`
- **API Key**: 你的上游 API Key (或启用访问控制后使用客户端 Key)
- **模型**: `claude-opus-4-6` 等

### 配置上游服务器

通过管理后台或直接编辑 `data/config.json`：

```json
{
  "upstreams": [
    {
      "id": "upstream-anthropic",
      "base_url": "https://api.anthropic.com/v1",
      "api_format": "anthropic",
      "keys": ["sk-ant-xxx"],
      "model_map": {
        "claude-opus-4-6": "claude-opus-4-20250514"
      },
      "priority": 1,
      "enabled": true
    }
  ],
  "debug_mode": false,
  "access_control_enabled": false,
  "client_keys": []
}
```

### 支持的上游 API 格式

所有格式统一转换为 Anthropic 格式返回给客户端：

| 格式 | 说明 |
|------|------|
| `anthropic` | Anthropic Claude API（透传，无需转换）|
| `openai` | OpenAI Chat Completions API |
| `openai_response` | OpenAI Responses API |
| `gemini` | Google Gemini API |

## 🔧 管理接口

| 端点 | 方法 | 说明 |
|------|------|------|
| `/admin/verify` | POST | 验证管理员密码 |
| `/admin/config` | GET/POST | 获取/更新配置 |
| `/admin/client-keys` | GET/POST | 管理客户端 API Keys |
| `/admin/generate-key` | POST | 生成新的客户端 Key |
| `/admin/logs` | GET/DELETE | 查看/清空请求日志 |
| `/` | GET | 健康检查 |

## 📁 项目结构

```
apirouter/
├── Cargo.toml          # Rust 项目配置
├── src/
│   ├── main.rs         # 主入口 (axum web 服务)
│   ├── config.rs       # 配置管理 (本地 JSON 存储)
│   ├── converters.rs   # API 格式转换器
│   ├── proxy.rs        # 反向代理 + 熔断器
│   └── admin.rs        # 管理 API 路由
├── frontend/
│   └── index.html      # Web 管理界面
└── data/               # 数据存储目录
    ├── config.json     # 配置文件
    └── logs.json       # 请求日志
```

## 📄 开源协议

本项目采用 [MIT License](LICENSE) 协议开源。
