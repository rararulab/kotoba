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

## 6. 怎么添加动漫角色 model（重点）

动漫角色声音一般走 **RVC 模型**。在 Kotoba 里通常是：
- 基础 TTS 用 `kokoro:<voice>` 生成音频
- 再用 `+rvc:<model>` 做角色音色转换

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

### 6.3 启用“基础音色 + 角色模型”

```bash
kotoba voice set kokoro:af_heart+rvc:naruto-rvc-v2
```

### 6.4 测试

```bash
kotoba play こんにちは
```

## 7. 手动添加本地角色模型（不走下载命令）

如果你已经有本地 RVC 文件，直接放到：

```text
~/.kotoba/models/rvc/<model_name>/model.pth
~/.kotoba/models/rvc/<model_name>/model.index   # 可选
```

然后设置：

```bash
kotoba voice set kokoro:af_heart+rvc:<model_name>
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
