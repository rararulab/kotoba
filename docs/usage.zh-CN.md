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

动漫角色声音走 **Kokoro + RVC** pipeline：
- 基础 TTS 用 `kokoro:<voice>` 生成日语音频
- 再用 `+rvc:<model>` 做角色音色转换（RVC v2 推理）

完整流程分 4 步：准备 Python 环境 → 下载模型 → 设置声音 → 测试。

### 6.1 准备 RVC Python 环境（一次即可）

RVC 推理依赖 Python + PyTorch，需要用 [uv](https://docs.astral.sh/uv/) 创建一个独立 venv：

```bash
# 安装 uv（如果还没装）
curl -LsSf https://astral.sh/uv/install.sh | sh

# 在 kotoba 管理目录下创建 Python 3.10 venv
uv venv --python 3.10 ~/.kotoba/venvs/rvc

# 安装 RVC 推理依赖
uv pip install --python ~/.kotoba/venvs/rvc/bin/python3 \
  infer-rvc-python soundfile "setuptools<81" "numpy<2"
```

> **为什么要 Python 3.10？** fairseq（RVC 依赖）在 3.11+ 上有 dataclass 兼容问题。

安装完用 `kotoba doctor` 确认：

```bash
kotoba doctor
```

输出中应该能看到：

```text
✓ rvc_python   infer-rvc-python available (~/.kotoba/venvs/rvc/bin/python3)
```

**如果 Python 不在默认位置**，可以通过环境变量或配置指定：

```bash
# 方式 1：环境变量（临时）
export RVC_PYTHON=/path/to/your/python3

# 方式 2：写入配置（永久）
kotoba config set rvc.python /path/to/your/python3
```

Python 路径解析优先级：`RVC_PYTHON` 环境变量 > `config.toml` 中 `rvc.python` > `~/.kotoba/venvs/rvc/bin/python3` > 系统 `python3`。

### 6.2 下载 Kokoro 基础模型（一次即可）

```bash
kotoba huggingface add kokoro
```

### 6.3 下载角色 RVC 模型

去 [HuggingFace](https://huggingface.co) 搜索 RVC 模型，比如：

```bash
# 标准仓库（自动检测 .pth 和 .index 文件）
kotoba huggingface add rvc:some-user/naruto-rvc-v2

# 多模型仓库（指定子路径）
kotoba huggingface add rvc:ttttdiva/rvc_okiba:Hatsune_Miku
```

下载后模型目录：

```text
~/.kotoba/models/rvc/<model_name>/
  ├── *.pth          # 模型权重（必须）
  └── *.index        # 特征索引（可选，有则自动使用）
```

> **模型名** = 仓库最后一段（或子路径最后一段），比如 `naruto-rvc-v2` 或 `Hatsune_Miku`。

查看已安装的模型：

```bash
kotoba huggingface list
```

### 6.4 启用"基础音色 + 角色模型"

```bash
kotoba voice set kokoro:af_heart+rvc:naruto-rvc-v2
```

格式说明：`kokoro:<kokoro_voice>+rvc:<rvc_model_name>`

- `kokoro_voice`：Kokoro 基础声音，如 `af_heart`、`af_bella`、`am_adam`
- `rvc_model_name`：`~/.kotoba/models/rvc/` 下的目录名

### 6.5 测试

```bash
kotoba play こんにちは
```

预期输出：

```text
synthesizing: こんにちは...
converting with RVC model: naruto-rvc-v2...
cached (kokoro): ~/.kotoba/audio/こんにちは_kokoro_af_heart+rvc:naruto-rvc-v2.wav
```

第一次合成需要加载模型（约 10-30 秒），之后会使用缓存。

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

## 8. 完整 example：从零到出声

```bash
# 1. 安装 kotoba
cargo install --path .
kotoba setup

# 2. 准备 RVC Python 环境
uv venv --python 3.10 ~/.kotoba/venvs/rvc
uv pip install --python ~/.kotoba/venvs/rvc/bin/python3 \
  infer-rvc-python soundfile "setuptools<81" "numpy<2"

# 3. 下载模型
kotoba huggingface add kokoro
kotoba huggingface add rvc:ttttdiva/rvc_okiba:Hatsune_Miku

# 4. 设置声音
kotoba voice set kokoro:af_heart+rvc:Hatsune_Miku

# 5. 验证环境
kotoba doctor

# 6. 开始用
kotoba play こんにちは
kotoba play ありがとう
kotoba play おはようございます
```

## 9. 常见问题

### Q1: `no .pth file in ~/.kotoba/models/rvc/xxx`

检查目录下是否有 `.pth` 文件：

```bash
ls ~/.kotoba/models/rvc/xxx/
```

如果是空的或只有其他文件，可能是下载的仓库结构不对。试试指定子路径：

```bash
kotoba huggingface add rvc:user/repo:子目录名
```

### Q2: `ModuleNotFoundError: No module named 'torch'`

Python 环境没有 RVC 依赖。检查：

```bash
kotoba doctor   # 看 rvc_python 那行
```

如果显示的 python 路径不是你的 venv，手动指定：

```bash
export RVC_PYTHON=~/.kotoba/venvs/rvc/bin/python3
# 或永久写入配置
kotoba config set rvc.python ~/.kotoba/venvs/rvc/bin/python3
```

### Q3: 我想看系统依赖是否正常

```bash
kotoba doctor
```

输出示例：

```text
  ✓ database            level=N5, vocab=3, due=1
  ✓ voicevox_installed  ~/.kotoba/voicevox/run
  ○ voicevox_api        localhost:50021 unreachable
  ✓ audio_cache         5 cached files
  ✓ kokoro_model        installed
  ✓ rvc_python          infer-rvc-python available (~/.kotoba/venvs/rvc/bin/python3)
  ✓ rvc_models          2 installed
  ✓ disk_space          data dir: ~/.kotoba
```

### Q4: RVC 转换很慢

首次加载模型需要 10-30 秒（PyTorch 初始化 + 模型加载）。之后的调用会走音频缓存，直接返回。

如果想清除缓存重新生成：

```bash
rm ~/.kotoba/audio/こんにちは_kokoro_*.wav
kotoba play こんにちは
```

### Q5: 想切回普通声音（不用 RVC）

```bash
kotoba voice set kokoro:af_heart    # 只用 Kokoro
kotoba voice set voicevox:3         # 切回 VOICEVOX
```
