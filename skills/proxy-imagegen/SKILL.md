---
name: proxy-imagegen
description: 通过本机 Codex 配置和中转站调用 Responses API 的 GPT Image 2 生图；用户或任务要求生成图片时默认使用，除非明确指定其他方式。
---

# 代理生图

按用户选择，通过当前 Codex 上游调用 GPT Image 2。保留用户已经授予的接口和密钥使用授权；技能不额外添加逐次确认要求。若用户另有指定，以用户指示为准，不静默更换模型或供应商。

## 执行

MWC Linux 工作区直接运行下面的 Bash 脚本；不要为此安装 Python。读取 `CODEX_HOME`（默认 `~/.codex`）下的配置和认证文件；`--config`、`--auth` 可覆盖路径。配置中的 `model`、`model_provider` 和选中提供商的 `base_url` 使用单行 TOML 字符串（双引号或单引号）；不支持的写法会明确报错，不猜测其他提供商。

1. 读取 `$CODEX_HOME/config.toml`，取当前 `model_provider` 对应 `[model_providers.<name>]` 的 `base_url`，不要取第一个 `base_url`；读取 `model` 作为 Responses 外层模型。
2. 读取 `$CODEX_HOME/auth.json` 的 `OPENAI_API_KEY`。只用于请求认证，绝不输出、记录或回显。
3. 直接执行本技能的 `scripts/generate.sh --prompt "<用户的生图提示词>" --out "<输出文件>"`。脚本使用 Bash、curl、jq 和 Linux 基础命令，不依赖 Python 或 PowerShell。它向 `POST {base_url}/responses` 发送：

   ```json
   {
     "model": "<config.toml 中的 model>",
     "input": "<用户的生图提示词>",
     "tools": [{"type": "image_generation", "model": "gpt-image-2"}],
     "tool_choice": {"type": "image_generation"},
     "stream": true,
     "store": false
   }
   ```

4. 在同一进程内读取 SSE 至 `response.completed`，从最终 `response.output` 或已完成的 `response.output_item.done` 中取得 `image_generation_call.result`，解码并验证 PNG/JPEG 后保存。不把局部预览当成最终图片；终端进程结束才表示文件写完，不另起状态轮询。
5. 检查输出文件存在且非空，然后返回绝对路径。

## 失败处理

- 对无图片结果、SSE 中断、连接异常、HTTP 408/429/5xx，脚本默认最多尝试 3 次（含首次），短暂退避并输出不含凭据的原因；用户已授权正常重试，无需每次确认。可通过 `--attempts` 调整。
- HTTP 400/401/403 等请求或鉴权错误先诊断配置；不要相同请求无限重发。达到重试上限后报告错误、响应状态和脱敏事件类型，继续任务中不依赖图片的工作。
- 生成成功并验证文件后停止，不重复请求已成功资产；不要输出完整响应、Base64 或凭据。重试可能重复计费，不将其声称为免费或幂等操作。

## 约束

- 优先保存 PNG；不得用 SVG 代替模型生成的位图。
- 不要把 API Key、完整响应或包含凭证的请求体打印到终端。
- 默认输出到用户指定路径；未指定时输出到当前 workspace 或 `$CODEX_HOME/generated_images/`。
