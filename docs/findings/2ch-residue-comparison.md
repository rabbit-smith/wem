# 2ch/48 kHz residue 位流对比

日期：2026-09-20

> 状态：这是修复 aoTuV 联合量化之前的闭合基线。最终实现已从
> 38,648 B 收敛到 37,658 B，并与 Wwise 完整逐字节相同；最终结论见
> [`2ch-active-findings.md`](2ch-active-findings.md)。本页的 +990 B 分解用于证明旧实现的
> 缺口，不描述当前剩余差异。

## 输入与方法

本报告比较同一份 2ch/48 kHz PCM 的两份输出：

- Wwise 2013.2 构建：37,658 B，SHA-256
  `41fe43e2b99077e9ae6f57a00443d4a8dfc60513b8c5c46311b9be97e83ef629`
- 当时实现：38,648 B，SHA-256
  `3a31227bd4ea175c21f7cdc6218daa083d1aa1a182b54b7834e0bdc334225fa9`

系数、classword 和逐 stage VQ entry 均由仓库的
[`scripts/decode_wem.py`](../../scripts/decode_wem.py) type-2 decoder 解出；一次性汇总脚本
未作为正式工具保留。

两边都严格解出 142 个音频包，其中 55 个 short、87 个 long；完整 mode 序列相同，
所有 residue 都满足 `status in {complete, empty}` 且包尾剩余不足 8 bit。逐包统计覆盖全部包，
没有按“能否配对”筛选样本。

## 位数闭合

| bins | Wwise VQ bit | 当时实现 VQ bit | 当时实现 − Wwise |
|---|---:|---:|---:|
| 0–64 | 69,584 | 66,357 | **−3,227** |
| 64–128 | 31,005 | 34,658 | **+3,653** |
| 128–256 | 44,620 | 47,463 | **+2,843** |
| 256–512 | 82,147 | 83,483 | **+1,336** |
| 512–1024 | 35,292 | 38,268 | **+2,976** |
| **合计** | **262,648** | **270,229** | **+7,581** |

classword 位数是 14,770 对 15,201，当时实现再多 431 bit。VQ 与 classword 合计多
8,012 bit；实际音频包载荷多 7,920 bit，也就是完整的 **990 B**，其余 −92 bit 来自
floor、header 和逐包 padding。这个分解闭合了总账。

## 类别与重建系数

| bins | class 相同率 | 系数相同率 | Wwise 非零 | 当时实现非零 |
|---|---:|---:|---:|---:|
| 0–64 | 39.3% | 61.9% | 12,378 | 11,603 |
| 64–128 | 35.7% | 83.1% | 6,286 | 6,058 |
| 128–256 | 60.9% | 79.1% | 10,693 | 9,138 |
| 256–512 | 66.2% | 73.2% | 19,066 | 19,405 |
| 512–1024 | 58.0% | 93.6% | 7,411 | 9,689 |

64–256 的当时输出并不是因为“非零系数远多于 Wwise”而变大；可靠 decoder 得到的非零数
反而更少。超支来自不同 class/stage/book 和不同 VQ entry 的码长组合。

一个有方向性的现象是，当时的 class 主要落在 1/3/5/7，而 Wwise 在相同 partition 中大量使用
2/4/6/8。重建 residue 的 channel 1 幅度也系统性更低，例如 128–256 的平均绝对值是 Wwise
0.724、当前 0.561。结合 type-2 分类器把 channel 0 作为 magnitude peak、channel 1 作为
angle peak，这支持“分类前的整数 angle residue 不同”这一候选。它仍是推断；VQ 重建值不能
替代构建侧分类器实际读取的整数输入。

## 被撤回的旧结论

旧脚本把 classword group、stage 和 partition 的循环顺序写错，随后在失步位流上继续读 VQ。
因此以下数字全部作废：

- bins 64–128 非零数 38× 或 42×；
- bins 64–128 多 21,518 bit；
- bins 128–256 多 40,014 bit；
- 从配对子集推出的“0–64 当前 residue 偏大”。

对应的手写 reader 已删除；下列结果均来自仓库 decoder 的重新验证。

## 已完成的决定性观测

随后已经直接观测真实构建的整数 residue handoff。把同一次调用的 raw MDCT、整数 floor、
impulse peak 和 nonzero 输入交给公开 aoTuV beta6.03 算法后，217,088 个输出整数全部相同。
这项观测终结了“继续调整 class/VQ 打包”的路线，并促成 Python/Rust 两侧的联合量化移植。
完整证据和后续端到端闭合见
[`2ch-residue-xiph.md`](2ch-residue-xiph.md)。
