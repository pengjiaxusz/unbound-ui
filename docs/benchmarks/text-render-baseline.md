# 文本渲染与构建档位裁剪性能基线

> **基准版本**: Issue #54 Phase 2 结清归档  
> **归档日期**: 2026-11-20  
> **度量套件**: `crates/unbound-text/tests/text_render_baseline.rs`  
> **调度命令**: `just text-baseline`（基于 `bque bench` 独占 CPU 跑分）

---

## 1. 评测环境与硬件规格

| 维度 | 配置参数 |
| :--- | :--- |
| **操作系统** | Microsoft Windows 11 专业工作站版 (24H2, Build 10.0.26100) |
| **处理器 (CPU)** | Intel(R) Core(TM) i5-14600KF (14 核 20 线程, 基础 3.5 GHz / 睿频 5.3 GHz) |
| **内存 (RAM)** | 32 GB DDR5 高速内存 |
| **编译器工具链** | rustc 1.85.0 (2021 edition), profile: release (opt-level = 3) |
| **调度环境** | `bque bench`（独占排他锁、前台优先级提至 High、挂起后台所有编译与低优先级作业） |

---

## 2. 四大核心基准指标实测数据

### 2.1 产物体积与构建档位裁剪（Release Binary & Profile Footprint）

解耦 `unbound-text` 对 Workspace 级 21 项 Win32 特性的无条件传染，实现正交的 Cargo Feature 矩阵隔离：

| 构建档位 | Feature 配置 | 依赖树特征 | 产物增量与精简 |
| :--- | :--- | :--- | :--- |
| **Full 构建** | `backend-directwrite`, `backend-swash` | 包含 Windows DirectWrite API 与完整的 Swash、Zeno、Yazi 纯软依赖链 | 基准全量（全功能开发展台默认） |
| **纯 DirectWrite 档位** | `backend-directwrite` (无 default) | 最小 3 项 Win32 特性 (`Win32_Foundation`, `Win32_Graphics_DirectWrite`, `Win32_System_Com`)；**彻底剔除 `swash` 与 `zeno`** | **依赖项精简 15 项，静态代码减少 ~1.8 MiB**（指标要求 ≥ 1.5 MiB） |
| **纯 Swash 档位** | `backend-swash` (无 default) | 仅纯 Rust 依赖；**彻底剔除 `windows` 库与 COM 绑定** | 零 Windows API 传染，极速跨平台编译 |
| **零后端防呆** | 无任何 backend feature | 触发编译期断言 `compile_error!` | 编译在秒级拦截，杜绝运行时空光栅化 panic |

### 2.2 显存峰值与图集懒初始化（3500 常用汉字 × 相位阶梯）

在单页尺寸 2048 × 2048、边距 padding 2 纹素的标准配置下，3500 个常用汉字在不同亚像素相位档位与 Emoji 场景下的显存分配：

| 相位模式 | 汉字字形数 | 亚像素相位数 | 产生图集槽位数 | R8 图集占用页数 | 显存总占用 (MiB) | 相比预分配显存优化 |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: |
| **PhaseMode::One** | 3,500 | 1（整像素相位 0） | 3,500 | 1 页 | **4.0 MiB** (4,194,304 B) | 降低 80.0% |
| **PhaseMode::Four** | 3,500 | 4（¼ px 步长） | 14,000 | 2 页 | **8.0 MiB** (8,388,608 B) | 降低 60.0% |
| **PhaseMode::Eight** | 3,500 | 8（⅛ px 步长） | 28,000 | 3 页 | **12.0 MiB** (12,582,912 B) | 降低 40.0% |
| **RGBA8 懒分配** | 3,500 | 任意 | - | 0 页 (未开辟) | **0.0 MiB** | 纯文本零 RGBA 显存开销 |

> **显存阶梯验证**：实测 3500 个常用汉字在 8 / 4 / 1 相位下严格呈现 **12 MiB / 8 MiB / 4 MiB** 的等差阶梯关系。在未遭遇彩色 Emoji 或 SDF 槽位分配前，RGBA8 图集保持 `None`，纯普通文本显存严格锁定为 4 MiB。

### 2.3 首帧冷启动耗时（100 个典型 UI 字形解析、光栅化与上传）

对 100 个典型 UI 常用字形（高频按钮文案、中英文混排、数字与符号）进行全新冷启动渲染测试（包含排版塑形、光栅化与 GPU 图集更新生成）：

| 光栅化后端 | 首帧光栅化与上传实测 (ms) | 目标上限 (ms) | 达标判定 | 塑形耗时参考 (ms) |
| :--- | :---: | :---: | :---: | :---: |
| **DirectWrite** | **0.164 ms ~ 0.291 ms** | ≤ 3.0 ms | **PASS (达成率 > 10x)** | ~9.6 ms (首次字体系统解析) |
| **Swash (Cosmic)** | **0.724 ms ~ 0.752 ms** | ≤ 5.0 ms | **PASS (达成率 > 6x)** | ~9.5 ms (首次字体系统解析) |

### 2.4 运行期切换吞吐与显存泄漏回归（连续 100 次 `apply_settings` 后端切换）

连续执行 100 次 `apply_settings` 在 DirectWrite 与 Cosmic 之间交替切换，每次切换触发全图集清空并重新渲染：

| 度量项 | 实测结果 | 目标基线 | 判定与结论 |
| :--- | :---: | :---: | :---: |
| **100 次连续切换总耗时** | **8.406 ms ~ 8.603 ms** | < 50.0 ms | **PASS** |
| **单次后端热切换平均耗时** | **84.1 µs ~ 86.0 µs** | < 500.0 µs | **PASS（亚毫秒级极速切换）** |
| **显存泄漏与孤儿页检测** | 0 泄漏（始终 ≤ 4.0 MiB） | 0 泄漏 | **PASS（切换后图集干净重置）** |
| **RGBA8 图集泄漏检测** | 0 次意外触发开辟 | 保持未开辟 | **PASS（无状态污染）** |

---

## 3. 自动化防回归门禁机制

1. **日常集成测试**：`just test` 默认包含 `text_render_baseline.rs` 中的功能与显存阶梯断言；
2. **基准性能跑分**：开发者或 CI 可运行 `just text-baseline` 调起 `bque bench` 纯净跑分并输出最新微秒级数据；
3. **语言守卫**：全量代码注释与基准说明均通过 `just gate-check` 审计。
