# 2ch/48 kHz 字节精确性结论

日期：2026-09-20

## 最终结果

固定配对输入是 96,000-frame、2ch/48 kHz PCM，输入 SHA-256 为
`90c3a5b4b2c5008badff42da7e9ecc46d0ec87c944f9aa32a1151903da2a1ff3`。

| 输出 | 字节 | SHA-256 |
|---|---:|---|
| Wwise 2013.2.10 build 4884 | 37,658 | `41fe43e2b99077e9ae6f57a00443d4a8dfc60513b8c5c46311b9be97e83ef629` |
| Rust 原生内核 | 37,658 | `41fe43e2b99077e9ae6f57a00443d4a8dfc60513b8c5c46311b9be97e83ef629` |
| Python oracle | 37,658 | `41fe43e2b99077e9ae6f57a00443d4a8dfc60513b8c5c46311b9be97e83ef629` |

三者完整 WEM 逐字节一致。142 个音频包全部精确，mode 序列为 55 short + 87 long。
`tests/integration/test_core_oracle_golden.py` 另有无资产的确定性 2ch 契约，固定 native、direct
core 与 oracle 的输出 SHA、包数和 short/long 计数。

另有六个 48,000-frame 的真实构建契约，覆盖静音、相反直流、低频音调、高频音调、孤立脉冲和
双声道独立噪声。对应 WAV 与 WEM 位于 `tests/data/2ch-reference/`；六个输入的完整 WEM 均与
Rust 输出逐字节一致，`tests/contract/test_2ch_reference_corpus.py` 会同时固定输入/输出 SHA、
包数和 short/long 计数。

## 根因

最终差异由六个相互独立的缺口叠加而成。

1. **输入边界**：真实构建先做状态化 DC 高通，再经过 signed-16 存储边界。公式为
   `float32((x - previous_x) + coefficient * previous_y)`，系数位型 `0x3f7f546d`；随后执行
   `round_ties_even(float32(y * 32767)) / 32768`。完整转换的 192,000 个输出样本已逐位核对。
2. **residue 联合量化与分类**：2ch type-2 路径必须使用 aoTuV beta6.03 的耦合量化、nonzero
   传播和 coupled-mid 分类表。真实 handoff 的 217,088 个量化整数以及 4,836 个分区类别均由
   仓库 decoder 闭合验证。
3. **psychoacoustic look**：运行时直接读取四个 look 的 `tonecurves` 指针图。两个 short look
   共享同一张 17×8×58 表，两个 long look 也共享同一张表，但 2ch/48 kHz 与原先沿用的
   6ch/44.1 kHz 表不同。短表原始字节 SHA-256 为
   `dab0ce2556b2dbacc8927579b03d033eb963ae7c68238c4b72f16ee91e8f2e96`，长表为
   `a4b2eb1e15485451897beac834e6fce858c6dac062537601dd1e4a24a033399f`。
4. **short look**：short remap 的峰值上限必须随瞬态选择 `short_look_0/1`；固定使用 look 0
   只会在强低频峰触发时出错。
5. **瞬态能量环**：15 槽能量状态中的 `last_energy` 是滑动累计值，`energy_sum` 是每轮回
   重建用的分段累计值；原实现把二者职责写反，导致下降沿提前一个 64-sample quantum，进而
   选择错误的 short profile。修正依据是已保存的检测器能量更新段逐指令布局，未按目标包调阈值。
6. **EOS 状态**：32-tap LPC 的训练长度是结束标记到来时仍在分析缓冲区中的 PCM 数量，并在
   `blocksizes[1]` 截断；它会随最后几个 block 的实际 hop 改变，不能固定为 2048。公开 Xiph
   的 `vorbis_analysis_wrote` 使用相同的动态长度，随后用 32-tap predictor 外推
   ([block.c](https://github.com/xiph/vorbis/blob/1b75110b5a2754ba1931d82dd83cb822b266a21d/lib/block.c#L468-L505)，
   [lpc.c](https://github.com/xiph/vorbis/blob/1b75110b5a2754ba1931d82dd83cb822b266a21d/lib/lpc.c#L53-L147))。

直接替换 2ch tone bank 后，音频包从 119/142 提升到 139/142；按 look 选择 peak cap 后第
135、138 帧精确；修正 EOS 训练窗后第 141 帧的 128 个预测样本逐位一致，最终达到 142/142。

## 可信边界

- profile 数值只来自运行时直接读取或公开参考实现，没有根据目标输出拟合。
- residue/classword 比较使用仓库 decoder；早期手写 reader 因循环失步产生的统计已删除或标为
  历史无效。
- 反汇编只用于定位运行时观测点；语义由观测输入输出、位流或公开源码再次确认。
- `fft`/tone seed 中仍可观察到不会改变 floor posts 或位流的 float32 末位差；字节正确性由完整
  WEM、逐包和两条实现路径共同约束。
- 独立真实构建证据覆盖一个 96,000-frame 配对输入及八个 48,000-frame 参考/压力输入。4,096、
  4,097、8,192-frame 等边界长度由 native/oracle 一致性和 fuzz 契约覆盖，但没有独立真实构建
  输出，因此不把有限语料的结论表述为对任意 PCM 的穷尽证明。

## 压力输入闭合

`tests/data/2ch-stress/` 的两个边界样本现已进入默认绿色契约：

- 左右声道每 2,048 samples 交替的方波突发：369 个音频包全部精确，完整 WEM 为 35,172 B；
  它覆盖多声道 OR、下降沿、连续 short profile 切换和长 short run。
- 最后 2,048 samples 突变的尾部信号：68 个音频包全部精确，完整 WEM 为 4,909 B；它覆盖
  动态 EOS LPC 训练长度和最后一帧。

`tests/contract/test_2ch_stress_corpus.py` 同时验证批处理内核、Python oracle 与不规则分块流式内核，
三条路径都与真实构建完整 WEM 逐字节一致。绿色与压力 WAV 分别由
`scripts/generate_2ch_reference_inputs.py` 和 `scripts/generate_2ch_stress_inputs.py` 确定性重建。

旧的 +990 B 位数闭合保留在
[`2ch-residue-comparison.md`](2ch-residue-comparison.md)，公开 residue 算法依据见
[`2ch-residue-xiph.md`](2ch-residue-xiph.md)。
