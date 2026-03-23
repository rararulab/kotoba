# Kotoba CLI 使用说明（简单版）

这份文档只讲两件事：
- 这个 CLI 怎么快速用起来
- 怎么添加动漫角色声音 model

## 1. 先安装

```bash
git clone https://github.com/rararulab/kotoba && cd kotoba
cargo install --path .
```

安装后确认：

```bash
kotoba --help
```

## 2. 第一次启动（必做）

```bash
kotoba setup
```

`setup` 会做三件事：
- 初始化数据库
- 安装 VOICEVOX 引擎
- 生成配置文件（默认在 `~/.kotoba`）

如果你只想先建库，不下语音引擎，也可以：

```bash
kotoba init
```

## 3. 最小学习流程（3 条命令）

### 3.1 添加单词

```bash
kotoba add 成功 せいこう success --level N5
```

### 3.2 查看待复习

```bash
kotoba review
```

### 3.3 记录复习结果

```bash
kotoba seen 成功 recalled
```

`QUALITY` 现在使用文本值：
- `forgot`：忘了
- `recognized`：认识但不熟
- `recalled`：能快速回忆

## 4. 语法学习常用命令

```bash
kotoba grammar add ～ている "ongoing action" --level N5
kotoba grammar list
kotoba review --grammar
kotoba seen --grammar ～ている recognized
```

## 5. 发音与声音切换

```bash
kotoba play 成功
kotoba voice list
kotoba voice set voicevox:3
```

### 5.5 RVC 模型管理

```bash
# 查看所有已下载的 RVC 模型
kotoba voice rvc list

# 设置 RVC 模型（支持模糊匹配）
kotoba voice rvc set miku
kotoba voice rvc set ichika

# 关闭 RVC
kotoba voice rvc off
```

模糊匹配：输入名称的任意子串即可，大小写不敏感。如果匹配到多个模型会提示你输入更精确的名称。

## 6. 怎么添加动漫角色 model（重点）

动漫角色声音一般走 **RVC 模型**。在 Kotoba 里分两步配置：
- 基础 TTS 用 `voice.active`（如 `kokoro:jf_alpha`）控制发音
- 角色音色用 `rvc.model` 控制（如花泽香菜 RVC 模型）

### 6.1 下载 Kokoro 基础模型（一次即可）

```bash
kotoba huggingface add kokoro
```

### 6.2 下载角色 RVC 模型

假设你在 HuggingFace 找到角色模型仓库：`some-user/naruto-rvc-v2`

```bash
kotoba huggingface add rvc:some-user/naruto-rvc-v2
```

下载后模型目录大致是：

```text
~/.kotoba/models/rvc/naruto-rvc-v2/model.pth
~/.kotoba/models/rvc/naruto-rvc-v2/model.index   # 可选
```

注意：实际模型名取仓库最后一段，也就是这里的 `naruto-rvc-v2`。

### 6.3 启用基础音色和角色模型

Base voice 和 RVC model **分开设置**，互不影响：

```bash
# 设置基础 TTS 声音（控制发音）
kotoba voice set kokoro:jf_alpha

# 设置 RVC 角色模型（支持模糊匹配）
kotoba voice rvc set naruto
```

> **⚠️ 重要：Kokoro 的 base voice 必须和目标语言匹配。**
>
> 日语用 `jf_*` / `jm_*`（如 `jf_alpha`），英语用 `af_*` / `am_*`（如 `af_heart`）。
> 如果用英语 voice 播放日语文本，发音会完全错误——RVC 只改音色，不修正发音。
>
> | 前缀 | 语言 | 示例 |
> |------|------|------|
> | `jf_` | 日语女声 | `jf_alpha`, `jf_beta` |
> | `jm_` | 日语男声 | `jm_alpha`, `jm_beta` |
> | `af_` | 英语女声 | `af_heart`, `af_sky` |
> | `am_` | 英语男声 | `am_adam` |
>
> 要关闭 RVC（只用基础 TTS）：`kotoba voice rvc off`

### 6.4 测试

```bash
kotoba play こんにちは --enable
```

## 7. 手动添加本地角色模型（不走下载命令）

如果你已经有本地 RVC 文件，直接放到：

```text
~/.kotoba/models/rvc/<model_name>/model.pth
~/.kotoba/models/rvc/<model_name>/model.index   # 可选
```

然后设置：

```bash
kotoba voice rvc set <model_name>
```

## 8. 常见问题

### Q1: `model not found: xxx`

检查目录是否是：

```text
~/.kotoba/models/rvc/xxx/model.pth
```

并确认 `voice set` 里写的模型名和目录名完全一致。

### Q2: `sidecar failed to start ... check python3 and uvicorn`

RVC 侧车需要 Python 环境，先安装：

```bash
python3 -m pip install fastapi uvicorn
```

### Q3: 我想看系统依赖是否正常

```bash
kotoba doctor
```

### Q4: 为什么加了 RVC 模型，声音变化不明显？

当前版本的 `rvc-sidecar` 主要用于打通接口和流程，真实 RVC 推理还在接入中。也就是说你现在可以先验证：
- 模型下载是否成功
- 路径与命名是否正确
- `kokoro + rvc` 命令链路是否可用

---

如果你只想一句话记住“角色模型怎么加”：

```bash
kotoba huggingface add kokoro
kotoba huggingface add rvc:<hf用户名>/<模型仓库>
kotoba voice set kokoro:af_heart+rvc:<模型名>
kotoba play <任意日语文本>
```
