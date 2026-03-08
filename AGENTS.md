# APIRouter 项目上下文

> 本文件为 AI 助手提供项目上下文，帮助理解和修改代码。

## 项目概述

**APIRouter** 是一个多上游 API 代理服务，核心功能：
- **多上游故障切换**：配置多个 API 提供商，按优先级自动故障转移
- **熔断器保护**：连续失败 3 次后自动跳过故障上游 60 秒
- **API 格式转换**：支持 Anthropic、OpenAI、OpenAI Response、Gemini 格式互转
- **本地存储**：使用 JSON 文件存储配置和日志
- **访问控制**：可选的客户端 API Key 验证
- **请求日志**：记录最近 100 条请求详情

### 技术栈
- **语言**: Rust (Edition 2021)
- **Web 框架**: axum 0.7 + tokio 异步运行时
- **HTTP 客户端**: reqwest 0.12
- **序列化**: serde + serde_json
- **并发**: dashmap (并发 HashMap), RwLock
- **CLI**: clap 4

## 构建与运行

### 编译
```bash
# Debug 构建
cargo build

# Release 构建 (推荐生产使用)
cargo build --release
```

### 运行
```bash
# 直接运行
./target/release/apirouter

# 自定义参数
./target/release/apirouter --port 8080 --host 127.0.0.1 --data-dir /path/to/data

# 环境变量
RUST_LOG=debug ./target/release/apirouter  # 启用调试日志
HTTPS_PROXY=http://proxy:8080 ./target/release/apirouter  # 使用代理
```

### 测试
```bash
cargo test
```

## 项目结构

```
apirouter/
├── Cargo.toml              # 项目配置和依赖
├── src/
│   ├── main.rs             # 入口点，CLI 参数解析，路由定义
│   ├── config.rs           # 配置管理，本地 JSON 存储
│   ├── proxy.rs            # 代理核心：故障切换、熔断器、请求转发
│   ├── converters.rs       # API 格式转换器
│   └── admin.rs            # 管理 API 路由
├── frontend/
│   └── index.html          # Web 管理界面
├── data/                   # 数据目录 (运行时创建)
│   ├── config.json         # 配置存储
│   └── logs.json           # 请求日志
├── apirouter.service       # systemd 服务文件
└── apirouter.sh            # 部署脚本
```

## 核心模块说明

### `src/main.rs`
- CLI 参数定义 (`Args` struct)
- axum 路由配置
- 应用状态初始化 (`AppState`, `LogStore`)
- 请求入口和访问控制检查

### `src/config.rs`
- **核心数据结构**:
  - `AppConfig`: 全局配置
  - `Upstream`: 上游服务器配置 (base_url, keys, priority, model_map)
  - `ClientKey`: 客户端 API Key
  - `RequestLog`: 请求日志条目
- **存储**: JSON 文件读写，最多保留 100 条日志
- **工具函数**: `generate_api_key()`, `hash_password()`, `verify_password()`

### `src/proxy.rs`
- **熔断器** (`CIRCUIT_BREAKER`): DashMap 存储，失败阈值 3 次，冷却时间 60 秒
- **故障切换逻辑**:
  - `get_available_upstreams()`: 按优先级排序，排除熔断上游
  - 遍历上游和 API Keys 直到成功
- **触发故障切换的 HTTP 状态码**: `[401, 403, 429, 500, 502, 503, 504, 520, 522, 524]`
- **流式响应**: SSE 格式转换

### `src/converters.rs`
- **格式转换函数**:
  - `to_upstream()`: Anthropic → 目标格式
  - `from_upstream()`: 目标格式 → Anthropic
  - `convert_stream_chunk()`: SSE 流式块转换
- **支持的 API 格式**:
  - `anthropic`: Anthropic Claude API (原生格式)
  - `openai`: OpenAI Chat Completions API
  - `openai_response`: OpenAI Responses API
  - `gemini`: Google Gemini API

### `src/admin.rs`
- 管理 API 端点实现
- 密码验证通过 `x-admin-password` Header
- 配置更新后自动保存到 JSON 文件

## API 端点

| 端点 | 方法 | 说明 |
|------|------|------|
| `/` | GET | 健康检查 |
| `/v1/*` | ANY | 代理端点 (转发到上游) |
| `/admin-ui` | GET | Web 管理界面 |
| `/admin/verify` | POST | 验证管理员密码 |
| `/admin/config` | GET/POST | 获取/更新配置 |
| `/admin/client-keys` | GET/POST | 管理客户端 Keys |
| `/admin/generate-key` | POST | 生成新 Key |
| `/admin/logs` | GET/DELETE | 查看/清空日志 |

## 配置示例

```json
{
  "upstreams": [
    {
      "id": "primary",
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

## 开发约定

### 错误处理
- 使用 `Result<T, E>` 和 `?` 操作符
- HTTP 错误返回 JSON 格式: `{"error": {"message": "..."}}`

### 异步编程
- 使用 `tokio::sync::RwLock` 进行异步读写锁
- 使用 `Arc` 进行状态共享

### 日志
- 使用 `tracing` crate
- 日志级别通过 `RUST_LOG` 环境变量控制

### 代理支持
- 支持环境变量: `HTTPS_PROXY`, `HTTP_PROXY`, `NO_PROXY`

## 常见修改场景

### 添加新的 API 格式
1. 在 `src/config.rs` 的 `ApiFormat` enum 中添加新变体
2. 在 `src/converters.rs` 的 `to_upstream()` 和 `from_upstream()` 中添加转换逻辑
3. 更新 `convert_stream_chunk()` 支持流式转换

### 修改熔断器参数
- 编辑 `src/proxy.rs` 中的 `record_failure()` 函数
- 调整失败阈值 (当前为 3) 或冷却时间 (当前为 60 秒)

### 添加新的管理端点
1. 在 `src/admin.rs` 中添加处理函数
2. 在 `src/main.rs` 的路由配置中注册新路由

## 从 Python 迁移说明

本项目从 Python 重构为 Rust，主要变化：
- 原 `src/*.py` → 现 `src/*.rs`
- 原 `pyproject.toml` → 现 `Cargo.toml`
- 性能提升：单二进制文件，无运行时依赖
