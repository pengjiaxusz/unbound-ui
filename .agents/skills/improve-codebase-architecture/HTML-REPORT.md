# HTML 报告格式规范

架构评审报告渲染为操作系统临时目录下的单个自包含 HTML 文件。Tailwind 与 Mermaid 均通过 CDN 引入。Mermaid 擅长可靠绘制图结构（流程图/依赖图/时序图）；手写 `<div>` 与内联 SVG 则适合更有编排感的可视化（体量对比图、横截面图）。两者混用：不要所有图都指望 Mermaid，否则整份报告会显得千篇一律。

## 基础骨架

```html
<!doctype html>
<html lang="zh-CN">
  <head>
    <meta charset="utf-8" />
    <title>架构评审：{{repo name}}</title>
    <script src="https://cdn.tailwindcss.com"></script>
    <script type="module">
      import mermaid from "https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.esm.min.mjs";
      mermaid.initialize({ startOnLoad: true, theme: "neutral", securityLevel: "loose" });
    </script>
    <style>
      /* 补充 Tailwind 不便覆盖的小样式：虚线接缝、手绘感箭头等 */
      .seam { stroke-dasharray: 4 4; }
      .leak { stroke: #dc2626; }
      .deep { background: linear-gradient(135deg, #0f172a, #1e293b); }
    </style>
  </head>
  <body class="bg-stone-50 text-slate-900 font-sans">
    <main class="max-w-5xl mx-auto px-6 py-12 space-y-12">
      <header>...</header>
      <section id="candidates" class="space-y-10">...</section>
      <section id="top-recommendation">...</section>
    </main>
  </body>
</html>
```

## 页眉（Header）

仓库名称、日期，以及一个紧凑图例：实线框 = 模块（module），虚线 = 接缝（seam），红色箭头 = 泄漏（leakage），深色粗框 = 深度模块（deep module）。不要写引言段落，直接进入候选卡片。

## 候选卡片

图表承担主要信息量。文字要精简、平实，不加客套地直接采用 `/codebase-design` 技能中的术语。

每个候选对象对应一个 `<article>`：

- **标题**：简短，点明深化动作（例如“收拢 Order 接入流水线”）。
- **徽章行**：推荐强度（`Strong` = 翡翠绿，`Worth exploring` = 琥珀黄，`Speculative` = 石板灰），外加依赖类别标签（`in-process`、`local-substitutable`、`ports & adapters`、`mock`）。
- **涉及文件**：等宽字体列表，`font-mono text-sm`。
- **改造前 / 改造后对比图**：视觉核心。两栏并排，模式见下文。
- **问题**：一句话，点出痛点在哪。
- **方案**：一句话，点出要改什么。
- **收益（Wins）**：精简要点，每条不超过十余字（如“测试统一走单一接口”、“定价逻辑不再跨接缝泄漏”、“删掉 4 个浅包装层”）。
- **ADR 提示**（如适用）：放在琥珀色提示框中的一行说明。

不要写整段解释。如果一张图需要一整段文字才能看懂，重画那张图。

## 图表模式

为每个候选挑选最契合的模式并混搭使用，不要每张图长一个样——多样性本身就是可读性的一部分。

### Mermaid 图（依赖 / 调用流的主力）

当重点在于“X 调 Y 调 Z，一团乱麻”时，使用 Mermaid `flowchart` 或 `graph`。包在 Tailwind 卡片里以免显得突兀，并用 `classDef` 把泄漏边标红、深度模块标深色。时序图很适合表现“改造前 6 次往返；改造后 1 次”。

```html
<div class="rounded-lg border border-slate-200 bg-white p-4">
  <pre class="mermaid">
    flowchart LR
      A[OrderHandler] --> B[OrderValidator]
      B --> C[OrderRepo]
      C -.leak.-> D[PricingClient]
      classDef leak stroke:#dc2626,stroke-width:2px;
      class C,D leak
  </pre>
</div>
```

### 手写方框与箭头（当 Mermaid 排版跟你作对时）

用带边框和标签的 `<div>` 表示模块，用绝对定位在相对容器上的内联 SVG `<line>` 或 `<path>` 画箭头。当你想让“改造后”的图呈现出“一个粗边框深度模块里包着淡化内部细节”的视觉份量时，选这种方式——Mermaid 画不出这种份量感。

### 横截面图（适合层层浅薄透传）

堆叠横向条带（`h-12 border-l-4`）展示一次调用穿过多少层。改造前：6 个什么都不干的薄层；改造后：1 个标明整合职责的厚层。

### 体量对比图（适合“接口几乎和实现一样宽”）

每个模块画两个矩形：一个代表接口表面积，一个代表实现体量。改造前：接口矩形几乎和实现矩形一样高（浅）；改造后：接口矩形矮小，实现矩形高大（深）。

### 调用树收拢图

改造前：嵌套方框呈现的函数调用树；改造后：同一棵树收拢进一个方框，原本的调用退化为框内淡显的内部细节。

## 视觉风格指南

- 走克制的编辑部排版风格，而不是企业仪表盘。留白要足，标题可选衬线体（`font-serif` 与 stone/slate 灰调很搭）。
- 克制用色：一种主强调色（翡翠绿或靛蓝），外加红色表示泄漏、琥珀色表示警告。
- 图表高度控制在 ~320px 左右，使改造前/后能舒适地并排同屏对比、无需滚动。
- 图表内模块标签使用 `text-xs uppercase tracking-wider`，让它看起来像架构示意图而不是 UI 组件。
- 页面仅引入 Tailwind CDN 与 Mermaid ESM；其余完全静态，无应用逻辑代码，除了 Mermaid 自身渲染外无交互。

## 首推建议区（Top recommendation）

一张较大的卡片：候选名称、一句话理由、跳往对应卡片的锚点链接。仅此而已。

## 语气与用词

平实、精炼，但架构名词和动词必须直接取自 `/codebase-design` 技能。精炼不是用词漂移的借口。

**严格使用：** module（模块）、interface（接口）、implementation（实现）、depth（深度）、deep（深）、shallow（浅）、seam（接缝）、adapter（适配器）、leverage（杠杆）、locality（局部性）。

**绝不替换：** 不要用 component、service、unit 代替 module；不要用 API、signature 代替 interface；不要用 boundary 代替 seam；指代模块时不要用 layer、wrapper。

**符合风格的表达示例：**

- “Order 接入模块过浅：接口几乎与实现等宽。”
- “定价逻辑跨接缝泄漏。”
- “深化：单一接口，单一测试面。”
- “两个适配器证明接缝真实存在：生产用 HTTP，测试用内存。”

**收益要点**用词汇表术语点出所得：*“局部性：bug 集中于单一模块”*、*“杠杆：一个接口覆盖 N 处调用”*、*“接口收窄，实现吸收薄包装层”*。不要写“更易维护”或“代码更干净”，因为这些词不在词汇表里。

不要含糊铺垫、不要客套开场、不要写“值得注意的是……”。能写成要点的句子就写成要点，能砍掉的要点就砍掉。如果某个词不在 `/codebase-design` 词汇表里，先去词汇表找现成词，不要自己另造。