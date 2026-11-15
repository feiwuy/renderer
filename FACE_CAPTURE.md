# lcolonq/renderer 面捕机制核对

> 2026-11-14。依据作者公开 master 源码及随仓库 vendored 的 OpenSeeFace。
> 这只能说明这份公开版本，不代表作者今天直播的私有代码与配置完全相同。
> 本次只研究和记录：未安装依赖、未开启摄像头、未修改当前稳定版 Rust/shader。

## 1. 整条链路

```text
摄像头（或视频文件）
    ↓ OpenCV 读取画面
OpenSeeFace（ONNX 人脸检测/关键点模型）
    ↓ 估计头部姿态、眼睛张开程度、嘴部特征
本地 UDP 127.0.0.1:11573
    ↓ Rust tracking.rs 接收，更新共享状态
头部骨骼旋转 + blink/oh 两个表情权重
    ↓ vertex shader 蒙皮与 morph target
OpenGL 离屏渲染 → ASCII
    ↓
上游 st终端；本地稳定版 OBS浏览器源
```

**不是用真人视频直接贴在模型脸上，也不是把真人脸转成ASCII。**
画面里始终是VRM模型；摄像头只负责控制旋转和表情数据。

## 2. 作者怎么启动

上游 [launch.sh](https://github.com/lcolonq/renderer/blob/master/launch.sh) 的真实命令：

```sh
(cd deps/OpenSeeFace/ || exit; steam-run poetry run python facetracker.py -c 1 >/dev/null &)
../terminal/st -T "Colonq's Face" -e ./run.sh
```

含义：

- `-c 1` 是摄像头索引1，不能照抄成所有机器都适用；本机应该先选择正确摄像头。
- `&` 让面捕进程在后台运行，跟Rust渲染进程分开。
- `steam-run` 是作者NixOS环境的兼容启动方式，不是面捕算法的一部分。
- `poetry run` 选择Python依赖环境。
- `st` 只显示ASCII；不做面捕。

## 3. OpenSeeFace实际做什么

[facetracker.py发送逻辑](https://github.com/lcolonq/renderer/blob/master/deps/OpenSeeFace/facetracker.py#L246-L368)：

1. OpenCV读取摄像头帧；
2. ONNX Runtime运行检测和关键点模型；
3. 通过关键点及3D参考脸模型，用 `solvePnP` 估计头部姿态；
4. 根据眼睑、嘴唇关键点计算特征，再打包UDP。

发送的是数字，不是摄像头视频：时间戳、face ID、姿态四元数、眼睛状态、68个二维点、70个三维点及14个表情相关特征。
正常单脸数据包为 **1785 bytes**。

这份vendored脚本的默认参数：

| 参数 | 默认 |
|---|---|
| 摄像头 | 0（作者launch覆盖为1） |
| 采集目标帧率 | 24fps |
| 输入尺寸 | 640×360 |
| 面部关键点模型 | 3 |
| 最大面部数量 | 1 |
| 最大线程参数 | 1 |
| 眼球追踪 | 开启 |
| 发送地址/端口 | 127.0.0.1:11573 |

ONNX Runtime使用它当前可用的execution providers；安装普通CPU版时就是CPU推理。
并不是“renderer用了4060，所以面捕也自动用了4060”。
随仓库已有ONNX模型文件；后续安装Python运行依赖通常不必重新下载这些权重。

## 4. Rust只用了哪些信息

上游 [tracking.rs](https://github.com/lcolonq/renderer/blob/master/src/tracking.rs) 只更新：

| 接收数据 | 用法 |
|---|---|
| 头部姿态四元数 | 改变VRM的head骨骼旋转 |
| 两眼张开程度 | 取平均，再 `1 - average` 得到blink闭眼权重 |
| mouth_open（代码意图） | clamp到0~1，写入VRM的oh表情 |

上游 [主渲染循环](https://github.com/lcolonq/renderer/blob/master/src/main.rs#L119-L139) 将 `orientation.inverse()` 应用到head骨骼。
表情权重则被上传到 `expression_blink_weight` / `expression_oh_weight`，由vertex shader混合morph target。

因此这份公开版并没有完整实现：

- 每只眼睛独立眨眼；
- 根据嘴形识别aa/ih/ou/ee/oh五种发音；
- 眼球转动跟踪（OpenSeeFace虽发送点位，但Rust没有把它们应用到眼球骨骼）；
- 眉毛、微笑、悲伤等全部自动表情；
- 摄像头驱动手臂或身体动作。

> **本分支已经补上了前四项中的大部分**：14 个特征全部接上，11 个表情槽都有驱动源，
> 只剩"每只眼睛独立眨眼"没做（需要 `blinkLeft`/`blinkRight`，而顶点属性槽已用满）。
> 映射细节见本文第 10 节。

## 5. 核对到一个上游包偏移问题：嘴巴读错字段

将发送端、Unity参考接收器及Rust偏移计算逐字段对比，并离线构造单脸数据包验证：

| 数据 | 正确起始偏移（byte） |
|---|---:|
| 两眼张开程度 | 20 / 24 |
| 姿态四元数 | 33 |
| 14个特征区 | 1729 |
| mouth_open | **1777** |
| 完整单脸包长度 | 1785 |

上游Rust计算的 `mouth_offset` 是 **1769**，实际对应 `mouth_corner_updown_r`（右嘴角上下），不是mouth_open。
偏差是8字节。因此严格照抄上游接收器，模型张嘴可能跟着嘴角变化而不是实际开合。

这个问题也存在于当前稳定版接收代码，因为之前尚未接真实摄像头，没在真人数据中验证过它。
**本次已证实包布局问题，但没有修改稳定版。** 接面捕时应该先修接收器，再测试。

此外接收器目前还缺少：

- 使用 `recv_from` 返回的实际包长度做检查（现在读的是固定大缓冲区）；
- 过滤3D估计失败、NaN/无效/全零四元数；
- 多脸时挑选目标face ID；
- 丢脸或超时后回正/转待机；
- 正脸中性姿态校准及旋转平滑。

这些比“启动一个Python摄像头脚本”更重要，建议在新开发副本中完善。

## 6. 和当前版本的区别

当前仍有同样的UDP接收线程，入口已存在，OBS推流方式与面捕本身无冲突。
但当前加了程序生成的待机转头和眨眼：

```text
头部旋转 = 追踪旋转的inverse × 待机摆动
眨眼权重 = max(追踪blink, 待机blink)
```

所以真实面捕接入后若不关待机，角色会同时听从真人和正弦动作，不是纯面捕。
初次验证建议 `COLONQ_IDLE=0`；后续更好的行为是“有有效追踪数据时真人驱动，丢脸时才回待机”。

上游四元数处理还带固定轴翻转和Euler姿态补偿，这是按作者模型/坐标系调的，不是通用的正脸校准。
你现在用的是VRM 0.x适配模型，不能假定照抄补偿后它一定朝向正确。

## 7. 当前Python环境的障碍

本次只检测到：

- 系统Python：3.14.10；
- numpy、OpenCV(cv2)、Pillow已存在；
- onnxruntime未安装；
- poetry未安装。

vendored [pyproject.toml](https://github.com/lcolonq/renderer/blob/master/deps/OpenSeeFace/pyproject.toml) 要求 **Python >=3.7,<3.11**，是老项目依赖约束。
源码还使用 `np.float` 等旧NumPy API。

所以不能再说“直接poetry install就好了”。后续要么建立兼容的独立Python环境，
要么在新环境中升级/修补OpenSeeFace并实测；不要直接改系统Python或向系统环境随意pip安装。

## 8. 后续接入顺序（尚未执行）

1. 保留已创建的稳定版备份，在新副本中改面捕接收器。
2. 修mouth_open偏移、包长度/有效性过滤。
3. 经确认后建立独立Python环境并安装依赖。
4. 先只运行OpenSeeFace可视化，确认摄像头、眼睛、嘴部信号正常。
5. 在renderer中关待机、做正脸校准，再核对转头方向/眨眼/张嘴。
6. 最后接回现有OBS浏览器源，目标帧率仍24，不需要恢复可见SDL窗口。

将来环境准备好后，可参考以下启动参数（**不是现在可以直接运行的保证**）：

```bash
# 在合适的Python环境中，从deps/OpenSeeFace目录运行；摄像头索引按本机情况选择
python facetracker.py -c 0 -F 24 -W 640 -H 360 --faces 1 -m 1 -i 127.0.0.1 -p 11573 -v 1

# 在另一个终端中启动renderer；纯面捕初测关闭待机动画
COLONQ_IDLE=0 ./run-obs.sh
```

不需要把真人摄像头画面加入OBS；OBS继续显示VRM生成的ASCII。
本次没有启用摄像头，也没有更改稳定显示配置。

## 9. 延迟排查（转头发木 / 动作慢半拍）

先量一下，别猜。启动后 `grep "面捕包率" colonq.log`：

```
面捕包率 ≈ 24.0 Hz（间隔 41.7 ms），smooth=0.5 → 每包混合 0.50，平滑约引入 60 ms 延迟
```

整条链路的延迟大致是这些项相加，按影响从大到小：

| 环节 | 典型值 | 怎么降 |
|---|---|---|
| 手机 → Ubuntu Server 的 Wi-Fi MJPEG 推流 | 30–100 ms | 手机改 **USB 连接**（DroidCam USB 模式）；或降到 480p；或换成本地 USB 摄像头直连 Arch，整段两跳全免 |
| 面捕端采样：`-F 24` → 每帧 41.7 ms | 42 ms | Ubuntu 机器有余量就把 `-F 30`（或 60），`mjpeg_to_raw.py --fps` 同步改；同时降分辨率能省 CPU |
| MJPEG 缓冲积压（旧版隐患） | 0 → 数秒 | 已修：`mjpeg_to_raw.py` 改成读线程只保留最新帧，旧帧直接丢弃。看它 stderr 的「丢弃旧帧 N 个」应稳定在每秒 (源fps − 输出fps) 左右，不再持续上涨 |
| 平滑：`COLONQ_SMOOTH` | `0.5` → 60 ms（24 Hz 下） | `COLONQ_SMOOTH=0.2` → 约 26 ms；`0` → 0 ms（代价是抖动可见） |
| 渲染 + OBS 刷新：`COLONQ_FPS=24` | 42 ms | 渲染 30 fps、OBS 浏览器源也设 30 fps；两者不一致时按低的一方走 |
| OpenSeeFace 推理耗时 | 与上面 `-F` 相关 | 降低输入分辨率（640×360 → 480×270）、确保用 onnxruntime CPU 而不是软回退 |

按上面的账，如果先用 `COLONQ_SMOOTH=0.2`，再把面捕端和渲染端都提到 30 fps，
一般能省掉 60–90 ms；剩下的大头通常是 Wi-Fi 和手机摄像头本身，只能靠换传输方式解决。

> 注意：延迟不会来自渲染器的网络侧——推流是 SSE 异步推送，且 `COLONQ_PBO` 默认关闭（同步回读），
> 不会额外积一帧。

## 10. 14 个面捕特征 → 11 个表情槽（本分支已实现）

上游只接了 3 个字段（四元数、两眼张开、mouth_open）。本分支把包里**最后 56 字节的全部 14 个特征**都解析出来，映射到 11 个 VRM 表情槽。字段偏移与顺序见 `src/tracking.rs` 的 `F_*` 常量（14×4 = 56，1729+56 = 1785 = 整包长度）。

### 这些特征是什么

全部由 OpenSeeFace 的 `FeatureExtractor`（`deps/OpenSeeFace/tracker.py`）**每帧本来就在算**，本分支只是把它们用起来，面捕端 CPU 开销不变。

每个特征都是一个 `Feature` 对象输出的**自适应归一化量**：以使用者自己的中位数为 0，超出历史 min/max 时饱和到 ∓1，因此约 ∈[-1, 1] 且**正负代表方向**。`--max-feature-updates`（默认 900 秒）之后 min/max/median 冻结，不再漂移。

| # | 特征 | 由哪些关键点算 | 符号含义 |
|---|---|---|---|
| 1-2 | `eye_l` / `eye_r` | 眼睑上下间距 ÷ 鼻长 | 负 = 在闭眼 |
| 3, 6 | `eyebrow_steepness_l/r` | 眉线与眼线夹角（度，已扣头部倾斜） | 负 = 内侧下压（凶），正 = 内侧上挑（困り眉） |
| 4, 7 | `eyebrow_updown_l/r` | 眉内侧均值 − 鼻梁点 | 正 = 抬眉 |
| 5, 8 | `eyebrow_quirk_l/r` | 眉上最大起伏 ÷ 鼻长 | 只有幅度（≥0） |
| 9, 11 | `mouth_corner_updown_l/r` | 上唇线 − 嘴角 | 正 = 嘴角上扬 |
| 10, 12 | `mouth_corner_inout_l/r` | 嘴角离面部中线的距离 | 只有幅度（≥0） |
| 13 | `mouth_open` | 上下唇间距 ÷ 鼻长 | 正 = 比平时张 |
| 14 | `mouth_wide` | 左右嘴角横距 ÷ 脸宽 | 正 = 比平时宽，负 = 嘟嘴 |

### 映射到哪些槽

实现是 `src/tracking.rs` 的 `compute_weights`（纯函数，一个槽一行，注释写清了各自吃哪些特征）：

| 槽位 | 来源 |
|---|---|
| `blink` | `eye_l`/`eye_r`（+ 包里预算的 eye_blink，同源同值，交叉校验） |
| `surprised` | 抬眉 + 歪眉辅助 |
| `angry` | 眉内侧下压 + 压眉 + 歪眉 |
| `happy` | 嘴角**大幅**上扬 |
| `relaxed` | **同一信号的窄区间**——小微笑算放松，大微笑才升级成开心（两段梯形自然交叉，不会同时打满） |
| `sad` | 嘴角下垂 + 眉内侧上挑（困り眉） |
| `aa` | 张嘴大 + 嘴宽（あ） |
| `oh` | 张嘴大 + 嘴不宽（お）——**保留上游"只要张嘴就有 oh"的手感** |
| `ou` | 嘴比平时窄（嘟嘴）+ 少量张嘴（う） |
| `ee` | 微张 + 嘴宽（え） |
| `ih` | 半张 + 嘴宽（い） |

系数是第一版凭经验给的。调法：

```fish
# 看 14 个特征的真实数值（对着镜头做一遍惊讶/生气/笑/瘪嘴/啊咦呜哎哦）
set -x COLONQ_TRACK_DEBUG 2
./target/release/colonq

# 单独放大 / 关掉某个表情
set -x COLONQ_EXPR_GAINS "happy=1.4,aa=0.7,sad=0"
```

日志里每 100 个包会打一行 `特征 …` 和 `权重 happy=0.62 surprised=0.31`，可以直接对着数值调。

### 已知边界

- **11 个顶点属性槽已经用满**（`GL_MAX_VERTEX_ATTRIBS = 16`：0 位置 1 法线 2 UV 3 关节 4 权重，5..15 全是表情）。想再加表情（比如单眼 wink 用的 `blinkLeft`/`blinkRight`）必须换机制：要么做"槽位重映射"（用环境变量把某个槽改绑到别的 morph target），要么把 morph target 搬到纹理里用 `gl_VertexID` 取样。
- 模型里还有 13 个没有名字的 morph target（VRoid 内部库遗留），目前不可达。
- 表情幅度取决于**模型自身的 morph delta**。实测 `my_avatar.vrm` 的眨眼 delta 中位数只有 1.8 mm、最大 26 mm（按真实人体尺度导出），在 128×64 的网格下折算下来只有零点几个像素——所以"表情读不出来"很多时候不是权重的问题，而是**分辨率/幅度**的问题。要更明显，优先考虑：网格提到 `COLONQ_GRID=192x96`、把表情相关部位（眼/眉/嘴）单独分 part id 换字形、或对 morph delta 做整体放大。

### 模型里还有多少没用到的 morph target

`my_avatar.vrm` 的脸 mesh 一共有 **57 个 morph target**，VRM 只命名了 14 个（neutral + 5 情绪 + 5 元音 + blink/Left/Right）。按顶点位移的**区域**分类（离线统计，见下），其余的是：

| 下标 | 顶点数 | 平均位移 | 区域 / 特征 |
|---|---:|---:|---|
| 14, 15 | 925 / 925 | 0.0038 | **blinkR / blinkL**，左右分开（可以做单眼 wink） |
| 6, 7, 9, 10 | 72~88 | 0.0005~0.0031 | 眼周小形变（左右对称，量很小） |
| 12, 16, 17, 20, 21, 22 | 998~1420 | 0.0006~0.0039 | 眼型（闭眼/眯眼一类） |
| 18, 19 | 525 / 521 | 0.0060 | 眼型，**左右分开** |
| 23 | 462 | 0.0319 | 眼部最大的一处形变（眼型/高光隐藏一类） |
| 24 | 204 | 0.0107 | 眼下/颊 |
| 25~38 | 29~1772 | 0.0004~0.0190 | 嘴部形状（含左右分开的 37/38、上/下唇） |
| 39~43 | — | — | **aa / ih / ou / ee / oh**（已用） |
| 44~56 | 42~500 | 0.0006~0.0594 | 嘴部形状，其中 44 的位移是全模型最大的一处（0.059，比 aa 还大） |
| 8, 11 | 0 | 0 | **空 target**，绑了也不会动 |

（40 个非标准 target 里，8、11 是空的；其余的位移都集中在眼周和嘴部。）

想自己看某个 target 长什么样：

```fish
sh tools/morph_sheet.sh 23 33 44 45     # 渲染这几个到 tests/morph-sheet/*-view.png
LEVEL=3 sh tools/morph_sheet.sh 23      # 放大 3 倍看形变
```

或者直接用环境变量（`COLONQ_EXPR_MAP` 换绑 + `COLONQ_EXPR_FORCE` 钉死权重）：

```fish
set -x COLONQ_EXPR_MAP "happy=23"
set -x COLONQ_EXPR_FORCE "happy=1"
./run-obs.sh
```

单眼 wink 就是：`COLONQ_EXPR_MAP="relaxed=blinkLeft"`，然后让 `relaxed` 槽由左眼驱动
（需要小改 `compute_weights`，目前 `relaxed` 还是嘴角小微笑）。

### 还缺的能力

- **眼球注视**：模型有 `leftEye`/`rightEye` 骨骼，但 VRM 没有 `lookAt` 扩展，本渲染器也没驱动它们——眼睛永远是正视。OpenSeeFace 发的 68 个 2D 点/70 个 3D 点里其实没有瞳孔信息，只能靠头部姿态近似，价值有限。
- **真正的"自定义表情"**：受 16 个顶点属性上限限制，只能在 11 个槽位里换绑；要同时启用任意多个 target，得把 morph 数据搬到纹理里用 `gl_VertexID` 取样（方案见 README 的后续计划）。

### 额外 target 的实测可用性（权重=2，128×64，face 取景）

把每个 target 单独绑到 `happy` 槽、权重钉死 2，跟中性帧逐像素比对：

| 下标 | 差异像素 | 目视确认的样子 |
|---|---:|---|
| 3 | 1531 (18.7%) | `happy`（已用）——张开嘴的笑 |
| 2 | 1228 (15.0%) | `relaxed`（已用） |
| **17** | 1219 (14.9%) | **整只眼睛闭上、眼珠消失**（比 `blink` 更彻底；适合"笑到眯眼"） |
| 16 | 1090 (13.3%) | 眼型变化（眯/闭） |
| 4 | 999 (12.2%) | `sad`（已用） |
| 20 | 996 (12.2%) | 眼型变化 |
| 1 | 834 (10.2%) | `angry`（已用） |
| 12 | 811 (9.9%) | 眼型变化（较轻） |
| 5 | 787 (9.6%) | `surprised`（已用） |
| 15 / 14 | 763 / 755 (9.3%) | **blinkL / blinkR**——单眼 wink 直接可用 |
| 21 / 22 | 739 / 592 | 眼型变化 |
| 18 / 19 | 642 / 633 (7.8%) | **左右分开**的眼型 |
| 30 | 529 (6.5%) | 嘴部形状 |
| 26 / 29 / 33 | 325 / 288 / 312 | 嘴部形状（张嘴/咧嘴） |
| **23** | 247 (3.0%) | **眼珠变白/瞳色消失**（`><` 闭眼笑的关键） |
| 27 / 31 / 32 / 24 | 198 / 121 / 133 / 73 | 嘴部小形变 |
| **44~56** | **0~1 像素** | **渲染里完全看不出来**（大概率是口腔内部/牙齿几何），别浪费槽位 |
| 8、11 | — | 空 target |

结论：**真正值得接的是 17（彻底闭眼）、23（眼珠变白）、14/15（单眼 wink）**，三个都能让 ASCII 里的表情辨识度明显提升；
44~56 那 13 个可以直接放弃。

（生成对照图：`sh tools/morph_sheet.sh 17 23 14 15`；量化对比可以用 `COLONQ_EXPR_FORCE` + 逐像素 diff。）
