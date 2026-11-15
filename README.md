# renderer（lcolonq/renderer）本地部署版

这是 [lcolonq/renderer](https://github.com/lcolonq/renderer) 的本地可运行副本。
上游是 GPL-3.0，本目录的改动同样遵循 GPL-3.0（见 `LICENSE`）。

**已在本机（Arch Linux / RTX 4060 + AMD Phoenix1 / rustc 2026）编译通过并实跑出画面。**

---

## 它是什么

把 VRM 虚拟形象用 OpenGL 渲染到离屏 FBO，再降采样成终端里的彩色字符打印到 stdout。
上游版本还带面部追踪（OpenSeeFace → UDP）和 websocket 推流，本地版把推流去掉了。

数据流：

```
VRM 模型 ──gltf 载入──> VAO/骨骼/表情属性
                            │
                    OpenGL 渲染到 64×64 FBO
                            │  GetTextureImage 回读
                    CPU 逐像素 → 调色板替换（默认换成 "colonq" 字样）→ ANSI 字符
                            │
                      stdout（终端里看到画面）
```

## 环境要求

| 依赖 | 说明 |
|---|---|
| `sdl2`（Arch 上是 `sdl2-compat`） | 建窗口 + GL 上下文，`pkg-config --modversion sdl2` 有输出即可 |
| `mpv` | libmpv 运行库（`/usr/lib/libmpv.so`），视频贴图功能要用 |
| 支持 OpenGL 4.6 的显卡 | 着色器本身只写 `#version 330`，但生成的绑定是 4.6 级别 |
| Rust 工具链 | `cargo` |
| 显示环境 | 必须有 `DISPLAY`/`WAYLAND_DISPLAY`，SDL 要开真窗口 |

**不需要** `SDL2_image`（已改掉）、不需要 nix、不需要 clang/libclang（已改用预生成绑定）。

## 构建与运行

```bash
./run-local.sh          # 编译 + 运行
# 或者
cargo build --release && ./target/release/colonq
```

运行后：

- 会弹出一个 SDL 窗口，**但真正的画面在终端里**（一堆 ANSI 彩色字符）
- 退出：关掉 SDL 窗口（或 Ctrl+C / `pkill colonq`）
- 日志写在当前目录的 `colonq.log`（DEBUG 级别，含模型解析细节）

## 资源

`assets/` 下需要：

| 文件 | 说明 |
|---|---|
| `shader.vert` / `shader.frag` | 本项目自己写的 shader（上游没公开），带蒙皮 + 表情形变 + MToon 风格卡通明暗 |
| `my_avatar.vrm` ⭐默认 | 你从 Downloads 里的 `8319220030028561131.vrm`（**VRM 0.x**，VRoid Studio 1.27 导出）用 `tools/vrm0_to_vrm1_min.py` 转成的 VRM 1.0 壳。注意原模型 `meta.licenseName = "Other"`，是第三方作品，仅供本地自用 |
| `threevrm_sample.vrm` | `VRM1_Constraint_Twist_Sample.vrm`，来自 [pixiv/three-vrm examples](https://github.com/pixiv/three-vrm/tree/master/packages/three-vrm/examples/models)，**VRM Public License 1.0，© 2022 pixiv Inc.**。11 个表情一个不缺 |
| `seed_san.vrm` | Seed-san，来自 [vrm-specification samples](https://github.com/vrm-c/vrm-specification/tree/master/samples/Seed-san)，同为 VRM Public License 1.0，模型作者 VirtualCast, Inc. |

**换模型**：任何 **VRM 1.0**（必须带 `VRMC_vrm` 扩展）都行，放进 `assets/` 再用环境变量指定，不用改代码：

```bash
COLONQ_MODEL=seed_san.vrm ./target/release/colonq
```

模型缺某个表情不会崩（只会少一个形变），缺 `head`/`leftUpperArm`/`rightUpperArm` 骨骼也会跳过。

资源目录默认是 `<crate>/assets`，可用环境变量 `COLONQ_ASSETS=/path/to/assets` 覆盖。

## VRM 0.x 模型怎么用（转换）

本项目只认 **VRM 1.0**（`VRMC_vrm` 扩展），而 VRoid Studio 1.x 默认导出的是 **VRM 0.x**。转换用自带的小工具：

```bash
python3 tools/vrm0_to_vrm1_min.py 你的模型.vrm assets/新名字.vrm
```

它只重写 GLB 里的 JSON 扩展（`extensions.VRM` → `extensions.VRMC_vrm`），**二进制数据完全不动**，因为渲染器只用这两样东西：

- `humanoid.humanBones`（骨骼 → node 索引）
- `expressions.preset`（每个表情的 morphTargetBinds）

同时会做两件事：VRM 0.x 的表情预设名映射到 1.0（`a→aa`、`joy→happy`、`sorrow→sad`、`fun→relaxed`、`unknown→surprised`…），以及把 0~100 的表情权重归一化到 0~1。

> ⚠️ 这不是完整的 VRM 1.0 转换：**meta / springBone / MToon 材质参数都没迁移**（本渲染器不读，所以不影响显示）。要让别的软件（VSeeFace、VRChat 等）也能用，请改用 Blender + [VRM Add-on](https://extensions.blender.org/add-ons/vrm/) 做标准转换。

**朝向是自动识别的**：脚本保留了旧的 `VRM` 扩展作为标记，`main.rs` 启动时会检查它 —— 带旧扩展的（0.x 转换件）朝向是 -Z，默认 `yaw=0`；原生 VRM 1.0 朝 +Z，默认 `yaw=180°`。手动覆盖仍然用 `COLONQ_YAW`。

## 构建与检查

需要 Rust **1.77+**（用了 C 字符串字面量 `c"..."`）。

```bash
make check        # fmt + clippy -D warnings + cargo test + viewer 测试 + 渲染基准
make check-fast   # 跳过渲染基准（没有 GPU / 没放模型时用这个）
make golden-update# 确认画面改动符合预期后，刷新渲染基准
```

- **渲染基准**（`tools/golden.sh`）：在 `COLONQ_IDLE=0` + 固定 GRID/MODEL 下渲染第 120 帧并比
  sha256。基准文件里同时记录了生成它的 **GL renderer 指纹**：
  - 同一渲染器 + 哈希不同 → **FAIL**（这是真回归）
  - 渲染器不同（换机器/升级驱动）→ **SKIP** 并提示 `--update`，不会给你假的 FAIL
  - 缺模型或缺 GL 环境 → **SKIP** 返回 0
  所以**每台机器需要各自 `--update` 一次**：确认画面正确后跑，然后提交 `tests/golden/`。
- **CI**：`.github/workflows/ci.yml` 跑上面除渲染基准外的全部检查，警告即失败。
- 依赖版本已在 `Cargo.toml` 固定（不再是 `"*"`），升级时改版本号 + 跑 `make check`。

## 环境变量

**开关类变量的统一取值**（`COLONQ_PARTS`、`COLONQ_IDLE`、`COLONQ_FIT`、`COLONQ_PROFILE`、`COLONQ_NO_MPV`、`COLONQ_DUMP_*` 等）：

| 写法 | 含义 |
|---|---|
| `1` / `true` / `on` / `yes`（或任何其他非空值） | 打开 |
| `0` / `false` / `off` / `no` | 关闭 |
| 未设置，或设成空字符串 | 用该变量的默认值 |

> 也就是说 `COLONQ_PARTS=0` 与 `COLONQ_PARTS=` 都是**关闭**。早期版本里这两种写法反而会打开开关，是已修复的 bug。

**排查"改了却没生效"**：启动时日志（`colonq.log`）会先打印一行 `配置: 共 N 个 COLONQ_* 变量生效`，后面逐条列出名字和值；数值/三元组解析失败会打 `WARN 环境变量 XXX 的值 "..." 无法解析…`，不会再静默回落默认值。

| 变量 | 默认 | 作用 |
|---|---|---|
| `COLONQ_VIEW` | `head` | 取景预设：`head`=头部+肩胸、`face`=面部特写（更近）、`bust`=半身（自动开胸部裁切）、`full`=全身 |
| `COLONQ_IDLE` | 开（run-obs 为 `0`） | 待机动画（转头/点头/呼吸/眨眼），面捕在线时自动让位 |
| `COLONQ_IDLE_SPEED` | `1.0` | 待机动画速度倍率 |
| `COLONQ_T0` | `0.0` | 待机动画相位偏移（调试/对照用） |
| `COLONQ_TERM_EVERY` | `1` | 每几帧重画一次终端：`1` = 每帧（刷新率 = 渲染帧率），`2` = 一半 |
| `COLONQ_EXPOSURE` | `0.85` | 亮度倍率 |
| `COLONQ_SATURATION` | `1.30` | 饱和度（1.0 = 原样） |
| `COLONQ_CONTRAST` | `1.0` | 对比度，围绕线性中灰 0.18 拉伸（1.0 = 不变） |
| `COLONQ_TOONY` | `0.95` | 明暗分界硬度，越接近 1 越像二分色块 |
| `COLONQ_SHIFT` | `-0.10` | 整体明暗偏移，负值变暗、正值变亮 |
| `COLONQ_SHADE_TINT` | `0.82,0.80,0.88` | 暗部色调（乘在贴图上，越低对比越强） |
| `COLONQ_SHADE_LIFT` | `0.0` | 暗部整体提亮 |
| `COLONQ_RIM` | `0.03` | 边缘光强度（过大就发白） |
| `COLONQ_SKIN_TINT` | `1.22,1.18,1.20` | **皮肤/脸部单独染色**（只作用于 FACE/SKIN 材质），数值越大越白 |
| `COLONQ_SKIN_SAT` | `1.00` | 皮肤饱和度倍率（越低越接近纯白） |
| `COLONQ_OUTLINE` | `0.004` | 描边外扩量（世界单位），`0` = 关闭（128×64 网格下 0.004 ≈ 1 像素） |
| `COLONQ_OUTLINE_GLOW` | `0.6` | 描边发光强度（0=普通描边，>0 颜色提亮产生发光感） |
| `COLONQ_OUTLINE_COLOR` | `0.85,0.88,0.95` | 描边颜色 |
| `COLONQ_PARTS` | 未设（run-obs 同） | 启用按部位分字符表 |
| `COLONQ_WORD` | `lcolonq` | 默认填充词（人物内部循环显示） |
| `COLONQ_PART_WORDS` | run-obs 为 `2=.,4=#` | 各部位字符表 `编号=字符`，逗号分隔；没写的部位用默认词 |
| `COLONQ_NO_MPV` | 未设 | 完全不创建 libmpv 实例（省 46 个线程，视频填充不可用） |
| `COLONQ_CLIP` | `none` | 水平裁剪面：`chest`=胸口以上、数值=世界 Y、`none`=不裁 |
| `COLONQ_MAX_JOINTS` | 自动（读模型骨骼数） | shader 里骨骼矩阵数组大小；NVIDIA 上限 256（1024 常量寄存器） |
| `COLONQ_TEX_MAX` | `0`（不限制） | 贴图最长边上限，超过就降采样（`512` 可省一半内存） |
| `COLONQ_LOG` | `info` | 日志级别：`debug`（排查用）/`info`/`warn`/`error`/`off`。`colonq.log` 超过 8 MB 时启动会先滚成 `colonq.log.1`，保留上一次运行的日志 |
| `COLONQ_PROFILE` | 未设 | 每 60 帧输出一次分层耗时（avatar/blit/swap/终端）到日志 |
| `COLONQ_BGCOLOR` | `000000`（run-obs 同） | 背景色（`RRGGBB` 或 `#RRGGBB`） |
| `COLONQ_ASSETS` | `<crate>/assets` | 资源目录 |
| `COLONQ_MODEL` | `my_avatar.vrm` | 用哪个 VRM 文件（相对 assets 目录） |
| `COLONQ_YAW` | 自动 | 自动判断：带 VRM 0.x 旧扩展的用 `0`，原生 VRM 1.0 用 `3.1416`（朝 +Z，相机在 -Z 侧） |
| `COLONQ_SCALE` | `1.0` | 模型缩放 |
| `COLONQ_OFFSET` | `0,0,0` | 模型位移 `x,y,z` |
| `COLONQ_CAM` | head: `0,1.377,-0.48`／face: `0,1.40,-0.27`／bust: `0,1.29,-0.70`／full: `0,0.82,-2.55` | 相机位置 `x,y,z` |
| `COLONQ_LOOK` | head: `0,0,1`／full: `0,0.05,1` | 视线方向（相对相机） |
| `COLONQ_ARM_POSE` | `-0.80` | 上臂旋转角（弧度）：`-0.80` ≈ 46°（手臂下垂外斜，A-pose）；`-1.40` ≈ 80°（贴身下垂）；`0` = T-pose；正值抬手 |
| `COLONQ_SERVE` | 未设 | 本地推流端口（如 `8080`），OBS 用浏览器源填 `http://127.0.0.1:8080` |
| `COLONQ_FPS` | `24` | 目标帧率；`0` = 不限速 |
| `COLONQ_VSYNC` | 关 | GL 上下文创建后设置垂直同步；只影响开启预览时的 swap |
| `COLONQ_PREVIEW` | 推流时关，普通终端模式开 | `0` 隐藏 colonq 预览且不调用 blit/swap；`1` 显示预览 |
| `COLONQ_FIT` | 开 | 只对终端输出等比缩放，不改变内部/OBS 网格（`0` = 关闭） |
| `COLONQ_SIZE` | 未设；run-obs为385×350 | 像素显示提示，例如 `385x350`；实际终端行列仍是上限 |
| `COLONQ_ROT_OFFSET` | `0,-10,360` | 头部姿态三轴静态偏移（度），`roll,pitch,yaw`（实测确认的正脸组合） |
| `COLONQ_PITCH_SIGN` | `1` | 原始四元数 x 分量符号（旧参数，被 QSIGN 覆盖） |
| `COLONQ_PITCH_FLIP` | `1` | 真人点头方向与模型相反时设 `1`（最终姿态欧拉分解后只翻转俯仰） |
| `COLONQ_QSIGN` | `1,1,1` | 四元数三分量符号 `x,y,z`（实测确认 `1,1,1` 正确） |
| `COLONQ_PIXELS` | 关 | `1`：读取终端像素大小来估算输出容量 |
| `COLONQ_CELL_W` / `COLONQ_CELL_H` | `7` / `14` | 像素模式的单格宽高估算 |
| `COLONQ_GRID` | `128x64` | 内部渲染/推流网格；小终端仅缩放输出，不改变它 |
| `COLONQ_NO_TERM` | 未设 | 不写 ANSI、无需 TTY；ASCII 推流继续更新 |
| `| `COLONQ_FLIP` | `1` | 头部旋转补偿翻转：`0`=无、`1`=Z180°、`2`=Y180°、`3`=X180°、`4`=Z+Y、`5`=X+Y、`6`=X+Z |
| `COLONQ_TRACK_BIND` | 程序 `127.0.0.1:11573`；`run-obs.sh` 为 `0.0.0.0:11573` | 面捕 UDP 绑定地址。**面捕跑在局域网另一台机器时必须是 `0.0.0.0:11573`**（否则一个字节都收不到），只有面捕和渲染同机时才用 `127.0.0.1:11573`；共享网络里请用防火墙只放行面捕机 IP |
| `COLONQ_TRACK_GAIN` | `1` | 面捕头部旋转增益（>1 夸张动作，<1 减小） |
| `COLONQ_EXPR_GAIN` | `1` | 眨眼/张嘴幅度增益 |
| `COLONQ_SMOOTH` | `0.5` | 面捕平滑 0~0.95（越大越平滑、延迟越高）。**延迟偏高时先降这个**：24 包/秒下 `0.5` ≈ 60 ms、`0.2` ≈ 26 ms、`0` = 不平滑。排查见 `FACE_CAPTURE.md` 第 9 节 |
| `COLONQ_EXPR_GAIN` | `1` | 面捕表情权重增益（blink/oh 放大） |
| `COLONQ_PBO` | 未设（=同步回读） | `1` 启用异步双 PBO 回读（真 GPU 上可省掉每帧等 GPU 画完的时间）；任何异常会自动退回同步，不会黑屏 |
| `COLONQ_DUMP_PNG` | 未设 | 保存 `frame64.png`（原始）和 `frame-view.png`（字符比例视图） |
| `COLONQ_DUMP_EXIT` | 未设 | 存完图立刻退出（`tools/golden.sh` 用它，省得靠 timeout 兜底） |
| `COLONQ_DUMP_AT` | `120` | 存图间隔帧数，调小可快速出图（如 25） |
| `COLONQ_DUMP_SEQ` | 未设 | 存图带帧号（`frame-000012.png`），方便合成 GIF |
| `COLONQ_DUMP_FRAMES` | 未设 | 每 12 帧把序列化帧写到 `frame-*.bin`（原本是推流用的数据） |

调模型摆位的常用组合：

```bash
# 看单帧静态图，分辨率放大，不刷终端
COLONQ_DUMP_PNG=1 COLONQ_GRID=512x256 COLONQ_NO_TERM=1 timeout 8 ./target/release/colonq
```

> 注意：`frame64.png` 是 OpenGL 回读的原始数据，**上下是反的**，看图前要翻转。

## 动态显示（画面怎么动起来）

**默认目标为 24fps，终端每帧更新一次。** 实际帧率取决于渲染耗时、终端吞吐和图形后端，不能仅凭目标值保证。
所以"看着不动"并不是没刷新，而是**没有动作来源**。现在有两个来源：

| 来源 | 状态 | 说明 |
|---|---|---|
| **内置待机动画** | ✅ 默认开启 | 头部左右转 + 轻微点头、脊柱呼吸起伏、每 ~3.7 秒眨一次眼。纯程序生成，不需要摄像头 |
| **面部追踪（OpenSeeFace）** | 未接线 | 需要 `poetry install` + `facetracker.py`，数据走 UDP 11573，和 `tracking.rs` 对齐 |

调参示例：

```bash
COLONQ_TERM_EVERY=1 ./run-local.sh          # 与目标帧率一致，默认 24 次/秒
COLONQ_IDLE_SPEED=2.0 ./run-local.sh        # 动画快一倍
COLONQ_IDLE=0 ./run-local.sh                # 关掉待机动画（纯静态）
```

顺带做掉的两个小优化：

- 去掉了每帧的整屏清除 `\x1b[2J`（64×64 个格子每帧都会重写，清屏只会闪），重画改为"光标归位 + 覆盖写"
- 实测各阶段耗时：avatar 3ms + blit 2ms + swap 0.2ms + 终端 0.1ms，合计约 5.3ms/帧，离 60fps 的 16.7ms 还有余量

## 颜色不对的原因与调法

原版跑出来发白/发灰，根因有三个，现在都修了：

| 问题 | 说明 | 修法 |
|---|---|---|
| **gamma 被忽略** | 贴图是 sRGB，之前当线性数据用，光照也在 sRGB 空间算，中间调会被抬高 | 贴图改用 `GL_SRGB8_ALPHA8` 上传（采样自动转线性）→ 光照在线性空间算 → 输出前手动 `pow(1/2.2)` 转回 sRGB |
| **`baseColorFactor` 没上传** | glTF 材质自带的基础色系数被完全忽略（26 个材质是纯白，1 个 0.936） | `vrm.rs` 每个图元上传 `base_color` uniform，shader 里 `texture * base_color` |
| **shade/rim 参数过头** | 原实现 `SHADE_TINT` 偏紫、`SHADING_SHIFT=0.18` 把整体推亮、`RIM=0.22` 糊一层白 | 改成中性暗部色 + shift 0.02 + rim 0.06，且全部变成 uniform 可调 |

调法（都不需要改代码）：

```bash
# 更饱和、更暖
COLONQ_SATURATION=1.5 COLONQ_EXPOSURE=1.05 ./run-local.sh
# 更暗、明暗更硬（更像 MToon 二分）
COLONQ_EXPOSURE=0.85 COLONQ_TOONY=0.95 COLONQ_SHIFT=-0.10 COLONQ_RIM=0.03 ./run-local.sh
# 暗部偏蓝的夜景色调
COLONQ_SHADE_TINT="0.72,0.76,0.95" ./run-local.sh
```

## 帧率

默认目标 **24fps**（`COLONQ_FPS` 可改，`0` = 不限速）。节拍由程序控制；终端/网络写入不阻塞它。但操作系统调度、GPU/合成器 swap 阻塞仍可能降低实际帧率，需查看日志验证。

为此做了三件事：

1. 主循环改成"累计时间算步数 + 睡到下一帧"，不到下一帧就 `sleep`，不空转（原来 60fps 时是忙等）
2. **关闭 vsync**（`gl_set_swap_interval(0)`），否则会被显示器刷新率绑住；要开会 `COLONQ_VSYNC=1`
3. 待机动画、`float_time` 改用**墙钟时间**，帧率变化不影响动作速度

实测（软件渲染环境下，看相对值）：

| `COLONQ_FPS` | 实际帧率 | CPU | 终端带宽 |
|---|---|---|---|
| **24（默认）** | 23.6 fps | **113%** | 105 KB/s |
| 60 | 59.5 fps | 148% | 261 KB/s |
| 0（不限） | ~60 fps（受渲染耗时限制） | 149% | 280 KB/s |

也就是**锁 24fps 大约省 1/4 的 CPU**。

**终端刷新率默认跟渲染帧率 1:1**（`COLONQ_TERM_EVERY=1`），24fps 下就是 24 次/秒：

```
实际 fps=23.9   终端刷新=23.9/s
```

终端刷新的上限就是渲染帧率 —— 想要更顺只有两条路：

```bash
COLONQ_FPS=30 ./run-local.sh                    # 30fps 渲染 + 30 次/秒终端刷新（实测 30.4/s）
COLONQ_FPS=60 COLONQ_TERM_EVERY=1 ./run-local.sh  # 60fps（CPU 约 148%）
COLONQ_TERM_EVERY=2 ./run-local.sh              # 反过来省带宽：12 次/秒，CPU 几乎不变
```

```bash
COLONQ_FPS=24 ./run-local.sh     # 默认
COLONQ_FPS=30 ./run-local.sh     # 稍流畅
COLONQ_FPS=0 ./run-local.sh      # 不限速，能跑多快跑多快
COLONQ_VSYNC=1 ./run-local.sh    # 打开垂直同步（会被绑到显示器刷新率）
```

> 若因不可见 SDL 窗口的交换而停顿，请用 `run-obs.sh`；它不执行窗口 swap，不必修改合成器窗口规则。

## 面捕（Ubuntu Server + 手机摄像头）

面捕链路与用法详见 [UBUNTU_FACECAP.md](<UBUNTU_FACECAP.md>)。要点：

- 手机（DroidCam App）→ Wi-Fi 视频流 → Ubuntu Server 上 OpenSeeFace → UDP → renderer。
- 接收器已修正上游的 **mouth_open 偏移错误**（正确偏移 1777，上游读到 1769 的右嘴角特征），并加了包长度/数值校验（无效/NaN 包丢弃）；见 `src/tracking.rs` 及其 3 个测试。
- **面捕在线时待机动画自动让位**；停发数据约 2 秒后回退到待机转头/眨眼（`tracking.rs` 的 `TRACKING_TIMEOUT`）。
- stdout 不是 TTY（重定向/直接跑二进制）时自动只推流，不再 panic。
- 跨机接收：`COLONQ_TRACK_BIND=0.0.0.0:11573`（配合防火墙只对服务器 IP 放行）。
- **面捕在线时待机动画自动让位**；停发数据约 2 秒后回退到待机转头/眨眼（`tracking.rs` 的 `TRACKING_TIMEOUT`）。
- 服务器侧部署步骤、Python 3.10 环境和试跑命令都在部署文档里；自检脚本：`./tools/facecap_selfcheck.sh`。
- 面捕程序本身约占 **1 个 CPU 核心**（官方实测 model 3 约 44fps、单脸；跑 24fps 用不到一整核），内存约 300~500MB（含 ONNX Runtime）；你的 R7-4800U 8 核完全放得下。

## 面捕姿态校准（实测结论）

真人面捕接入后，头部旋转的补偿链路经过多轮实测校准，**当前默认值即为验证过的正确组合**：

| 参数 | 值 | 作用 |
|---|---|---|
| `COLONQ_FLIP=0` | 无组合翻转 | 静态测试的 Z 轴翻转对真人数据是错的 |
| `COLONQ_ROT_OFFSET="0,-10,360"` | 三轴偏移 | yaw 加 360°（等效 0°）+ pitch 下压 10° |
| `COLONQ_QSIGN="1,1,1"` | 四元数分量不翻 | 单翻任何分量都会连带倒置（q/-q 同旋转） |
| `COLONQ_PITCH_FLIP=1` | 俯仰取反 | **关键**：真人点头方向与模型相反，在最终姿态上欧拉分解后只翻转俯仰 |

校准过程中的现象记录（供以后参考）：

- 静态测试包（固定四元数）验证的 Z 轴翻转，对真人动态数据不成立——测试包姿态与真实姿态的补偿轴不同
- 单翻四元数分量（QSIGN 任一轴）会导致其他轴连带错乱（四元数 q/-q 同旋转，单分量翻转=改变整个旋转语义）
- 正确方法：**补偿完成后做欧拉分解，只取反俯仰分量再重组**——保证只影响点头方向

排查顺序（以后遇到类似问题）：
1. 静态正脸对不对 → `COLONQ_ROT_OFFSET` 三轴偏移
2. 点头方向 → `COLONQ_PITCH_FLIP`
3. 摇头方向 → `COLONQ_QSIGN` 或 ROT_OFFSET yaw 偏移
4. 幅度 → `COLONQ_TRACK_GAIN` / `COLONQ_EXPR_GAIN`

## 本地推流与 385×350 小窗口

内部渲染继续使用默认 **128×64**。模型、取景、柔和配色、纯黑背景、皮肤染色和手臂姿态都保留。

### OBS 浏览器源（推荐）

```bash
./run-obs.sh
# 等价于：COLONQ_SERVE=8080 COLONQ_NO_TERM=1 COLONQ_PREVIEW=0 COLONQ_PROFILE=1 ./run-local.sh
```

这个启动方式**不会弹出 colonq 预览窗口**，这是为了不依赖可见窗口的 frame callback。
GL 仍然渲染到离屏 FBO，CPU 回读、ASCII 转换和本地推流照常进行。
仍需本机的图形会话/显卡驱动；这不是无显示服务器的通用 headless 实现。

OBS 浏览器源设置：

- URL：`http://127.0.0.1:8080/?width=385&height=350`
- 源宽度 **385**、源高度 **350**，自定义帧率 **24**。
- 不要勾选「源不可见时关闭」。
- 更换运行程序后重启 renderer，然后刷新浏览器源缓存。

128×64 对应正方形显示区域：在 385×350 中内容为 **350×350**，左右各 **17.5px** 留边，人物不拉宽。
页面没有尺寸参数时默认用 385×350 作为显示边界；可用 URL width/height 覆盖，内部网格不变。

### colonq 窗口不在界面中时卡顿：残留的 swap 路径（本次修正）

之前独立终端线程只解决了 stdout 阻塞，但主循环仍然无条件执行
`gl_swap_window()`。不可见的 Wayland/EGL surface 可能使这一步等待合成器，ASCII 的生产也会跟着停。
此外旧代码在创建 GL 上下文**之前**设置 swap interval、并忽略错误，不能保证 vsync 真正关闭。

现在推流默认：

1. 创建隐藏的 GL 窗口，渲染仅使用离屏 FBO；
2. **不执行预览 blit，也不执行窗口 swap**；
3. 在 GL 上下文创建后设置 swap interval，并记录设置失败信息；
4. profile 统计真实绘制帧与 ASCII 生成次数，不再把补跑的模拟步数当成绘制帧率。

要临时看 SDL 预览，可设置 `COLONQ_PREVIEW=1`；这会重新启用窗口交换，因此不建议用于排查不可见窗口卡顿。

### 卡顿诊断

先确认日志里有：

```
SDL 预览: 隐藏；窗口交换: 禁用（推流不依赖窗口可见性）
profile 实际 fps=24.0 ASCII生成=24.0/s ... swap 0.0
```

不用启动第二个 renderer，直接测正在运行的服务：

```bash
python3 tools/check_stream.py
```

它会输出实际接收帧率、帧间隔 P95/最大值、序号跳过量、平均帧龄。
浏览器 URL 临时加 `&debug=1`：

```
http://127.0.0.1:8080/?width=385&height=350&debug=1
```

左上角显示浏览器的接收帧率、绘制次数、JS 绘制耗时和帧龄。去掉 debug 参数即可隐藏。

- 源端生成低于24：检查日志的 avatar/回读/其他耗时和 GPU 后端。
- 源端稳定24但浏览器接收/绘制低：检查浏览器源是否被关闭、页面有没有报错、源帧率设置和 CEF 吞吐。
- 接收/绘制都接近24但录制仍卡：检查 OBS「统计」中的渲染延迟/编码延迟；该问题不能用 shader 调色解决。

本次本机验证（开发环境的软件渲染，不是用户 OBS 的性能保证）：无可见 colonq 窗口、零 swap，
发布约24fps、接收24.21fps，采样最大间隔44.4ms、跳过序号0；Firefox 385×350 实际绘制持续更新。

### foot 终端（385×350 像素）

```bash
COLONQ_SIZE=385x350 ./run-local.sh
```

代码会以 **实际终端列数/行数**为上限，`COLONQ_SIZE` 只是像素估算提示。
例如字体格约 7×14px、终端报告55×25字符时，安全输出区是 **54×24**，人物等比缩放成 **48×24**。
终端显示会牺牲细节，但内部/OBS 推流仍是 **128×64**。若想在这样的小尺寸保留高密度 ASCII，请用上面的浏览器源，
或在 foot 内降低字号；程序不能让一列终端格同时容纳多个可独立定位的字符。

`COLONQ_PIXELS=1` 可读取终端报告的像素大小；不支持像素报告时自动退回字符列数。
`COLONQ_CELL_W` / `COLONQ_CELL_H` 是估算单格宽高，默认 7/14px。实际列数总是优先，绝不会通过乘大网格“挤”更多字符。

### 端点

| 地址 | 用途 |
|---|---|
| `/` | Canvas ASCII 显示页面 |
| `/stream` | SSE JSON 推帧，断线自动重连 |
| `/snapshot` | 最新完整帧的 JSON（含序号、网格、精确 RGB 调色板、稀疏字符坐标） |
| `/frame` | 兼容的 HTML 调试帧，保留行首和行尾空格 |
| `/size` | 内部网格尺寸，例如 `128x64` |

**不再使用之前的粗量化**：旧代码实际只有每通道 4 个色阶（不是 4 bit），且 u8 乘法会溢出。
现在按原始 RGB 去重生成调色板，用 Canvas 绘制，不重建几千个 span。字符的前导空白保留在坐标里，不再导致脸部左移/错位。

不设 `COLONQ_SERVE` 就不监听端口，服务只绑定 `127.0.0.1`。

## 终端阻塞与 SDL 窗口交换是两个问题

独立终端写线程解决的是 **PTY 背压**：终端来不及读时丢弃 ANSI 帧，不阻塞渲染。
这不能保证 SDL 的 `swap` 不等待合成器，也不能强迫终端模拟器在不可见时继续产出屏幕内容。
OBS 推荐使用上面的浏览器源 + 无预览推流模式，而不是抓取不可见的终端/SDL窗口。

## 终端尺寸：小窗口不再折行

背景空格也占终端列。固定输出 128 字符的行，在不足 128 列的终端会折行。
现在只缩放 **ANSI 输出**，而不是重建内部网格：每次输出读取终端容量，保留一列/一行安全边界，
按原网格比例缩放并居中，使用绝对行定位；不在最后一行发送换行，避免滚屏。
缩放时擦除旧的行尾/底部内容，防止留下上一尺寸的残影。

```bash
COLONQ_FIT=0 ./run-local.sh      # 强制原尺寸；仅在终端足够大时使用
```

这样缩小 foot 不会降低 OBS 的 128×64 精度，也不会不停重建 FBO 泄漏 GL 资源。
`COLONQ_NO_TERM=1` 时不进入 raw mode、不向终端写帧，但继续生成 ASCII 并供推流客户端读取。

## 分辨率：为什么是 128×64

原作者是 **64×64 像素**，但他每个像素往终端里塞 **2 个字符**，所以终端上是 128 列宽。我们的做法更直接：

| | 原版 | 现在（默认，作者同款占位） | 可选更高 |
|---|---|---|---|
| 渲染分辨 | 64×64 | **128×64** | 128×96 |
| 字符网格 | 64×64 格，每格 2 字符 | **128×64 格，1 字符/格** | 128×96 格，1 字符/格 |
| 终端占位 | 128 列 × 64 行 | **128 列 × 64 行** | 128 列 × 96 行 |
| 像素数 | 4096 | **8192** | 12288 |
| 实测帧率 | 60fps | 60.5fps | 59.6fps |

默认用 128×64：终端占位和原作者完全一致（128 列 × 64 行），但横向采样是他的 2 倍。想更清晰再手动上 128×96。

终端占位完全一样，但横向采样密度翻倍 —— 头发丝、睫毛、发箍边这些细节明显清楚了。

改造时踩到原作者代码里的两个**只在正方形缓冲下才成立**的坑：

1. `get_pixel` 的索引公式是 `invx * 4 * h + invy * 4` —— 乘的是 `h`，缓冲一变宽就错位（应该是乘 `w`）
2. `render_term` 用 `x` 遍历宽度、`y` 遍历高度，但写格子时是 `t.write(y, x)` —— 也就是 `x` 其实是**行**、`y` 是**列**；缓冲非正方形时循环边界得对调

另外终端光栅化的投影矩阵要按字符长宽比（约 1:2）校正：`aspect = (宽 × 0.5) / 高`，否则人物会被横向拉宽一倍。

```bash
COLONQ_GRID=64x64  ./run-local.sh    # 降低采样密度（当前版本仍然每格一个字符）
COLONQ_GRID=128x96 ./run-local.sh    # 纵向再提一级（终端要 128 列 × 96 行）
COLONQ_GRID=160x96 ./run-local.sh    # 更狠：终端得放得下 160 列 × 96 行
```

窗口大小不够时，`stty` 报的列数/行数会决定实际显示区域，**装不下就会被截断或折行**。查一下当前终端尺寸：

```bash
echo "$COLUMNS x $LINES"     # 或用 `tput cols` / `tput lines`
```

> 小提示：`COLONQ_DUMP_PNG=1` 除了存原始帧，还会存一张 `frame-view.png`（纵向拉一倍），那是屏幕上的真实比例。

## 资源占用（实测）

用 `./tools/measure.sh <网格> [秒数]` 可以自己复测。下面是本机实测（**注意：这台机器上跑的是 llvmpipe 软件渲染**，原因见下）：

| 网格 | CPU | 内存 RSS | 线程 | 终端输出 | 帧率 | 每帧 avatar/blit |
|---|---|---|---|---|---|---|
| 64×64 | 116% | 1091 MB | 81 | 90 KB/s | 60.7 | 9.7 / 7.5 ms |
| **128×64（默认）** | **149%** | **1108 MB** | 81 | **173 KB/s** | 60.5 | 8.3 / 5.2 ms |
| 128×96 | 186% | 1100 MB | 81 | 258 KB/s | 59.6 | 8.4 / 5.3 ms |

结论：

- **128×96 相对默认 128×64**：CPU +25%，终端带宽 +50%（格子数 8192→12288 正好 +50%），内存和帧率基本不变
- **内存 ~1.1 GB 与分辨率无关** —— 主要是 5 个 libmpv 实例、55 张贴图、模型数据（23 MB）和依赖库；81 个线程里大部分是 mpv 的
- 分辨率翻倍几乎不花 GPU，主要花在 CPU 侧的软件光栅化（见下）

### ⚠️ 注意：上面那张表是在「没有 GPU 的环境」里测的

开发环境的 `/dev/dri` 是空的，Mesa 于是退到 **llvmpipe（CPU 软件渲染）**，线程列表里能看到 `llvmpipe-0…15`：

```
线程占用: colonq 69% / llvmpipe-0 17% / llvmpipe-1 15%
```

也就是说**那 148% 的 CPU 大部分是软件光栅化**，不代表真实占用。在有 GPU 的桌面上会低一个数量级。

#### 双显卡笔记本实测（RTX 4060 Max-Q + Radeon 780M，niri / Wayland）

| 组合 | 实际 GL 渲染器 |
|---|---|
| 默认 | `AMD Radeon 780M (radeonsi, phoenix, ACO)` —— **核显** |
| `__NV_PRIME_RENDER_OFFLOAD=1 __GLX_VENDOR_LIBRARY_NAME=nvidia` | `NVIDIA GeForce RTX 4060 Laptop GPU` —— 独显 |
| `LIBGL_ALWAYS_SOFTWARE=0` / `MESA_LOADER_DRIVER_OVERRIDE=radeonsi` / `SDL_VIDEODRIVER=x11` | 都是核显（无差别） |

这个项目的绘制量很小（128×64 = 8192 像素），**核显完全够用**，而且更省电、更凉、能让独显保持休眠。只有在系统内存吃紧（想让贴图进 8GB 显存而不是内存）或者想上很高分辨率时才值得切独显：

```bash
# 用独显跑（贴图进显存，系统内存占用会降）
__NV_PRIME_RENDER_OFFLOAD=1 __GLX_VENDOR_LIBRARY_NAME=nvidia ./run-local.sh
```

> ⚠️ **NVIDIA 上的已知问题（已修）**：上游把骨骼矩阵写成 `uniform mat4 joint_matrices[256]`，256 个 mat4 = **1024 个 vec4 常量寄存器**，正好吃满 NVIDIA 顶点着色器的常量寄存器配额，链接时报：
>
> ```
> error C6020: Constant register limit exceeded at expression_angry_weight
> ```
>
> AMD/Mesa 的上限高得多，所以核显上不报错。现在改成**按模型实际的骨骼数**决定数组大小（启动日志会打印 `joint_matrices 数组大小: N（M 个常量寄存器 / NVIDIA 上限 1024）`）。也可以用 `COLONQ_MAX_JOINTS` 手动指定。
>
> 另外顺手修了一个潜藏 bug：`glShaderSource` 传入的长度是 `len - 1`，会砍掉源码最后一个字节（原来靠文件末尾的换行侥幸没出事）。
>
> ⚠️ 独显下还可能会挂：libmpv 的 GL 渲染上下文在 NVIDIA 驱动上有几率创建失败（`libmpv-rs` 是 2023 年的老绑定）。已改成**失败不致命**——创建不出来就只打个 warn、视频填充不可用，角色渲染照跑。如果仍然起不来，用 `COLONQ_NO_MPV=1` 彻底跳过 libmpv：
>
> ```bash
> __NV_PRIME_RENDER_OFFLOAD=1 __GLX_VENDOR_LIBRARY_NAME=nvidia COLONQ_NO_MPV=1 ./run-local.sh
> ```
>
> 现在 `run-local.sh` 遇到非 0 退出会把 `colonq.log` 里的错误行直接打到终端，不用自己翻日志。

启动时日志里会直接打印 GL 后端，这是最可靠的判断依据：

```bash
grep "GL 后端" colonq.log
# vendor=Mesa | renderer=llvmpipe (LLVM …)        → 软件渲染，没走显卡
# vendor=Mesa | renderer=AMD Radeon 780M (radeonsi…) → 核显
# vendor=NVIDIA Corporation | renderer=NVIDIA GeForce RTX 4060 … → 独显
```

一键对比几种强制走独显的组合：

```bash
./tools/check-gpu.sh
```

**自己复测时看 `后端` 这一栏**：

```bash
./tools/measure.sh 128x96 6
# 期望输出: 后端 NVIDIA 硬件  CPU 20%~40%
# 如果显示 "llvmpipe 软件渲染" 说明 GL 没走显卡，需要排查（SDL_VIDEODRIVER / 驱动 / 容器设备透传）
```

## 对比度（让人物更跳）

用三个旋钮叠加：**对比度曲线**（`COLONQ_CONTRAST`）、**暗部压深**（`COLONQ_SHADE_TINT`）、**明暗分界硬度**（`COLONQ_TOONY`）。

实测三档（同一帧、纯黑背景）：

| 强度 | 参数 | 观感 |
|---|---|---|
| **柔（当前默认）** | `contrast=1.0`，`shade_tint=0.82,0.80,0.88` | 层次最全，但和纯黑背景的对比偏弱 |
| 中 | `contrast=1.35`，`shade_tint=0.62,0.60,0.72` | 人物清晰跳出来，暗部仍有层次 |
| 强 | `contrast=1.65`，`shade_tint=0.45,0.43,0.58` | 对比更狠，但袖子会糊成一块黑、白裙高光有溢出 |

```bash
COLONQ_CONTRAST=1.65 COLONQ_SHADE_TINT="0.45,0.43,0.58" ./run-local.sh   # 更狠
COLONQ_CONTRAST=1.0 ./run-local.sh                                       # 回到柔和
COLONQ_TOONY=0.98 ./run-local.sh                                        # 明暗更像二分色块
```

> 对比度是在**线性空间**围绕 0.18（线性中灰）拉伸的，比直接乘 RGB 更可控，不会把暗部整体推黑。

## 皮肤颜色

肤色可以**单独**调，不影响头发和衣服 —— 靠的是材质分区（部位编号 1 = `FACE` / `SKIN` 材质），在片元着色器里只对这部分做染色和提饱和：

```glsl
if (pid == 1.0) {           // 1 = 脸 / 皮肤
    base *= skin_tint;      // 先调色温
    base = mix(vec3(lum), base, skin_sat);   // 再提饱和度
}
```

实测三档（面部特写，同机位）：

| 档 | 参数 | 观感 |
|---|---|---|
| 原样 | `1,1,1` / `1.0` | 偏灰、有点蜡 |
| 暖一点 | `1.10,0.97,0.95` / `1.15` | 自然带血色 |
| **白一点（当前默认）** | `1.22,1.18,1.20` / `1.00` | **稍微发白、干净，仍保留腮红** |
| 更白 | `1.32,1.28,1.30` / `0.95` | 接近瓷白，血色变淡 |
| 白偏冷 | `1.16,1.14,1.24` / `1.05` | 冷白（略偏蓝） |

```bash
COLONQ_SKIN_TINT="1.32,1.28,1.30" COLONQ_SKIN_SAT=0.95 ./run-local.sh   # 更白（瓷白）
COLONQ_SKIN_TINT="1.10,0.97,0.95" COLONQ_SKIN_SAT=1.15 ./run-local.sh   # 回到偏暖肤色
COLONQ_SKIN_TINT="1,1,1" COLONQ_SKIN_SAT=1.0 ./run-local.sh             # 关掉染色
COLONQ_SKIN_TINT="1.05,0.98,1.02" ./run-local.sh                        # 偏冷白（苍白系）
```

> 只对材质名里带 `SKIN` / `FACE` 的部分生效，头发（`HAIR`）、衣服（`CLOTH`）、眼睛（`EYE`）都不受影响。

## 背景色

默认是 **纯黑 `000000`**。上游的默认值其实是 `0x15050f`（一个很深的品牌紫），这就是之前背景发紫的原因。

```bash
COLONQ_BGCOLOR=000000 ./run-local.sh     # 纯黑（当前默认）
COLONQ_BGCOLOR=15050f ./run-local.sh     # 原作者的深紫
COLONQ_BGCOLOR=1a1b26 ./run-local.sh     # 东京夜配色那种深蓝灰
COLONQ_BGCOLOR=ffffff ./run-local.sh     # 纯白（配深色描边会更清楚）
```

颜色写 `RRGGBB` 或 `#RRGGBB` 都行；留空或写错会退回默认并打一条 warn，不会崩。

> 注意：终端里这块背景是程序用 ANSI 真彩色刷上去的，所以会盖住你终端主题的背景色。如果想让终端主题透出来，把背景色设成和终端一致即可。

## 手臂姿势

模型导出的绑定姿势是 T-pose（双臂平举）。渲染时对上臂骨骼绕 Z 轴旋转，把手臂放下来：

| `COLONQ_ARM_POSE` | 效果 |
|---|---|
| `0` | T-pose（原始绑定姿势） |
| **`-0.80`（当前默认）** | **约 46°，手臂下垂并向外张开（A-pose）** |
| `-1.20` | 约 69°，更贴身 |
| `-1.40` | 约 80°，几乎贴住身体，手在胯边 |
| `-1.60` | 贴太紧，会稍微陷进裙子 |
| `+0.8` | 手抬起来（符号相反） |

实测扫描过 `-0.9 / -1.2 / -1.3 / -1.4 / -1.5 / -1.6`：数值越负手臂越贴身体。

```bash
COLONQ_ARM_POSE=0 ./run-local.sh        # 回到 T-pose
COLONQ_ARM_POSE=-1.2 ./run-local.sh     # 稍微张开一点
```

> 这个角度是按 VRM 标准 humanoid 骨骼约定调的，换模型基本通用；如果某个模型的手臂往奇怪方向甩，微调这个值即可（零点几个弧度就有明显变化）。

## 只渲染上半身（裁剪模型）

按世界 Y 的水平面裁剪：低于裁剪面的片元直接 `discard`，同时加载时把**整体在裁剪面下方的图元**跳过（连顶点都不提交）。

```bash
COLONQ_VIEW=bust ./run-local.sh              # 半身取景 + 自动开启胸部裁剪
COLONQ_CLIP=chest ./run-local.sh             # 保持当前取景，只把胸以下裁掉
COLONQ_CLIP=1.15 ./run-local.sh              # 手动指定裁剪高度（世界 Y）
COLONQ_CLIP=none ./run-local.sh              # 关掉
```

裁剪高度 `chest` 是自动算的：先按骨骼名找 `upperChest` / `chest` / `spine` 的世界 Y（这台模型是 **1.012**），找不到就退化成身高 × 0.65。启动日志会打印实际值。

**效果**：`COLONQ_VIEW=full COLONQ_CLIP=chest` 能从全身图变成"裁到胸口"，裙摆和腿消失；`bust` 是配套的半身取景。

**代价与收益**（实测）：

| 配置 | avatar 阶段 |
|---|---|
| full，不裁剪 | 9.35 ms |
| full + 裁剪 | 9.50 ms |
| bust + 裁剪 | 8.82 ms |

也就是说**省不到什么** —— 因为这条连衣裙是一个跨裁剪面的图元，图元级剔除没法跳过它；真正省下的只是被裁剪区域的光栅化。在 GPU 上本来也无所谓。

> 注意：水平面是"硬切"，T-pose 的手臂会被切出平口。想让切口更自然，把 `COLONQ_CLIP` 调到肩膀以下（比如 `1.15`）。

## 降低占用

实测（128×64，llvmpipe 软件渲染下，看**相对**变化即可）：

| 措施 | CPU | 内存 | 线程 | 终端带宽 | 视觉 |
|---|---|---|---|---|---|
| 什么都不做 | 148% | 1160 MB | 81 | 175 KB/s | — |
| `COLONQ_NO_MPV=1` | 145% | ~1176 MB | **35** | 171 KB/s | 不变（视频填充不可用） |
| + `COLONQ_TEX_MAX=1024` | 146% | **873 MB** | 35 | 170 KB/s | 几乎无差别 |
| + `COLONQ_TEX_MAX=512` | 145% | **616 MB** | 35 | 170 KB/s | 几乎无差别 |
| + `COLONQ_TEX_MAX=256` | 144% | **523 MB** | 35 | 170 KB/s | 眼睛高光略软 |
| `COLONQ_GRID=96x48` | 128% | 621 MB | 35 | 98 KB/s | 清晰度下降 |
| `COLONQ_TERM_EVERY=6` | 143% | — | — | **87 KB/s** | 刷新 10 次/秒 |

结论：

1. **内存的大头是贴图**。这模型 55 张 2048×2048，原始 RGBA 就约 880 MB。`COLONQ_TEX_MAX=512` 直接把 1160 → 616 MB，而渲染分辨率只有 128×64，肉眼看不出差别
2. **`COLONQ_NO_MPV=1` 省的是线程**（81 → 35，5 个 libmpv 实例每个带若干线程），内存变化不明显
3. **`COLONQ_TERM_EVERY` 只影响终端带宽**（175 → 87 → 46 KB/s），对 CPU 几乎无影响
4. **网格越小 CPU 越低**（96×48 比 128×64 少 12%），但清晰度也降
5. CPU 在这里几乎全是 **llvmpipe 软件光栅化**的开销；真 GPU 上会低一个数量级，先确认 `./tools/measure.sh` 里「后端」不是 llvmpipe

推荐组合（视觉几乎无损）：

```bash
COLONQ_NO_MPV=1 COLONQ_TEX_MAX=512 ./run-local.sh     # 1160 → 616 MB，线程 81 → 35
```

还想更省：

```bash
COLONQ_NO_MPV=1 COLONQ_TEX_MAX=256 COLONQ_GRID=96x48 COLONQ_TERM_EVERY=6 ./run-local.sh
```

顺带内置的两个优化（无需开关）：

- `render_term` 里的 emoji 查表改成按字符缓存，去掉了每格一次的 `String` 分配
- 描边/部位这两遍额外绘制只在开启时才跑

## 描边（让角色从背景里跳出来）

用的是 MToon 的「反向壳 / inverted hull」做法：

1. 再把模型画一遍，但每个顶点沿法线外扩 `COLONQ_OUTLINE`（默认 0.004 世界单位）；
2. 这一遍只画**背面**（`glCullFace(GL_FRONT)`），于是只有轮廓外侧露出来的部分可见；
3. 这一遍**关掉深度写入**（`glDepthMask(false)`），否则壳会把本体整个挡住；
4. 整场景**先画完所有壳，再画所有本体** —— 逐节点交错画的话，后画的节点（头发）的壳会盖住先画好的本体（脸）。

前三步踩过坑：一开始忘了第 3 步，整个头变成一块纯色；第 4 步之前，头发壳会把脸涂掉。

注意本项目全局关着 `CULL_FACE`（`context.rs` 注释写着开了会破坏 libmpv），所以描边只在那一遍临时打开、画完立刻关。

```bash
COLONQ_OUTLINE=0 ./run-local.sh                        # 关掉描边
COLONQ_OUTLINE=0.014 ./run-local.sh                    # 更粗的轮廓
COLONQ_OUTLINE_COLOR="0.06,0.05,0.09" ./run-local.sh   # 深色描边（适合浅色背景）
COLONQ_OUTLINE_COLOR="0.55,0.85,1.0" COLONQ_OUTLINE_GLOW=1.2 ./run-obs.sh   # 冰蓝强发光
COLONQ_OUTLINE_COLOR="1.0,0.4,0.9" COLONQ_OUTLINE_GLOW=1.0 ./run-obs.sh     # 霓虹粉发光
COLONQ_OUTLINE=0.008 COLONQ_OUTLINE_COLOR="1,0,0" ./run-local.sh   # 调试用红边
```

> 64×64 下描边就是 1 个像素（终端 1 个字符格），所以宽度超过 0.014 会糊成一片光晕。

## 可选：按部位分字符表（作者的招牌效果）

作者那张直播画面里，**头发是密点、身体是文字、裙摆是方块** —— 这是调色板按部位分表的结果。他的 `PaletteType::from_color` 是按**他自己模型的精确 RGB** 分类的（头发必须正好 186/176/189），换个模型就全落到默认词。

本版本做了个通用替代：**按材质名关键字分区**，编号写进 alpha 通道，终端渲染时按编号选字符表。

```bash
COLONQ_PARTS=1 ./run-local.sh        # 启用（默认关闭，不影响现有观感）
```

部位编号（多数 VRoid 模型都带 FACE / EYE / SKIN / HAIR / CLOTH 关键字）：

| 编号 | 部位 | 判定关键字 |
|---|---|---|
| 1 | 脸 / 皮肤 | `SKIN`、`FACE` |
| 2 | 头发 | `HAIR` |
| 3 | 眼睛 | `EYE` |
| 4 | 衣服 | `CLOTH`、`OUTERWEAR` |
| 9 | 其他 | 剩下的 |

默认就是 **头发 `.` / 衣服 `#`，脸和眼睛保持默认词**（最接近作者那个观感）：

```bash
COLONQ_PARTS=1 ./run-local.sh
# 等价于 COLONQ_PART_WORDS="2=.,4=#"
```

想换别的组合：

```bash
COLONQ_PARTS=1 COLONQ_PART_WORDS="2=~,4=#" ./run-local.sh    # 头发用波浪线，更密
COLONQ_PARTS=1 COLONQ_PART_WORDS="2=.,4=:" ./run-local.sh    # 衣服用冒号，更轻
COLONQ_PARTS=1 COLONQ_PART_WORDS="1=.,2=~,3=#,4=lcolonq" ./run-local.sh  # 全都分开（脸也会变点）
```

字符越多视觉越"实"，越少越"空"。**部位没写进 `COLONQ_PART_WORDS` 的，就回落到 `COLONQ_WORD`（默认 `lcolonq`）** —— 所以只写 `2=.,4=#` 时脸和眼睛仍是文字。

```bash
COLONQ_PART_WORDS="1= ,2=.,3=#,4=:"     # 脸留空、头发点、眼睛方块
COLONQ_PART_WORDS="2=abcdefgh,3=@"      # 头发随机英文、眼睛 @
```

### 实现方式（为什么用 alpha 通道）

渲染时**再画一遍所有网格**，但这一遍：

- `glColorMask(0,0,0,1)` —— 只允许写 alpha 通道，RGB 不动
- `glDisable(GL_BLEND)` —— 混合会污染编号
- 片元着色器直接输出 `alpha = 部位编号 / 255`

于是最终帧里 **RGB 是画面、alpha 是部位编号**，CPU 侧 `get_pixel_rgba` 一次读回两样东西，终端渲染按 alpha 选字符表。代价是多一遍绘制（12K 像素，实测无感）。

## 原作者那些"特殊效果"是怎么做的

**不是蒙版**，是一套「调色板替换」系统（代码在 `framebuffer.rs:render_term` + `avatar.rs:Palette`）：

1. 先把角色渲染成 64×64 的普通图像；
2. 逐像素查调色板 `palette.lookup(color)`，按**颜色分区**（`PaletteType`：头发 / 皮肤 / 眼睛 / 高光 / 眼袋 / 帽子）决定这一格怎么画；
3. 每个分区可以映射成四种东西：
   - `PaletteEntry::Color` → 换成另一种颜色
   - `PaletteEntry::Pattern` → 换成一张图案的对应像素
   - `PaletteEntry::Animation` → 换成多帧动画（按帧号循环）
   - `PaletteEntry::Video` → 换成 libmpv 播放的视频帧
4. 字形层面还有一层：默认用 `Palette::default_word`（现在是 `"lcolonq"`）填满角色内部，边缘再由 `get_surrounding()` 按邻域自动换成 `| / \ - _` 勾轮廓。

所以他能做到：角色用任意文字填充、某个部位换成图案、整块变视频、按部位单独换色 —— 都是"逐像素查表替换"，不是蒙版。

**实时切换**需要跑控制通道：`lcolonq/fig`（Haskell，监听 `localhost:32050`，s-expression 协议），`fig.rs` 里已经写好了订阅/解析。不跑它也能改，但要改代码里的默认值。

## 相对上游做的改动

| 文件 | 改动 |
|---|---|
| `Cargo.toml` | 去掉 `sdl2` 的 `image` feature（不再需要 SDL2_image）；删掉 `tungstenite` 依赖（连带摆脱 2023 年锁定的 openssl-sys，它就是最初编不过的原因） |
| `src/texture.rs` | 改用 `image` crate 解码贴图，替代 `sdl2::image::LoadSurface` |
| `src/context.rs` | 移除 `Sdl2ImageContext` |
| `src/main.rs` | ① 资源路径改为 crate 内 `assets/`，可用 `COLONQ_ASSETS` 覆盖（上游是 `../assets`，相对仓库父目录）② 删除 rebroadcast（TCP 31340）与 websocket 推流（`wss://colonq.computer/...`，连不上会 panic）③ 相机/模型变换/分辨率/手臂姿态改为环境变量 ④ 表情权重上传容错 ⑤ 加 PNG 调试出口 |
| `src/fig.rs` | 控制通道 `localhost:32050` 连不上时安静跳过，不再 panic |
| `src/tracking.rs` | UDP 11573 绑定/收包失败时跳过，不再 panic |
| `src/vrm.rs` | 缺表情/缺 morph target 时跳过而不是 panic；非 VRM 1.0 给出明确报错 |
| `src/avatar.rs` | 缺骨骼时跳过 `transform_bone` |
| `deps/libmpv-rs/libmpv-sys` | 用仓库自带的 `old_pregenerated_bindings.rs` 复制成 `pregenerated_bindings.rs`，关闭 `use-bindgen`（旧 bindgen 处理不了 mpv 0.41 头文件里的匿名 union） |
| `assets/shader.*` | 新写（上游未公开）。注意蒙皮矩阵里已含模型变换，顶点**只能** `skin * p`，再乘一次 `position` 会让旋转/位移平方 |

## 已知限制

1. **画面是 T-pose 静态的**：没有面部追踪，也没有动画。要动起来看下面"下一步"。
2. **shader 是自写的**，只有"二值化明暗 + shade 色 + 边缘光"，不是作者的原始效果。
3. **没接 MToon 参数**：`deps/gltf-vrm` 不解析 `VRMC_materials_mtoon`，所以 shade 色等参数目前是 `src/../assets/shader.frag` 里的常量。
4. 终端字符效果用的是默认 palette（用 "colonq" 字样填充），换色/换字/视频贴图那套要在 `framebuffer.rs` 里配。

## 下一步

- **动起来**：`cd deps/OpenSeeFace && poetry install && poetry run python facetracker.py -c 0`（默认就发 UDP 到 127.0.0.1:11573，和 `tracking.rs` 对得上），启动顺序随意。
- **接 MToon 参数**：从 VRM 的 JSON chunk 读 `VRMC_materials_mtoon` 的 `shadeColorFactor` / `shadingShiftFactor` / `shadingToonyFactor`，做成 uniform 传给 shader。参考实现见 `../参考-shader-three-vrm/`。
- **换自己的模型**：VRoid Studio 导出的 VRM 1.0 放进 `assets/`，用 `COLONQ_MODEL=你的模型.vrm` 指定，再按需调 `COLONQ_VIEW/CAM/YAW`。
