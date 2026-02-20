# GoldBot - AI 终端自动化助手

一个基于 Rust 开发的跨平台 TUI Agent，通过 LLM 自动规划和执行 Shell 命令来完成任务。

## 特性

- 三级安全控制：Safe/Confirm/Block
- 持久化记忆：短期按日期存储，长期自动提取偏好
- ReAct 循环：思考-执行-观察-再思考
- 智能命令分类：Read/Write/Update/Search/Bash
- 实时 TUI：流式显示思考过程，完成后默认折叠
- 跨平台支持：macOS/Linux (bash) / Windows (PowerShell)

## 安装

### 方法一：从 GitHub Releases 安装（推荐）

直接下载预编译的二进制文件：

```bash
# 获取最新版本
VERSION=$(curl -s https://api.github.com/repos/your-username/GoldBot/releases/latest | grep tag_name | head -1 | cut -d '"' -f 4)

# macOS / Linux (下载并解压)
curl -LO "https://github.com/your-username/GoldBot/releases/download/${VERSION}/goldbot-${VERSION#v}-linux-x86_64.tar.gz"
tar -xzf "goldbot-${VERSION#v}-linux-x86_64.tar.gz"
mkdir -p ~/.local/bin && mv goldbot ~/.local/bin/
```

或访问 [Releases 页面](https://github.com/your-username/GoldBot/releases) 手动下载。

### 方法二：从源码编译

```bash
git clone https://github.com/your-username/GoldBot.git
cd GoldBot
cargo run
# 或
cargo build --release
cargo install --path .
```

### 环境变量（可选）

```bash
export GOLDBOT_USE_CODEX=1
export GOLDBOT_MEMORY_DIR=/your/custom/path
```
